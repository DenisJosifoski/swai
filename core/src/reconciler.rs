//! Startup reconciliation — checks for models that may have been running
//! before SWAI started and determines the initial state.
//!
//! Also provides an unmanaged-server prober that scans common LLM ports on
//! localhost, filters out those already configured in `Config::models`, and
//! performs a short HTTP GET `/v1/models` to discover model names from any
//! OpenAI-compatible or Ollama server found listening.

use crate::config::Config;
use crate::process_manager::{PortState, ProcessError};
use serde::Deserialize;
use std::collections::HashSet;
use std::net::TcpStream;
use std::time::Duration;
use tracing::{debug, info, warn};

/// Common LLM server ports to probe during unmanaged-server discovery.
const COMMON_LLM_PORTS: &[u16] = &[8000, 8080, 8081, 11434];

/// Information about an unmanaged (not in Config::models) LLM server found
/// on a candidate port during probing.
#[derive(Debug, Clone)]
pub struct UnmanagedModelInfo {
    /// The local TCP port the server is listening on.
    pub port: u16,
    /// The discovered model name extracted from the server's `/v1/models`
    /// response (e.g. `"gpt-3.5-turbo"`, `"llama-3"`, etc.).
    pub model_name: String,
}

/// Build an HTTP client with a 500ms timeout for probing candidate ports.
/// The short timeout prevents hanging on unresponsive listeners during the
/// startup scan.
fn probe_http_client() -> reqwest::blocking::Client {
    reqwest::blocking::Client::builder()
        .timeout(Duration::from_millis(500))
        .build()
        .expect("reqwest client build failed")
}

/// Result of the startup reconciliation.
#[derive(Debug)]
pub enum ReconcileResult {
    /// Exactly one model is running — return its id.
    OneRunning(String),
    /// Multiple models are running — surface a warning.
    MultipleRunning(Vec<String>),
    /// No models are running — all start in Stopped state.
    NoneRunning,
}

/// Reconciler — checks for running models at startup.
pub struct Reconciler {
    config: Config,
}

impl Reconciler {
    /// Create a new reconciler from a loaded config.
    pub fn new(config: Config) -> Self {
        Self { config }
    }

    /// Run the reconciliation.
    pub fn reconcile(&self) -> Result<ReconcileResult, ProcessError> {
        let mut running_ids: Vec<String> = Vec::new();

        for model in &self.config.models {
            match PortState::from_port_check(model.port) {
                PortState::OccupiedByModel => {
                    debug!("found running model '{}' on port {}", model.id, model.port);
                    running_ids.push(model.id.clone());
                }
                PortState::OccupiedByUnknown(pid) => {
                    warn!(
                        "port {} occupied by unknown process (pid: {})",
                        model.port, pid
                    );
                }
                PortState::Free => {}
            }
        }

        let result = match running_ids.len() {
            0 => ReconcileResult::NoneRunning,
            1 => ReconcileResult::OneRunning(running_ids.into_iter().next().unwrap()),
            _ => ReconcileResult::MultipleRunning(running_ids),
        };

        info!("reconciliation result: {:?}", result);
        Ok(result)
    }

    /// Probe common local ports for unmanaged LLM servers that are not present
    /// in `Config::models`.
    ///
    /// For each candidate port in [`COMMON_LLM_PORTS`] that is **not** already
    /// used by a configured model, this function:
    /// 1. Sends an HTTP GET `/v1/models` with a 500ms timeout.
    /// 2. Parses the JSON response body looking for either:
    ///    - OpenAI-compatible format: `data[0].id`
    ///    - Ollama-compatible format: `models[0].name`
    /// 3. Returns a `Vec<UnmanagedModelInfo>` containing the port and discovered
    ///    model name for every responding server found.
    pub fn probe_unmanaged_servers(&self) -> Vec<UnmanagedModelInfo> {
        let configured_ports: HashSet<u16> = self.config.models.iter().map(|m| m.port).collect();

        let client = probe_http_client();
        let mut results = Vec::new();

        for &port in COMMON_LLM_PORTS {
            // Skip ports already configured
            if configured_ports.contains(&port) {
                debug!("skipping configured port {}", port);
                continue;
            }

            // Quick TCP connect check — avoids an HTTP round-trip on closed ports
            if TcpStream::connect(format!("127.0.0.1:{}", port)).is_err() {
                continue;
            }

            let url = format!("http://127.0.0.1:{}/v1/models", port);
            debug!("probing {} for unmanaged server", url);

            if let Ok(resp) = client.get(&url).send() {
                if !resp.status().is_success() {
                    continue;
                }

                if let Ok(body) = resp.text() {
                    if let Some(name) = extract_model_name_from_response(&body) {
                        info!("discovered unmanaged model '{}' on port {}", name, port);
                        results.push(UnmanagedModelInfo {
                            port,
                            model_name: name,
                        });
                    }
                }
            }
        }

        if !results.is_empty() {
            info!("found {} unmanaged LLM server(s)", results.len());
        }

        results
    }
}

/// OpenAI-compatible `/v1/models` response shape (only the `data` array).
#[derive(Deserialize)]
struct OpenAIModelsResponse {
    data: Option<Vec<OpenAIModelEntry>>,
}

#[derive(Deserialize)]
struct OpenAIModelEntry {
    id: Option<String>,
}

/// Ollama-compatible `/api/tags` response shape.
#[derive(Deserialize)]
struct OllamaTagsResponse {
    models: Option<Vec<OllamaModelEntry>>,
}

#[derive(Deserialize)]
struct OllamaModelEntry {
    name: Option<String>,
}

/// Extract a model name from an HTTP `/v1/models` or `/api/tags` response body.
///
/// Tries OpenAI-compatible format first (`data[0].id`), then falls back to
/// Ollama-compatible format (`models[0].name`). Returns `None` if the payload
/// cannot be parsed or contains no usable model name.
fn extract_model_name_from_response(body: &str) -> Option<String> {
    // Try OpenAI-compatible format: {"data": [{"id": "..."}]}
    if let Ok(parsed) = serde_json::from_str::<OpenAIModelsResponse>(body) {
        if let Some(data) = parsed.data {
            if let Some(first) = data.first() {
                if let Some(id) = &first.id {
                    if !id.is_empty() {
                        return Some(id.clone());
                    }
                }
            }
        }
    }

    // Try Ollama-compatible format: {"models": [{"name": "..."}]}
    if let Ok(parsed) = serde_json::from_str::<OllamaTagsResponse>(body) {
        if let Some(models) = parsed.models {
            if let Some(first) = models.first() {
                if let Some(name) = &first.name {
                    if !name.is_empty() {
                        return Some(name.clone());
                    }
                }
            }
        }
    }

    None
}

/// Extension trait for PortState to check if a port is occupied by a llama-server.
trait PortCheck {
    fn from_port_check(port: u16) -> PortState;
}

impl PortCheck for PortState {
    fn from_port_check(port: u16) -> Self {
        // Try to connect to the port
        if TcpStream::connect(format!("127.0.0.1:{}", port)).is_ok() {
            // Port is occupied — probe it for /v1/models
            let url = format!("http://127.0.0.1:{}/v1/models", port);
            if let Ok(resp) = probe_http_client().get(&url).send() {
                if resp.status().is_success() {
                    // Check if it's a valid llama-server response
                    if let Ok(body) = resp.text() {
                        if body.contains("\"id\"") && body.contains("model") {
                            return PortState::OccupiedByModel;
                        }
                    }
                }
            }
            // Occupied but not a llama-server — try to get pid
            if let Ok(pid) = crate::process_manager::ProcessManager::get_port_pid(port) {
                return PortState::OccupiedByUnknown(pid);
            }
            PortState::OccupiedByUnknown(0)
        } else {
            PortState::Free
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_port_check_on_free_port() {
        // Port 9999 should be free
        let state = PortState::from_port_check(9999);
        assert_eq!(state, PortState::Free);
    }

    #[test]
    fn test_extract_model_name_openai_format() {
        let body = r#"{"data": [{"id": "gpt-3.5-turbo", "object": "model"}]}"#;
        assert_eq!(
            extract_model_name_from_response(body),
            Some("gpt-3.5-turbo".to_string())
        );
    }

    #[test]
    fn test_extract_model_name_openai_empty_data() {
        let body = r#"{"data": []}"#;
        assert_eq!(extract_model_name_from_response(body), None);
    }

    #[test]
    fn test_extract_model_name_ollama_format() {
        let body = r#"{"models": [{"name": "llama3:8b", "size": 4700000000}]}"#;
        assert_eq!(
            extract_model_name_from_response(body),
            Some("llama3:8b".to_string())
        );
    }

    #[test]
    fn test_extract_model_name_ollama_empty_models() {
        let body = r#"{"models": []}"#;
        assert_eq!(extract_model_name_from_response(body), None);
    }

    #[test]
    fn test_extract_model_name_invalid_json() {
        let body = "not json at all";
        assert_eq!(extract_model_name_from_response(body), None);
    }

    #[test]
    fn test_extract_model_name_empty_id() {
        let body = r#"{"data": [{"id": "", "object": "model"}]}"#;
        assert_eq!(extract_model_name_from_response(body), None);
    }

    #[test]
    fn test_extract_model_name_no_data_key() {
        let body = r#"{"something_else": true}"#;
        assert_eq!(extract_model_name_from_response(body), None);
    }

    #[test]
    fn test_extract_model_name_ollama_empty_name() {
        let body = r#"{"models": [{"name": ""}]}"#;
        assert_eq!(extract_model_name_from_response(body), None);
    }

    #[test]
    fn test_unmanaged_server_info_display() {
        let info = UnmanagedModelInfo {
            port: 11434,
            model_name: "llama3:8b".to_string(),
        };
        assert_eq!(info.port, 11434);
        assert_eq!(info.model_name, "llama3:8b");
    }

    #[test]
    fn test_probe_filters_configured_ports() {
        // Create a config with port 8080 configured.
        let config = Config {
            schema_version: 1,
            models: vec![crate::config::ModelConfig {
                id: "test-model".to_string(),
                name: "Test Model".to_string(),
                script_path: std::path::PathBuf::from("/dev/null"),
                port: 8080,
                health_timeout_sec: 30,
                ctx_size: 65_536,
            }],
            global: crate::config::GlobalSettings::default(),
            preferences: crate::config::PreferencesConfig::default(),
        };

        let reconciler = Reconciler::new(config);
        // Port 8080 should be skipped; result is empty since no servers are running.
        let results = reconciler.probe_unmanaged_servers();
        for r in &results {
            assert_ne!(r.port, 8080);
        }
    }

    #[test]
    fn test_probe_returns_empty_when_no_servers_running() {
        // With no servers on common ports, probing should return an empty vec.
        let config = Config {
            schema_version: 1,
            models: vec![],
            global: crate::config::GlobalSettings::default(),
            preferences: crate::config::PreferencesConfig::default(),
        };

        let reconciler = Reconciler::new(config);
        let results = reconciler.probe_unmanaged_servers();
        assert!(results.is_empty());
    }

    #[test]
    fn test_common_ports_constant() {
        // Verify the common ports list includes expected LLM server ports.
        assert!(COMMON_LLM_PORTS.contains(&8000));
        assert!(COMMON_LLM_PORTS.contains(&8080));
        assert!(COMMON_LLM_PORTS.contains(&8081));
        assert!(COMMON_LLM_PORTS.contains(&11434));
    }

    #[test]
    fn test_extract_model_name_mixed_format_fallback() {
        // A response with no `data` key but valid Ollama format should still work.
        let body = r#"{"models": [{"name": "qwen2:7b"}]}"#;
        assert_eq!(
            extract_model_name_from_response(body),
            Some("qwen2:7b".to_string())
        );
    }

    #[test]
    fn test_extract_model_name_openai_with_extra_fields() {
        // Response with extra fields should still parse correctly.
        let body = r#"{
            "object": "list",
            "data": [
                {"id": "gpt-4", "object": "model", "owned_by": "openai"},
                {"id": "gpt-3.5-turbo", "object": "model", "owned_by": "openai"}
            ]
        }"#;
        assert_eq!(
            extract_model_name_from_response(body),
            Some("gpt-4".to_string())
        );
    }
}
