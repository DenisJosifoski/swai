use std::sync::{Arc, Mutex};
use serde::{Deserialize, Serialize};

use crate::config::Config;
use crate::process_manager::ProcessManager;
use crate::proxy::ProxyState;

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

/// Shared state owned by the IPC server.
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
