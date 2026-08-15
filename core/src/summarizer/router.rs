use serde_json::Value;

use super::client::call_summarizer;

pub struct SummarizerRoute {
    /// The port to send the summarization request to.
    pub port: u16,
    /// The model id used in the request body.
    pub model_id: String,
}

/// Resolve the summarizer route based on preferences and running models.
///
/// - If `checkpoint_summarizer_model` is set to a configured model id AND that
///   model is currently running (its port is available), return that model's port.
/// - Otherwise, fall back to the primary active model port.
///
/// Returns `None` if no model is available at all.
pub fn resolve_summarizer_route(
    preferred_model: Option<&str>,
    _configured_models: &[(&str, &str)],
    running_ports: &[(String, u16)],
    primary_port: Option<u16>,
) -> Option<SummarizerRoute> {
    // If a specific model is configured, check if it's running.
    if let Some(model_id) = preferred_model {
        // Check if this model is in the configured models list and currently running.
        for (id, port) in running_ports {
            if id == model_id {
                return Some(SummarizerRoute {
                    port: *port,
                    model_id: id.clone(),
                });
            }
        }

        // Model not running — fall through to primary.
        tracing::debug!(
            "configured summarizer model '{}' not running, falling back to primary",
            model_id
        );
    }

    // Fall back to primary model.
    primary_port.map(|port| SummarizerRoute {
        port,
        model_id: "primary".to_string(),
    })
}

/// Summarize a dropped message slice with LLM inference, falling back to
/// deterministic extraction on failure.
///
/// This is the main entry point called during compaction. It resolves the
/// target model via `resolve_summarizer_route`, makes the HTTP request, and
/// returns the parsed summary lines. If the LLM call fails, it falls back
/// to `extract_action_lines` from `compaction.rs`.
pub fn summarize_dropped_slice(
    dropped: &[Value],
    preferred_model: Option<&str>,
    configured_models: &[(&str, &str)],
    running_ports: &[(String, u16)],
    primary_port: Option<u16>,
) -> Vec<String> {
    let route = match resolve_summarizer_route(
        preferred_model,
        configured_models,
        running_ports,
        primary_port,
    ) {
        Some(r) => r,
        None => {
            tracing::warn!("no summarizer model available — using deterministic fallback");
            return extract_action_lines_fallback(dropped);
        }
    };

    // Attempt LLM summarization.
    match call_summarizer(route.port, &route.model_id, dropped) {
        Ok(lines) => {
            tracing::debug!(
                "summarized {} dropped messages via model '{}' on port {}",
                dropped.len(),
                route.model_id,
                route.port
            );
            lines
        }
        Err(e) => {
            tracing::warn!(
                "summarizer LLM call failed (port {}, model '{}'): {}. Falling back to deterministic extraction.",
                route.port,
                route.model_id,
                e
            );
            extract_action_lines_fallback(dropped)
        }
    }
}

/// Deterministic fallback for summarization when the LLM is unavailable.
///
/// Reuses the same `extract_action_lines` logic from `compaction.rs` to produce
/// bullet-point summaries without requiring an LLM call. This ensures compaction
/// always produces useful output even when no model is running.
fn extract_action_lines_fallback(messages: &[Value]) -> Vec<String> {
    use crate::compaction::extract_action_lines;
    extract_action_lines(messages)
}
