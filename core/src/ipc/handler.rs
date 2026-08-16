use std::io;
use std::os::unix::net::UnixStream;
use tracing::debug;

use super::protocol::{ActionRequest, ActionResponse, IpcState};
use crate::config::Config;
use crate::process_manager::ProcessManager;

pub fn handle_request_sync(stream: UnixStream, state: &IpcState) -> io::Result<()> {
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
    let mut pm = state
        .process_manager
        .lock()
        .unwrap_or_else(|e| e.into_inner());
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
pub fn resolve_cycle_model_id(
    config: &Config,
    running_id: Option<&str>,
    candidate: &str,
) -> Option<String> {
    let models = &config.models;
    if models.is_empty() {
        return None;
    }

    match candidate {
        "next" | "prev" => {
            // Determine the current running model index.
            let current_idx = running_id.and_then(|rid| models.iter().position(|m| m.id == rid));

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
pub fn dispatch_action(
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
            let data = request
                .data
                .as_ref()
                .and_then(|d| d.get("model_id").and_then(|v| v.as_str()));
            let model_id = match data {
                Some(id) => id,
                None => {
                    return ActionResponse::error(format!("missing model_id in {}", request.action))
                }
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
                    match pm.switch_model(from_id.as_deref().unwrap_or(""), &target.id) {
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
        other => ActionResponse::error(format!("unknown action: {}", other)),
    }
}

/// Send a JSON response over a Unix stream (synchronous).
pub fn send_response_sync(stream: &UnixStream, response: &ActionResponse) -> io::Result<()> {
    let body = serde_json::to_string(response)
        .map_err(|e| io::Error::other(format!("response serialization error: {}", e)))?;

    use std::io::Write;
    let mut writer = io::BufWriter::new(stream);
    writer.write_all(body.as_bytes())?;
    writer.write_all(b"\n")?;
    writer.flush()?;

    Ok(())
}
