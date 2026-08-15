use super::formatter::format_messages_for_summarization;
use serde_json::Value;

/// Instructs the LLM to produce a concise factual changelog with no conversational
/// prose — only numbered action lines describing files read, edited, and commands run.
const SUMMARIZER_SYSTEM_PROMPT: &str =
    "You are an internal session summarizer for a coding assistant.\n\
     Condense the following dropped conversation history into a concise, factual changelog.\n\
     Format each item as a bullet point:\n\
     - What files were read/viewed\n\
     - What files were edited and a summary of changes\n\
     - What commands were run and their results (pass/fail)\n\
     Do not write conversational prose, narrative, or explanations. Only output numbered factual lines.";

/// Summarization user prompt template.
const SUMMARIZER_USER_PROMPT: &str = "Condense the following dropped conversation history into a concise, factual changelog.\n\n{dropped_text}";

/// HTTP timeout for summarizer requests — must be strictly under 5 seconds to leave
/// room for response processing before the proxy request pipeline stalls.
const SUMMARIZER_TIMEOUT_SECS: u64 = 4;

/// Build an OpenAI-compatible chat completion request payload for the summarizer.
pub fn build_summarizer_request(dropped_text: &str, model_id: &str) -> Value {
    let user_prompt = SUMMARIZER_USER_PROMPT.replace("{dropped_text}", dropped_text);
    serde_json::json!({
        "model": model_id,
        "messages": [
            {"role": "system", "content": SUMMARIZER_SYSTEM_PROMPT},
            {"role": "user", "content": user_prompt}
        ],
        "max_tokens": 500,
        "temperature": 0.0,
    })
}

/// Parse an LLM's text response into summary lines.
///
/// Splits on newlines, strips leading bullet markers (`-`, `*`, `•`), trims whitespace,
/// and filters out empty lines and markdown code fences.
pub fn parse_summarizer_response(text: &str) -> Vec<String> {
    text.lines()
        .map(|line| {
            let trimmed = line.trim();
            // Strip leading bullet markers.
            let cleaned = if trimmed.starts_with('-') || trimmed.starts_with('*') || trimmed.starts_with('•') {
                trimmed[1..].trim()
            } else {
                trimmed
            };
            cleaned.to_string()
        })
        .filter(|line| {
            let trimmed = line.trim();
            // Filter out empty lines, markdown code fences (```, ```rust, etc.),
            // and standalone backtick sequences.
            if trimmed.is_empty() {
                return false;
            }
            if trimmed.starts_with("```") {
                return false;
            }
            true
        })
        .collect()
}

/// Send a summarization request to a local model server.
///
/// Uses the OpenAI-compatible `/v1/chat/completions` endpoint. The response is
/// parsed into summary lines via `parse_summarizer_response`.
///
/// Returns `Ok(Vec<String>)` on success, `Err(String)` on any failure (timeout,
/// network error, parse error). The caller should fall back to the deterministic
/// extractor when this returns an error.
pub fn call_summarizer(
    port: u16,
    model_id: &str,
    dropped_messages: &[Value],
) -> Result<Vec<String>, String> {
    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(SUMMARIZER_TIMEOUT_SECS))
        .build()
        .map_err(|e| format!("failed to build summarizer HTTP client: {}", e))?;

    // Serialize dropped messages into a readable text block for the LLM.
    let dropped_text = format_messages_for_summarization(dropped_messages);
    let request_body = build_summarizer_request(&dropped_text, model_id);
    let body_bytes = serde_json::to_vec(&request_body)
        .map_err(|e| format!("failed to serialize summarizer request: {}", e))?;

    let url = format!("http://127.0.0.1:{}/v1/chat/completions", port);
    let response = client
        .post(&url)
        .header("content-type", "application/json")
        .header("authorization", "Bearer local")
        .body(body_bytes)
        .send()
        .map_err(|e| format!("summarizer request failed: {}", e))?;

    if !response.status().is_success() {
        let status = response.status().as_u16();
        let body = response.text().unwrap_or_default();
        return Err(format!("summarizer returned HTTP {}: {}", status, body));
    }

    let body_bytes = response.bytes()
        .map_err(|e| format!("failed to read summarizer response body: {}", e))?;
    let json: Value = serde_json::from_slice(&body_bytes)
        .map_err(|e| format!("failed to parse summarizer response JSON: {}", e))?;

    // Extract the assistant's text content.
    let text = json
        .get("choices")
        .and_then(|c| c.as_array())
        .and_then(|arr| arr.first())
        .and_then(|choice| choice.get("message"))
        .and_then(|msg| msg.get("content"))
        .and_then(|content| content.as_str())
        .ok_or_else(|| "summarizer response missing choices[0].message.content".to_string())?;

    Ok(parse_summarizer_response(text))
}

