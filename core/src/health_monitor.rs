//! Health monitoring for a running model process.
//!
//! Polls `/v1/models` on the active model's port during startup,
//! transitioning the model state from Starting → Loading → Ready
//! (or Error if the health check timeout is exceeded).

use crate::process_manager::{ModelState, ProcessError};
use std::time::Duration;
use tracing::{debug, info, warn};

/// Build an HTTP client with a timeout to prevent indefinite hangs on
/// unresponsive or hung listeners during health monitoring.
fn http_client() -> reqwest::blocking::Client {
    reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .expect("reqwest client build failed")
}

/// Health monitor — polls `/v1/models` to track model startup progress.
pub struct HealthMonitor {
    port: u16,
    timeout_sec: u16,
}

impl HealthMonitor {
    /// Create a new health monitor for the given port and timeout.
    pub fn new(port: u16, timeout_sec: u16) -> Self {
        Self { port, timeout_sec }
    }

    /// Start monitoring until the model is Ready or the timeout is exceeded.
    #[allow(unused_assignments)]
    pub fn wait_until_ready(&self) -> Result<ModelState, ProcessError> {
        let deadline = std::time::Instant::now() + Duration::from_secs(self.timeout_sec as u64);
        let mut state = ModelState::Starting;

        loop {
            match self.fetch_model_info() {
                Ok((true, _)) => {
                    state = ModelState::Ready;
                    break;
                }
                Ok((false, Some(progress))) => {
                    // Model is still loading — report Loading with progress info.
                    debug!("model loading progress: {}", progress);
                    state = ModelState::Loading;
                }
                Ok((false, None)) => {
                    state = ModelState::Starting;
                }
                Err(e) => {
                    // Health check failed — model might still be starting up
                    warn!("health check failed: {}", e);
                    // Don't fail immediately — give it a few more seconds
                    if std::time::Instant::now() + Duration::from_secs(3) >= deadline {
                        state = ModelState::Error(format!(
                            "health check timeout after {}s",
                            self.timeout_sec
                        ));
                        break;
                    }
                }
            }

            if std::time::Instant::now() >= deadline {
                state = ModelState::Error(format!(
                    "health check timeout after {}s",
                    self.timeout_sec
                ));
                break;
            }

            // Wait 1 second before next poll
            std::thread::sleep(Duration::from_secs(1));
        }

        info!("model health state: {:?}", state);
        Ok(state)
    }

    /// Monitor health and report each state change through a channel sender.
    ///
    /// Sends `Starting` → `Loading` (repeatedly, as progress is detected) →
    /// `Ready` or `Error(msg)` through the channel. This is used by the UI to
    /// drive Starting → Loading → Ready transitions during model startup.
    pub fn wait_until_ready_with_updates(
        &self,
        tx: std::sync::mpsc::Sender<ModelState>,
    ) {
        let deadline = std::time::Instant::now() + Duration::from_secs(self.timeout_sec as u64);
        let mut last_state: ModelState = ModelState::Starting;

        // Send initial Starting state
        let _ = tx.send(ModelState::Starting);

        loop {
            let current_state = match self.fetch_model_info() {
                Ok((true, _)) => ModelState::Ready,
                Ok((false, Some(_))) => ModelState::Loading,
                Ok((false, None)) => ModelState::Starting,
                Err(e) => {
                    warn!("health check failed: {}", e);
                    // Keep current state, but timeout if we're past deadline + 3s
                    if std::time::Instant::now() + Duration::from_secs(3) >= deadline {
                        ModelState::Error(format!(
                            "health check timeout after {}s",
                            self.timeout_sec
                        ))
                    } else {
                        last_state.clone()
                    }
                }
            };

            // Only send if state changed (avoid spamming UI with repeated states)
            if current_state != last_state {
                let _ = tx.send(current_state.clone());
                last_state = current_state;
            }

            match &last_state {
                ModelState::Ready | ModelState::Error(_) => break,
                _ => {}
            }

            if std::time::Instant::now() >= deadline {
                let timeout_state = ModelState::Error(format!(
                    "health check timeout after {}s",
                    self.timeout_sec
                ));
                let _ = tx.send(timeout_state);
                break;
            }

            std::thread::sleep(Duration::from_secs(1));
        }
    }

    /// Fetch `/v1/models` once and return both health status and model ID.
    ///
    /// This replaces the previous two separate calls to `check_health()` and
    /// `get_loading_progress()`, eliminating redundant HTTP requests on every
    /// health-check tick. Returns `(is_healthy, model_id_or_none)`.
    fn fetch_model_info(&self) -> Result<(bool, Option<String>), ProcessError> {
        let url = format!("http://127.0.0.1:{}/v1/models", self.port);
        match http_client().get(&url).send() {
            Ok(resp) => {
                if resp.status().is_success() {
                    if let Ok(body) = resp.text() {
                        // Check for valid llama-server response shape.
                        let is_healthy = body.contains("\"id\"") && body.contains("model");

                        // Parse as JSON to safely extract the model id.
                        // This avoids panics from fragile string slicing
                        // when the response structure varies (e.g., when
                        // "id" appears in different positions).
                        let model_id = serde_json::from_str::<serde_json::Value>(&body)
                            .ok()
                            .and_then(|v| v.get("id").and_then(|id| id.as_str().map(|s| s.to_string())));

                        return Ok((is_healthy, model_id));
                    }
                }
                Ok((false, None))
            }
            Err(e) => Err(ProcessError::HealthCheckFailed(format!(
                "could not reach {}: {}",
                url, e
            ))),
        }
    }

}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_health_monitor_creation() {
        let monitor = HealthMonitor::new(8081, 30);
        assert_eq!(monitor.port, 8081);
        assert_eq!(monitor.timeout_sec, 30);
    }

    #[test]
    fn test_fetch_model_info_on_free_port() {
        let monitor = HealthMonitor::new(9999, 5);
        // Port 9999 should be free — expect an error
        let result = monitor.fetch_model_info();
        assert!(result.is_err());
    }
}
