//! IPC Socket Controller — Unix Domain Socket server & CLI client.
//!
//! Provides a Unix Domain Socket at `~/.config/swai/swai.sock` for terminal
//! commands (`swai start <id>`, `swai stop`, `swai status`, `swai list`) to
//! query and control SWAI programmatically.
//!
//! ## Protocol
//!
//! The server listens for JSON messages on the Unix socket. Each request is a
//! single-line JSON object with an `action` field:
//!
//! ```json
//! {"action": "status"}
//! {"action": "list"}
//! {"action": "start", "data": {"model_id": "llama-3"}}
//! {"action": "stop"}
//! {"action": "switch", "data": {"model_id": "llama-3"}}
//! ```
//!
//! Responses are JSON objects with `status`, `message`, and optional `data` fields:
//!
//! ```json
//! {"status": "ok", "message": "...", "data": {...}}
//! {"status": "error", "message": "..." }
//! ```
//!
//! ## Architecture
//!
//! The IPC server runs as a background tokio task inside the main app process.
//! It owns the `ProcessManager` and `ProxyState`, so it can start/stop/switch
//! models in response to CLI commands. The GTK app reads config separately and
//! shares the proxy state via `Arc<Mutex<>>`.

use serde::{Deserialize, Serialize};
use std::io;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc;
use tracing::{debug, error, info, warn};

use crate::config::Config;
use crate::process_manager::ProcessManager;
use crate::proxy::ProxyState;

// ---------------------------------------------------------------------------
// Message types
// ---------------------------------------------------------------------------

/// A request from a CLI client to the IPC server.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActionRequest {
    /// The action to perform: `start`, `stop`, `switch`, `status`, or `list`.
    pub action: String,
    /// Optional payload (used by `start` and `switch`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Value>,
}

/// A structured response from the IPC server.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActionResponse {
    /// `"ok"` or `"error"`.
    pub status: String,
    /// Human-readable message describing the result.
    pub message: String,
    /// Optional payload (status output, model list, etc.).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Value>,
}

impl ActionResponse {
    /// Create a success response with optional data.
    pub fn ok(message: impl Into<String>, data: Option<serde_json::Value>) -> Self {
        Self {
            status: "ok".to_string(),
            message: message.into(),
            data,
        }
    }

    /// Create an error response.
    pub fn error(message: impl Into<String>) -> Self {
        Self {
            status: "error".to_string(),
            message: message.into(),
            data: None,
        }
    }
}

// ---------------------------------------------------------------------------
// Shared state
// ---------------------------------------------------------------------------

/// Shared state owned by the IPC server.
///
/// The IPC server holds the `ProcessManager` (for starting/stopping models) and
/// a shared `ProxyState` (so the GTK app can read current proxy target).
pub struct IpcState {
    /// The process manager — owns running model processes.
    pub process_manager: std::sync::Mutex<ProcessManager>,
    /// Shared proxy state — updated by IPC server on start/stop/switch.
    pub proxy_state: Arc<Mutex<ProxyState>>,
    /// The loaded configuration.
    pub config: Config,
}

impl IpcState {
    /// Create new IPC state from a loaded config.
    pub fn new(config: Config) -> Self {
        Self {
            process_manager: std::sync::Mutex::new(ProcessManager::new(config.clone())),
            proxy_state: Arc::new(Mutex::new(ProxyState::new())),
            config,
        }
    }
}

// ---------------------------------------------------------------------------
// Socket path helpers
// ---------------------------------------------------------------------------

/// The directory where SWAI stores its runtime files.
pub fn config_dir() -> PathBuf {
    if let Ok(home) = std::env::var("HOME") {
        PathBuf::from(home).join(".config").join("swai")
    } else {
        PathBuf::from(".config/swai")
    }
}

/// The Unix socket path used by the IPC server.
pub fn socket_path() -> PathBuf {
    config_dir().join("swai.sock")
}

/// Remove a stale socket file if it exists (e.g. from a crashed previous run).
fn cleanup_stale_socket(path: &Path) {
    if path.exists() {
        match std::fs::remove_file(path) {
            Ok(()) => debug!("removed stale IPC socket at {:?}", path),
            Err(e) => warn!("failed to remove stale IPC socket {:?}: {}", path, e),
        }
    }
}

// ---------------------------------------------------------------------------
// IPC Server
// ---------------------------------------------------------------------------

/// Handle to the running IPC server.
///
/// Drop this handle to stop the background listener task and close the socket.
pub struct IpcServerHandle {
    /// Channel receiver for the background listener task.
    _receiver: mpsc::Receiver<()>,
}

impl IpcServerHandle {
    /// Stop the IPC server, closing the socket and cancelling the listener.
    pub fn stop(self) {
        info!("stopping IPC server");
        // Dropping `_receiver` cancels the spawned task.
        drop(self);
        // Clean up the socket file.
        let path = socket_path();
        if path.exists() {
            let _ = std::fs::remove_file(&path);
        }
    }
}

/// Start the IPC server in a background tokio task.
///
/// Returns an `IpcServerHandle` that can be used to stop the server. The server
/// binds to `~/.config/swai/swai.sock` (cleaning up any stale socket first).
pub fn start_ipc_server(state: Arc<IpcState>) -> Result<IpcServerHandle, io::Error> {
    let path = socket_path();

    // Ensure the config directory exists.
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    // Remove any stale socket from a previous crash.
    cleanup_stale_socket(&path);

    // Bind the Unix domain socket listener.
    let listener = UnixListener::bind(&path)?;
    listener.set_nonblocking(false)?;

    info!("IPC server listening on {:?}", path);

    let (tx, rx) = mpsc::channel::<()>(1);

    tokio::spawn(async move {
        loop {
            match listener.accept() {
                Ok((stream, addr)) => {
                    debug!("IPC client connected from {:?}", addr);
                    // Handle the request in a sub-task using blocking I/O.
                    let state = Arc::clone(&state);
                    tokio::task::spawn_blocking(move || {
                        if let Err(e) = handle_request_sync(stream, &state) {
                            error!("IPC request handler error: {}", e);
                        }
                        debug!("IPC client disconnected");
                    });
                }
                Err(e) => {
                    // Broken pipe / shutdown — exit the loop.
                    if e.kind() == io::ErrorKind::BrokenPipe {
                        info!("IPC listener broken pipe, shutting down");
                        break;
                    }
                    error!("IPC accept error: {}", e);
                }
            }
        }
        // Notify the handle that the server has stopped.
        let _ = tx.send(()).await;
    });

    Ok(IpcServerHandle { _receiver: rx })
}

/// Handle a single IPC client request over a Unix stream (synchronous).
fn handle_request_sync(stream: UnixStream, state: &IpcState) -> io::Result<()> {
    use std::io::Read;

    // Set a read timeout to prevent indefinite blocking.
    stream.set_read_timeout(Some(std::time::Duration::from_secs(5)))?;

    // Read the entire request body (single JSON message).
    let mut reader = io::BufReader::new(&stream);
    let mut buf = Vec::new();
    let _ = reader.read_to_end(&mut buf);

    // Parse the JSON request.
    let request: ActionRequest = match serde_json::from_slice(&buf) {
        Ok(r) => r,
        Err(e) => {
            let response = ActionResponse::error(format!("invalid JSON: {}", e));
            send_response_sync(&stream, &response)?;
            return Ok(());
        }
    };

    debug!("IPC request: action={}", request.action);

    // Dispatch the action with locked state.
    let mut pm = state.process_manager.lock().unwrap_or_else(|e| e.into_inner());
    let response = dispatch_action(request, &mut pm, state);
    send_response_sync(&stream, &response)?;

    Ok(())
}

/// Resolve a potentially cycling `next`/`prev` model_id to an actual model ID.
///
/// - `"next"` → the next model in `Config::models`, wrapping around from the
///   last index to 0. If no model is running, starts from index 0.
/// - `"prev"` → the previous model, wrapping from 0 to the last index. If no
///   model is running, starts from the last index.
/// - Anything else → returned unchanged (used as a literal model ID).
fn resolve_cycle_model_id(config: &Config, running_id: Option<&str>, candidate: &str) -> Option<String> {
    let models = &config.models;
    if models.is_empty() {
        return None;
    }

    match candidate {
        "next" | "prev" => {
            // Determine the current running model index.
            let current_idx = running_id.and_then(|rid| {
                models.iter().position(|m| m.id == rid)
            });

            let count = models.len();
            let new_idx = match (current_idx, candidate) {
                (Some(idx), "next") => (idx + 1) % count,
                (Some(idx), "prev") => {
                    if idx == 0 {
                        count - 1
                    } else {
                        idx - 1
                    }
                }
                // No model currently running: "next" → first, "prev" → last.
                (None, "next") => 0,
                (None, "prev") => count - 1,
                _ => unreachable!(),
            };

            Some(models[new_idx].id.clone())
        }
        other => {
            // Literal model ID — verify it exists in the config.
            if models.iter().any(|m| m.id == other) {
                Some(other.to_string())
            } else {
                None
            }
        }
    }
}

/// Dispatch a single IPC action against the shared state.
fn dispatch_action(
    request: ActionRequest,
    pm: &mut ProcessManager,
    state: &IpcState,
) -> ActionResponse {
    match request.action.as_str() {
        "status" => {
            let running = pm.get_primary_model_id();
            let message = match running {
                Some(id) => format!("Active model: {}", id),
                None => "No active model".to_string(),
            };
            ActionResponse::ok(message, None)
        }
        "list" => {
            let models: Vec<serde_json::Value> = state
                .config
                .models
                .iter()
                .map(|m| {
                    serde_json::json!({
                        "id": m.id,
                        "name": m.name,
                        "port": m.port,
                    })
                })
                .collect();
            let data = Some(serde_json::json!({"models": models}));
            ActionResponse::ok("Models listed", data)
        }
        "stop" => {
            match pm.stop_all(false) {
                Ok(()) => {
                    // Update proxy state to reflect stopped model.
                    if let Ok(mut ps) = state.proxy_state.lock() {
                        ps.clear();
                    }
                    ActionResponse::ok("Model stopped", None)
                }
                Err(e) => ActionResponse::error(format!("stop failed: {}", e)),
            }
        }
        "start" | "switch" => {
            let data = request.data.as_ref().and_then(|d| d.get("model_id").and_then(|v| v.as_str()));
            let model_id = match data {
                Some(id) => id,
                None => return ActionResponse::error(format!("missing model_id in {}", request.action)),
            };

            // Resolve cycling values (next/prev) to actual model IDs.
            let running_id = pm.get_primary_model_id();
            let resolved = match resolve_cycle_model_id(&state.config, running_id, model_id) {
                Some(id) => id,
                None => {
                    return ActionResponse::error(format!(
                        "model '{}' not found in configuration",
                        model_id
                    ));
                }
            };

            // Look up the target model config.
            let target = match state.config.models.iter().find(|m| m.id == resolved) {
                Some(m) => m.clone(),
                None => {
                    return ActionResponse::error(format!(
                        "model '{}' not found in configuration",
                        resolved
                    ));
                }
            };

            // Perform the switch.
            match request.action.as_str() {
                "start" => {
                    // Start a new model (don't stop existing first).
                    if let Err(e) = pm.start_model(&target.id) {
                        return ActionResponse::error(format!("start failed: {}", e));
                    }
                    // Update proxy state.
                    if let Ok(mut ps) = state.proxy_state.lock() {
                        ps.set_target(target.port);
                    }
                    let name = &target.name;
                    ActionResponse::ok(
                        format!("Started model '{}'", name),
                        Some(serde_json::json!({"model_id": resolved, "name": name})),
                    )
                }
                "switch" => {
                    // Stop current model then start the new one.
                    let from_id = pm.get_primary_model_id().map(|s| s.to_string());
                    match pm.switch_model(
                        from_id.as_deref().unwrap_or(""),
                        &target.id,
                    ) {
                        Ok(()) => {
                            if let Ok(mut ps) = state.proxy_state.lock() {
                                ps.set_target(target.port);
                            }
                            let name = &target.name;
                            ActionResponse::ok(
                                format!("Switched to '{}'", name),
                                Some(serde_json::json!({"model_id": resolved, "name": name})),
                            )
                        }
                        Err(e) => ActionResponse::error(format!("switch failed: {}", e)),
                    }
                }
                _ => unreachable!(),
            }
        }
        other => {
            ActionResponse::error(format!("unknown action: {}", other))
        }
    }
}

/// Send a JSON response over a Unix stream (synchronous).
fn send_response_sync(stream: &UnixStream, response: &ActionResponse) -> io::Result<()> {
    let body = serde_json::to_string(response).map_err(|e| {
        io::Error::other(format!("response serialization error: {}", e))
    })?;

    use std::io::Write;
    let mut writer = io::BufWriter::new(stream);
    writer.write_all(body.as_bytes())?;
    writer.write_all(b"\n")?;
    writer.flush()?;

    Ok(())
}

// ---------------------------------------------------------------------------
// IPC Client (used by CLI subcommands)
// ---------------------------------------------------------------------------

/// Custom error type for IPC client operations.
#[derive(Debug)]
pub enum IpcClientError {
    /// The socket file does not exist — SWAI is not running.
    SocketNotFound,
    /// Connection refused — the server is not accepting connections.
    ConnectionRefused,
    /// An I/O error occurred during communication.
    Io(io::Error),
    /// The server returned an error response.
    ServerError(String),
}

impl std::fmt::Display for IpcClientError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            IpcClientError::SocketNotFound => {
                write!(f, "IPC socket not found — is SWAI running?")
            }
            IpcClientError::ConnectionRefused => {
                write!(f, "connection refused — is SWAI running?")
            }
            IpcClientError::Io(e) => write!(f, "I/O error: {}", e),
            IpcClientError::ServerError(msg) => write!(f, "server error: {}", msg),
        }
    }
}

impl std::error::Error for IpcClientError {}

impl From<io::Error> for IpcClientError {
    fn from(e: io::Error) -> Self {
        if e.kind() == io::ErrorKind::NotFound {
            IpcClientError::SocketNotFound
        } else if e.kind() == io::ErrorKind::ConnectionRefused {
            IpcClientError::ConnectionRefused
        } else {
            IpcClientError::Io(e)
        }
    }
}

/// Connect to the IPC server and send an action request.
///
/// Returns the parsed `ActionResponse`, or an `IpcClientError` if the
/// connection fails or the server returns an error.
pub fn ipc_send(request: &ActionRequest) -> Result<ActionResponse, IpcClientError> {
    let path = socket_path();

    // Check that the socket file exists first for a friendlier error message.
    if !path.exists() {
        return Err(IpcClientError::SocketNotFound);
    }

    // Connect to the Unix domain socket.
    let stream = UnixStream::connect(&path)?;
    stream.set_read_timeout(Some(std::time::Duration::from_secs(10)))?;

    // Serialize and send the request.
    let body = serde_json::to_string(request).map_err(|e| {
        IpcClientError::Io(io::Error::other(
            format!("request serialization error: {}", e),
        ))
    })?;

    use std::io::Write;
    let mut writer = io::BufWriter::new(&stream);
    writer.write_all(body.as_bytes())?;
    writer.write_all(b"\n")?;
    writer.flush()?;

    // Read the response.
    use std::io::BufRead;
    let mut reader = io::BufReader::new(&stream);
    let mut response_buf = String::new();
    reader.read_line(&mut response_buf)?;

    // Parse the JSON response.
    let response: ActionResponse = serde_json::from_str(&response_buf).map_err(|e| {
        IpcClientError::Io(io::Error::other(
            format!("response parse error: {}", e),
        ))
    })?;

    if response.status == "error" {
        return Err(IpcClientError::ServerError(response.message));
    }

    Ok(response)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{GlobalSettings, ModelConfig, PreferencesConfig};
    use std::io::{BufRead, BufReader};
    use std::os::unix::net::UnixStream;
    use tempfile::TempDir;

    // Helper: read a line from a UnixStream using BufReader.
    fn read_response(client: &mut UnixStream) -> String {
        let mut reader = BufReader::new(client);
        let mut buf = String::new();
        reader.read_line(&mut buf).unwrap();
        buf
    }

    // -- Serialization tests ------------------------------------------------

    #[test]
    fn test_action_request_status_serialization() {
        let req = ActionRequest {
            action: "status".to_string(),
            data: None,
        };
        let json = serde_json::to_string(&req).unwrap();
        assert!(json.contains("\"action\":\"status\""));
        assert!(!json.contains("\"data\"")); // data is skipped when None
    }

    #[test]
    fn test_action_request_start_serialization() {
        let req = ActionRequest {
            action: "start".to_string(),
            data: Some(serde_json::json!({"model_id": "llama-3"})),
        };
        let json = serde_json::to_string(&req).unwrap();
        assert!(json.contains("\"action\":\"start\""));
        assert!(json.contains("\"model_id\":\"llama-3\""));
    }

    #[test]
    fn test_action_request_deserialization() {
        let json = r#"{"action":"stop"}"#;
        let req: ActionRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.action, "stop");
        assert!(req.data.is_none());
    }

    #[test]
    fn test_action_request_deserialization_with_data() {
        let json = r#"{"action":"start","data":{"model_id":"qwen-2.5"}}"#;
        let req: ActionRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.action, "start");
        assert_eq!(
            req.data.unwrap()["model_id"].as_str().unwrap(),
            "qwen-2.5"
        );
    }

    #[test]
    fn test_action_response_ok_serialization() {
        let resp = ActionResponse::ok("all good", Some(serde_json::json!({"model": "llama"})));
        let json = serde_json::to_string(&resp).unwrap();
        assert!(json.contains("\"status\":\"ok\""));
        assert!(json.contains("\"message\":\"all good\""));
        assert!(json.contains("\"model\":\"llama\""));
    }

    #[test]
    fn test_action_response_error_serialization() {
        let resp = ActionResponse::error("something broke");
        let json = serde_json::to_string(&resp).unwrap();
        assert!(json.contains("\"status\":\"error\""));
        assert!(json.contains("\"message\":\"something broke\""));
        assert!(!json.contains("\"data\"")); // data is skipped when None
    }

    #[test]
    fn test_action_response_deserialization() {
        let json = r#"{"status":"ok","message":"ready","data":{"port":9080}}"#;
        let resp: ActionResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.status, "ok");
        assert_eq!(resp.message, "ready");
        assert_eq!(resp.data.unwrap()["port"].as_i64().unwrap(), 9080);
    }

    #[test]
    fn test_action_response_roundtrip() {
        let original = ActionResponse::ok(
            "switched",
            Some(serde_json::json!({
                "model_id": "test-model",
                "port": 1234,
                "proxy_port": 9080
            })),
        );
        let json = serde_json::to_string(&original).unwrap();
        let decoded: ActionResponse = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded.status, original.status);
        assert_eq!(decoded.message, original.message);
        assert_eq!(
            decoded.data.unwrap()["model_id"].as_str().unwrap(),
            "test-model"
        );
    }

    #[test]
    fn test_action_request_roundtrip() {
        let original = ActionRequest {
            action: "switch".to_string(),
            data: Some(serde_json::json!({"model_id": "phi-3"})),
        };
        let json = serde_json::to_string(&original).unwrap();
        let decoded: ActionRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded.action, original.action);
        assert_eq!(
            decoded.data.unwrap()["model_id"].as_str().unwrap(),
            "phi-3"
        );
    }

    // -- Socket handling tests ------------------------------------------------

    #[test]
    fn test_socket_path_is_in_config_dir() {
        let path = socket_path();
        assert!(path.to_string_lossy().contains(".config/swai/"));
        assert!(path.to_string_lossy().ends_with("swai.sock"));
    }

    #[test]
    fn test_cleanup_stale_socket() {
        let tmp = TempDir::new().unwrap();
        let socket = tmp.path().join("test.sock");

        // Create a dummy file at the socket path.
        std::fs::write(&socket, "stale").unwrap();
        assert!(socket.exists());

        cleanup_stale_socket(&socket);
        assert!(!socket.exists());
    }

    #[test]
    fn test_cleanup_stale_socket_nonexistent() {
        let tmp = TempDir::new().unwrap();
        let socket = tmp.path().join("nonexistent.sock");
        // Should not panic when the file doesn't exist.
        cleanup_stale_socket(&socket);
    }

    #[test]
    fn test_ipc_server_accepts_and_responds() {
        let tmp = TempDir::new().unwrap();
        let socket_path = tmp.path().join("test.sock");

        // Bind a Unix listener manually (simulating what start_ipc_server does).
        let listener = UnixListener::bind(&socket_path).unwrap();

        std::thread::spawn(move || {
            if let Ok((stream, _)) = listener.accept() {
                // Echo back a response.
                let response = ActionResponse::ok("connected", None);
                let body = serde_json::to_string(&response).unwrap();
                use std::io::Write;
                let mut writer = std::io::BufWriter::new(stream);
                writer.write_all(body.as_bytes()).unwrap();
                writer.write_all(b"\n").unwrap();
                writer.flush().unwrap();
            }
        });

        // Connect a client.
        let mut client = UnixStream::connect(&socket_path).unwrap();
        let request = ActionRequest {
            action: "status".to_string(),
            data: None,
        };
        let body = serde_json::to_string(&request).unwrap();
        use std::io::Write;
        client.write_all(body.as_bytes()).unwrap();
        client.write_all(b"\n").unwrap();
        client.flush().unwrap();

        // Read the response.
        let response_line = read_response(&mut client);
        let response: ActionResponse = serde_json::from_str(response_line.trim()).unwrap();

        assert_eq!(response.status, "ok");
        assert_eq!(response.message, "connected");
    }

    #[test]
    #[serial_test::serial]
    fn test_ipc_client_socket_not_found() {
        let old_home = std::env::var("HOME").ok();
        let req = ActionRequest {
            action: "status".to_string(),
            data: None,
        };
        // Without HOME set, socket_path will be relative — and the file won't exist.
        std::env::remove_var("HOME");
        let result = ipc_send(&req);
        if let Some(ref h) = old_home {
            std::env::set_var("HOME", h);
        }
        assert!(result.is_err());
        match result.unwrap_err() {
            IpcClientError::SocketNotFound => {} // expected
            other => panic!("expected SocketNotFound, got {:?}", other),
        }
    }

    #[test]
    fn test_ipc_state_creation() {
        // Create a minimal config for testing.
        let tmp = tempfile::tempdir().unwrap();
        let script_path = tmp.path().join("dummy.sh");
        std::fs::write(&script_path, "#!/bin/sh\necho ok\n").ok();
        #[cfg(unix)]
        std::process::Command::new("chmod")
            .arg("+x")
            .arg(&script_path)
            .status()
            .ok();

        let config_content = format!(
            "schema_version = 1\n\n[[models]]\nid = \"test\"\nname = \"Test\"\nscript_path = \"{}\"\nport = 9999\nhealth_timeout_sec = 5\n",
            script_path.display()
        );
        let config: Config = toml::from_str(&config_content).unwrap();
        let state = IpcState::new(config);

        assert_eq!(state.config.models.len(), 1);
        assert_eq!(state.config.models[0].id, "test");
    }

    #[test]
    fn test_error_display_messages() {
        assert!(format!("{}", IpcClientError::SocketNotFound).contains("SWAI"));
        assert!(format!("{}", IpcClientError::ConnectionRefused).contains("SWAI"));
        assert!(format!(
            "{}",
            IpcClientError::ServerError("test error".to_string())
        )
        .contains("test error"));
    }

    #[test]
    fn test_invalid_json_request_handling() {
        let tmp = TempDir::new().unwrap();
        let socket_path = tmp.path().join("test.sock");

        let listener = UnixListener::bind(&socket_path).unwrap();

        std::thread::spawn(move || {
            if let Ok((mut stream, _)) = listener.accept() {
                // Read the request.
                use std::io::Read;
                let mut reader = std::io::BufReader::new(&stream);
                let mut buf = Vec::new();
                let _ = reader.read_to_end(&mut buf);

                // Try to parse invalid JSON — should fail.
                let result: Result<ActionRequest, _> = serde_json::from_slice(&buf);
                assert!(result.is_err());

                // Send an error response.
                let response = ActionResponse::error("invalid request");
                use std::io::Write;
                let body = serde_json::to_string(&response).unwrap();
                stream.write_all(body.as_bytes()).unwrap();
                stream.write_all(b"\n").unwrap();
                stream.flush().unwrap();
            }
        });

        // Client: connect and send invalid JSON.
        let mut client = UnixStream::connect(&socket_path).unwrap();
        use std::io::Write;
        client.write_all(b"not valid json{{{").unwrap();
        client.write_all(b"\n").unwrap();
        client.flush().unwrap();

        // Close the write half to signal EOF to the server.
        drop(client);
    }

    #[test]
    fn test_response_data_serialization_skips_none() {
        // Verify that `data: None` is omitted from JSON output.
        let resp = ActionResponse::ok("no data", None);
        let json = serde_json::to_string(&resp).unwrap();
        assert!(!json.contains("\"data\""));

        // And verify it round-trips correctly (data becomes None after deserialization).
        let decoded: ActionResponse = serde_json::from_str(&json).unwrap();
        assert!(decoded.data.is_none());
    }

    #[test]
    fn test_response_data_serialization_includes_some() {
        // Verify that `data: Some(...)` is included in JSON output.
        let resp = ActionResponse::ok("has data", Some(serde_json::json!({"x": 1})));
        let json = serde_json::to_string(&resp).unwrap();
        assert!(json.contains("\"data\""));
        assert!(json.contains("\"x\":1"));
    }

    #[test]
    fn test_config_dir_fallback() {
        // Verify that config_dir returns a path containing ".config/swai".
        // We don't remove HOME here to avoid affecting other tests.
        let dir = config_dir();
        assert!(dir.to_string_lossy().contains(".config/swai"));
    }

    #[test]
    fn test_handle_request_sync_invalid_json() {
        let tmp = TempDir::new().unwrap();
        let socket_path = tmp.path().join("test.sock");

        // Create a minimal IpcState for the handler.
        let script_path = tmp.path().join("dummy.sh");
        std::fs::write(&script_path, "#!/bin/sh\necho ok\n").ok();
        #[cfg(unix)]
        std::process::Command::new("chmod")
            .arg("+x")
            .arg(&script_path)
            .status()
            .ok();
        let config_content = format!(
            "schema_version = 1\n\n[[models]]\nid = \"test\"\nname = \"Test\"\nscript_path = \"{}\"\nport = 9999\nhealth_timeout_sec = 5\n",
            script_path.display()
        );
        let config: Config = toml::from_str(&config_content).unwrap();
        let mut state = IpcState::new(config);

        let listener = UnixListener::bind(&socket_path).unwrap();

        // Server thread: accept, read invalid JSON, send error response.
        std::thread::spawn(move || {
            if let Ok((stream, _)) = listener.accept() {
                let _ = handle_request_sync(stream, &mut state);
            }
        });

        // Client: connect and send invalid JSON.
        let mut client = UnixStream::connect(&socket_path).unwrap();
        use std::io::Write;
        client.write_all(b"not valid json{{{").unwrap();
        client.write_all(b"\n").unwrap();
        client.flush().unwrap();

        // Close the write half to signal EOF to the server using safe Rust std::net::Shutdown.
        client.shutdown(std::net::Shutdown::Write).unwrap();

        // Read the error response.
        let response_line = read_response(&mut client);
        let response: ActionResponse = serde_json::from_str(response_line.trim()).unwrap();

        assert_eq!(response.status, "error");
        assert!(response.message.contains("invalid JSON"));
    }

    #[test]
    fn test_handle_request_sync_valid_request() {
        let tmp = TempDir::new().unwrap();
        let socket_path = tmp.path().join("test.sock");

        // Create a minimal IpcState for the handler.
        let script_path = tmp.path().join("dummy.sh");
        std::fs::write(&script_path, "#!/bin/sh\necho ok\n").ok();
        #[cfg(unix)]
        std::process::Command::new("chmod")
            .arg("+x")
            .arg(&script_path)
            .status()
            .ok();
        let config_content = format!(
            "schema_version = 1\n\n[[models]]\nid = \"test\"\nname = \"Test\"\nscript_path = \"{}\"\nport = 9999\nhealth_timeout_sec = 5\n",
            script_path.display()
        );
        let config: Config = toml::from_str(&config_content).unwrap();
        let mut state = IpcState::new(config);

        let listener = UnixListener::bind(&socket_path).unwrap();

        // Server thread: accept and handle the request.
        std::thread::spawn(move || {
            if let Ok((stream, _)) = listener.accept() {
                let _ = handle_request_sync(stream, &mut state);
            }
        });

        // Client: connect and send a valid status request.
        let mut client = UnixStream::connect(&socket_path).unwrap();
        use std::io::Write;
        let request = ActionRequest {
            action: "status".to_string(),
            data: None,
        };
        let body = serde_json::to_string(&request).unwrap();
        client.write_all(body.as_bytes()).unwrap();
        client.write_all(b"\n").unwrap();
        client.flush().unwrap();

        // Close the write half to signal EOF to the server using safe Rust std::net::Shutdown.
        client.shutdown(std::net::Shutdown::Write).unwrap();

        // Read the response.
        let response_line = read_response(&mut client);
        let response: ActionResponse = serde_json::from_str(response_line.trim()).unwrap();

        assert_eq!(response.status, "ok");
        assert!(response.message.contains("No active model"));
    }

    // -- Model cycling tests --------------------------------------------------

    fn make_test_state(models: Vec<ModelConfig>) -> IpcState {
        let config = Config {
            schema_version: 1,
            models,
            global: GlobalSettings::default(),
            preferences: PreferencesConfig::default(),
        };
        IpcState::new(config)
    }

    /// Minimal ProcessGuard for unit tests — never starts or terminates anything.
    struct DummyGuard;
    impl crate::process_manager::ProcessGuard for DummyGuard {
        fn setup(_script: &std::path::Path, _port: u16, _log_dir: &std::path::Path) -> Result<Self, crate::process_manager::ProcessError>
        where
            Self: Sized,
        {
            Ok(DummyGuard)
        }
        fn terminate(&self, _fast_shutdown: bool) -> Result<(), crate::process_manager::ProcessError> {
            Ok(())
        }
    }

    fn make_running_model(_id: &str, _name: &str, _port: u16) -> crate::process_manager::RunningModel {
        crate::process_manager::RunningModel {
            id: _id.to_string(),
            guard: Box::new(DummyGuard),
            state: crate::process_manager::ModelState::Ready,
        }
    }

    fn make_state_with_running(
        models: Vec<ModelConfig>,
        running_id: &str,
    ) -> IpcState {
        let config = Config {
            schema_version: 1,
            models: models.clone(),
            global: GlobalSettings::default(),
            preferences: PreferencesConfig::default(),
        };
        let mut pm = crate::process_manager::ProcessManager::new(config.clone());
        // Find the running model's port.
        let port = models.iter()
            .find(|m| m.id == running_id)
            .map(|m| m.port)
            .unwrap_or(0);
        let name = models.iter()
            .find(|m| m.id == running_id)
            .map(|m| m.name.clone())
            .unwrap_or_default();
        pm.set_running_model(make_running_model(running_id, &name, port));
        IpcState {
            process_manager: std::sync::Mutex::new(pm),
            proxy_state: Arc::new(Mutex::new(ProxyState::new())),
            config,
        }
    }

    #[test]
    fn test_resolve_cycle_next_no_running_wraps_to_first() {
        let state = make_test_state(vec![
            ModelConfig {
                id: "m1".into(), name: "M1".into(), script_path: "/tmp/x".into(),
                port: 8001, health_timeout_sec: 5,
            },
            ModelConfig {
                id: "m2".into(), name: "M2".into(), script_path: "/tmp/x".into(),
                port: 8002, health_timeout_sec: 5,
            },
        ]);
        assert_eq!(resolve_cycle_model_id(&state.config, None, "next"), Some("m1".to_string()));
    }

    #[test]
    fn test_resolve_cycle_prev_no_running_wraps_to_last() {
        let state = make_test_state(vec![
            ModelConfig {
                id: "m1".into(), name: "M1".into(), script_path: "/tmp/x".into(),
                port: 8001, health_timeout_sec: 5,
            },
            ModelConfig {
                id: "m2".into(), name: "M2".into(), script_path: "/tmp/x".into(),
                port: 8002, health_timeout_sec: 5,
            },
        ]);
        assert_eq!(resolve_cycle_model_id(&state.config, None, "prev"), Some("m2".to_string()));
    }

    #[test]
    fn test_resolve_cycle_next_advances_index() {
        let state = make_state_with_running(
            vec![
                ModelConfig {
                    id: "m1".into(), name: "M1".into(), script_path: "/tmp/x".into(),
                    port: 8001, health_timeout_sec: 5,
                },
                ModelConfig {
                    id: "m2".into(), name: "M2".into(), script_path: "/tmp/x".into(),
                    port: 8002, health_timeout_sec: 5,
                },
                ModelConfig {
                    id: "m3".into(), name: "M3".into(), script_path: "/tmp/x".into(),
                    port: 8003, health_timeout_sec: 5,
                },
            ],
            "m1",
        );
        let pm = state.process_manager.lock().unwrap_or_else(|e| e.into_inner());
        assert_eq!(resolve_cycle_model_id(&state.config, pm.get_primary_model_id(), "next"), Some("m2".to_string()));
    }

    #[test]
    fn test_resolve_cycle_prev_wraps_from_first_to_last() {
        let state = make_state_with_running(
            vec![
                ModelConfig {
                    id: "m1".into(), name: "M1".into(), script_path: "/tmp/x".into(),
                    port: 8001, health_timeout_sec: 5,
                },
                ModelConfig {
                    id: "m2".into(), name: "M2".into(), script_path: "/tmp/x".into(),
                    port: 8002, health_timeout_sec: 5,
                },
            ],
            "m1",
        );
        let pm = state.process_manager.lock().unwrap_or_else(|e| e.into_inner());
        assert_eq!(resolve_cycle_model_id(&state.config, pm.get_primary_model_id(), "prev"), Some("m2".to_string()));
    }

    #[test]
    fn test_resolve_cycle_next_wraps_from_last_to_first() {
        let state = make_state_with_running(
            vec![
                ModelConfig {
                    id: "m1".into(), name: "M1".into(), script_path: "/tmp/x".into(),
                    port: 8001, health_timeout_sec: 5,
                },
                ModelConfig {
                    id: "m2".into(), name: "M2".into(), script_path: "/tmp/x".into(),
                    port: 8002, health_timeout_sec: 5,
                },
            ],
            "m2",
        );
        let pm = state.process_manager.lock().unwrap_or_else(|e| e.into_inner());
        assert_eq!(resolve_cycle_model_id(&state.config, pm.get_primary_model_id(), "next"), Some("m1".to_string()));
    }

    #[test]
    fn test_resolve_literal_model_id_returns_id() {
        let state = make_test_state(vec![
            ModelConfig {
                id: "m1".into(), name: "M1".into(), script_path: "/tmp/x".into(),
                port: 8001, health_timeout_sec: 5,
            },
        ]);
        assert_eq!(resolve_cycle_model_id(&state.config, None, "m1"), Some("m1".to_string()));
    }

    #[test]
    fn test_resolve_unknown_literal_returns_none() {
        let state = make_test_state(vec![
            ModelConfig {
                id: "m1".into(), name: "M1".into(), script_path: "/tmp/x".into(),
                port: 8001, health_timeout_sec: 5,
            },
        ]);
        assert_eq!(resolve_cycle_model_id(&state.config, None, "nonexistent"), None);
    }

    #[test]
    fn test_resolve_empty_models_returns_none() {
        let state = make_test_state(vec![]);
        assert_eq!(resolve_cycle_model_id(&state.config, None, "next"), None);
        assert_eq!(resolve_cycle_model_id(&state.config, None, "prev"), None);
    }

    #[test]
    fn test_dispatch_status_no_active_model() {
        let state = make_test_state(vec![
            ModelConfig {
                id: "m1".into(), name: "M1".into(), script_path: "/tmp/x".into(),
                port: 8001, health_timeout_sec: 5,
            },
        ]);
        let req = ActionRequest {
            action: "status".to_string(),
            data: None,
        };
        let mut pm = state.process_manager.lock().unwrap_or_else(|e| e.into_inner());
        let resp = dispatch_action(req, &mut pm, &state);
        assert_eq!(resp.status, "ok");
        assert_eq!(resp.message, "No active model");
    }

    #[test]
    fn test_dispatch_switch_with_next_cycling() {
        let state = make_state_with_running(
            vec![
                ModelConfig {
                    id: "m1".into(), name: "M1".into(), script_path: "/tmp/x".into(),
                    port: 8001, health_timeout_sec: 5,
                },
                ModelConfig {
                    id: "m2".into(), name: "M2".into(), script_path: "/tmp/x".into(),
                    port: 8002, health_timeout_sec: 5,
                },
            ],
            "m1",
        );

        let req = ActionRequest {
            action: "switch".to_string(),
            data: Some(serde_json::json!({"model_id": "next"})),
        };
        let mut pm = state.process_manager.lock().unwrap_or_else(|e| e.into_inner());
        let resp = dispatch_action(req, &mut pm, &state);
        // The switch will fail because m2's script doesn't exist on disk, but
        // we can verify the cycling resolved correctly.
        assert_eq!(resp.status, "error");
        assert!(resp.message.contains("switch failed"));

        // Verify that resolve_cycle_model_id resolved to m2.
        assert_eq!(resolve_cycle_model_id(&state.config, Some("m1"), "next"), Some("m2".to_string()));
    }

    #[test]
    fn test_dispatch_switch_with_prev_cycling() {
        let state = make_state_with_running(
            vec![
                ModelConfig {
                    id: "m1".into(), name: "M1".into(), script_path: "/tmp/x".into(),
                    port: 8001, health_timeout_sec: 5,
                },
                ModelConfig {
                    id: "m2".into(), name: "M2".into(), script_path: "/tmp/x".into(),
                    port: 8002, health_timeout_sec: 5,
                },
            ],
            "m2",
        );

        let req = ActionRequest {
            action: "switch".to_string(),
            data: Some(serde_json::json!({"model_id": "prev"})),
        };
        let mut pm = state.process_manager.lock().unwrap_or_else(|e| e.into_inner());
        let resp = dispatch_action(req, &mut pm, &state);
        assert_eq!(resp.status, "error");
        assert!(resp.message.contains("switch failed"));

        // Verify that resolve_cycle_model_id resolved to m1.
        assert_eq!(resolve_cycle_model_id(&state.config, Some("m2"), "prev"), Some("m1".to_string()));
    }

    #[test]
    fn test_dispatch_unknown_action_returns_error() {
        let state = make_test_state(vec![]);
        let req = ActionRequest {
            action: "foobar".to_string(),
            data: None,
        };
        let mut pm = state.process_manager.lock().unwrap_or_else(|e| e.into_inner());
        let resp = dispatch_action(req, &mut pm, &state);
        assert_eq!(resp.status, "error");
        assert!(resp.message.contains("unknown action"));
    }

    #[test]
    fn test_dispatch_switch_missing_model_id_returns_error() {
        let state = make_test_state(vec![]);
        let req = ActionRequest {
            action: "switch".to_string(),
            data: Some(serde_json::json!({})),
        };
        let mut pm = state.process_manager.lock().unwrap_or_else(|e| e.into_inner());
        let resp = dispatch_action(req, &mut pm, &state);
        assert_eq!(resp.status, "error");
        assert!(resp.message.contains("missing model_id"));
    }

    #[test]
    fn test_dispatch_switch_nonexistent_model_returns_error() {
        let state = make_test_state(vec![]);
        let req = ActionRequest {
            action: "switch".to_string(),
            data: Some(serde_json::json!({"model_id": "ghost"})),
        };
        let mut pm = state.process_manager.lock().unwrap_or_else(|e| e.into_inner());
        let resp = dispatch_action(req, &mut pm, &state);
        assert_eq!(resp.status, "error");
        assert!(resp.message.contains("not found"));
    }

    #[test]
    fn test_dispatch_list_returns_models() {
        let state = make_test_state(vec![
            ModelConfig {
                id: "m1".into(), name: "M1".into(), script_path: "/tmp/x".into(),
                port: 8001, health_timeout_sec: 5,
            },
        ]);
        let req = ActionRequest {
            action: "list".to_string(),
            data: None,
        };
        let mut pm = state.process_manager.lock().unwrap_or_else(|e| e.into_inner());
        let resp = dispatch_action(req, &mut pm, &state);
        assert_eq!(resp.status, "ok");
        let data = resp.data.unwrap();
        let models = data["models"].as_array().unwrap();
        assert_eq!(models.len(), 1);
        assert_eq!(models[0]["id"].as_str().unwrap(), "m1");
    }

    #[test]
    fn test_dispatch_stop_clears_proxy() {
        let state = make_test_state(vec![]);
        // Set proxy state to a target.
        {
            let mut ps = state.proxy_state.lock().unwrap_or_else(|e| e.into_inner());
            ps.set_target(8001);
        }
        let req = ActionRequest {
            action: "stop".to_string(),
            data: None,
        };
        let mut pm = state.process_manager.lock().unwrap_or_else(|e| e.into_inner());
        let resp = dispatch_action(req, &mut pm, &state);
        assert_eq!(resp.status, "ok");
        // Proxy should be cleared.
        let ps = state.proxy_state.lock().unwrap_or_else(|e| e.into_inner());
        assert!(ps.primary_port.is_none());
        assert!(ps.active_models.is_empty());
    }
}
