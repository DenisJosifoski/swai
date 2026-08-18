//! SWAI — Council route handler and SSE streaming adapter.
//!
//! Provides the proxy execution bridge between incoming client requests
//! targeting synthetic "council" models and the local `CouncilEngine`.

use reqwest::blocking::Client;
use crate::council::pipeline::Executor;
use crate::council::types::{DebateOutcome, PipelineStage};
use crate::council::CouncilPipelineConfig;

/// Check if a model name targets the council engine.
///
/// Matches "council" exactly or any model starting with "council:" prefix.
pub fn is_council_model(model: &str) -> bool {
    model == "council" || model.starts_with("council:")
}

/// Parse an optional X-SWAI-Pipeline header into a CouncilPipelineConfig.
pub fn parse_pipeline_header(header_value: &str) -> Option<CouncilPipelineConfig> {
    serde_json::from_str(header_value).ok()
}

/// Extract the `model` field from a JSON request body.
pub fn extract_model_from_body(body: &[u8]) -> Option<String> {
    let json_val = serde_json::from_slice::<serde_json::Value>(body).ok()?;
    json_val
        .get("model")
        .and_then(|m| m.as_str())
        .map(String::from)
}

/// Proxy executor that forwards council stages to the primary model backend.
pub struct ProxyExecutor {
    pub client: Client,
    pub primary_port: u16,
}

impl Executor for ProxyExecutor {
    fn execute(&self, stage: &PipelineStage, input: &str) -> Result<String, String> {
        let url = format!("http://localhost:{}/v1/chat/completions", self.primary_port);
        let content = if !stage.prompt_template.is_empty() {
            stage.prompt_template.replace("{input}", input)
        } else {
            input.to_string()
        };
        
        let mut messages = Vec::new();
        if let Some(ref sys) = stage.system_prompt {
            if !sys.is_empty() {
                messages.push(serde_json::json!({"role": "system", "content": sys}));
            }
        }
        messages.push(serde_json::json!({"role": "user", "content": content}));

        let body = serde_json::json!({
            "model": stage.model_id,
            "messages": messages,
            "temperature": stage.temperature,
            "top_p": stage.top_p,
        });
        let body_str = serde_json::to_string(&body).unwrap_or_default();
        match self
            .client
            .post(&url)
            .header("Content-Type", "application/json")
            .body(body_str)
            .send()
        {
            Ok(resp) => {
                if resp.status().is_success() {
                    let text = resp.text().unwrap_or_default();
                    let json: serde_json::Value = serde_json::from_str(&text).unwrap_or_default();
                    json["choices"]
                        .as_array()
                        .and_then(|choices| choices.first())
                        .and_then(|choice| choice["message"]["content"].as_str())
                        .map(String::from)
                        .ok_or_else(|| "No response content in choices".into())
                } else {
                    Err(format!("Backend returned status {}", resp.status()))
                }
            }
            Err(e) => Err(format!("Request failed: {}", e)),
        }
    }
}

/// Build SSE events for streaming a council debate outcome.
pub fn build_council_sse_events(outcome: &DebateOutcome, model_id: &str) -> Vec<Vec<u8>> {
    let mut events = Vec::new();
    let mut seq = 1u64;

    // debate.started event.
    events.push(
        format!(
            "event: debate.started\ndata: {{\"type\":\"debate.started\",\"model\":\"{}\"}}\n\n",
            model_id
        )
        .into_bytes(),
    );
    seq += 1;

    // Stream the final response as text deltas (chunked for SSE).
    let final_text = match outcome {
        DebateOutcome::Success { final_response, .. } => final_response.clone(),
        DebateOutcome::Partial {
            fallback_response,
            warnings: _,
            ..
        } => {
            format!("Debate partial: {}", fallback_response)
        }
        DebateOutcome::Aborted { reason, .. } => format!("Debate aborted: {}", reason),
    };

    // Chunk the text into ~50 char segments for realistic streaming.
    let chunk_size = 50;
    let chars: Vec<char> = final_text.chars().collect();
    let mut accumulated = String::new();
    let mut prev_len = 0;

    for (i, ch) in chars.iter().enumerate() {
        accumulated.push(*ch);
        if (i + 1) % chunk_size == 0 || i == chars.len() - 1 {
            let delta = &accumulated[prev_len..];
            let escaped = escape_sse_text(delta);
            events.push(format!(
                "event: text.delta\ndata: {{\"type\":\"text.delta\",\"sequence_number\":{},\"delta\":\"{}\"}}\n\n",
                seq, escaped
            ).into_bytes());
            seq += 1;
            prev_len = i + 1;
        }
    }

    // debate.completed event.
    events.push(
        format!(
            "event: debate.completed\ndata: {{\"type\":\"debate.completed\",\"sequence_number\":{}}}\n\n",
            seq
        )
        .into_bytes(),
    );
    events
}

/// Escape special characters in SSE text data.
pub fn escape_sse_text(text: &str) -> String {
    text.replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
}
