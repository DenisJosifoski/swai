//! Summarization engine for evicted message compaction.
//!
//! When Anthropic Messages API conversations grow beyond the model's context
//! window, older messages are evicted (compacted). This module performs an
//! active LLM call to condense those evicted messages into a concise factual
//! changelog that gets injected back into the conversation as a checkpoint.
//!
//! ## Data flow
//!
//! 1. `compact_messages_anthropic()` drops a slice of evicted messages.
//! 2. `summarize_dropped_slice()` sends the dropped messages to a local LLM
//!    for condensation.
//! 3. If the LLM call fails or times out, falls back to the deterministic
//!    `extract_action_lines()` from `compaction.rs`.
//! 4. The resulting `Vec<String>` is stored in a `SessionCheckpoint` entry.
//! 5. Before the next request, `format_for_injection()` builds the checkpoint
//!    block that gets inserted into the messages array.
//!
//! ## Multi-model routing (Phase 23)
//!
//! The summarizer model is configurable via `PreferencesConfig.checkpoint_summarizer_model`:
//! - `None` (default) → route to the active/primary model port.
//! - `Some(model_id)` → look up that model's port via `ProcessManager.get_port_for_model()`.
//!   If the configured model is not running, fall back to the primary model port.
//!
//! ## Timeout guarantee
//!
//! The HTTP request to the summarizer has a strict 5-second timeout. This ensures
//! compaction never stalls the proxy request pipeline.

use serde_json::Value;

/// Summarization prompt template.
///
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
fn build_summarizer_request(dropped_text: &str, model_id: &str) -> Value {
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

/// Format dropped messages into a readable text block for the summarizer LLM.
///
/// Converts each message in Anthropic format to a plain-text representation that
/// preserves the role, text content, and tool use information.
fn format_messages_for_summarization(messages: &[Value]) -> String {
    let mut lines = Vec::new();

    for msg in messages {
        let role = msg.get("role").and_then(|r| r.as_str()).unwrap_or("unknown");
        let content = msg.get("content");

        match (role, content) {
            ("user", Some(Value::String(text))) if !text.is_empty() => {
                lines.push(format!("[User]: {}", truncate_text(text, 200)));
            }
            ("user", Some(Value::Array(blocks))) => {
                for block in blocks {
                    if block.get("type").and_then(|t| t.as_str()) == Some("tool_result") {
                        let status = if let Some(is_error) = block.get("isError").and_then(|v| v.as_bool()) {
                            if is_error { "FAILED" } else { "passed" }
                        } else {
                            "passed"
                        };
                        lines.push(format!("[Tool Result {}]", status));
                    }
                }
            }
            ("assistant", Some(Value::Array(blocks))) => {
                for block in blocks {
                    match block.get("type").and_then(|t| t.as_str()) {
                        Some("text") => {
                            if let Some(text) = block.get("text").and_then(|t| t.as_str()) {
                                lines.push(format!("[Assistant]: {}", truncate_text(text, 150)));
                            }
                        }
                        Some("tool_use") => {
                            if let Some(name) = block.get("name").and_then(|n| n.as_str()) {
                                match name {
                                    "Read" | "read" | "ViewFile" | "view_file" | "read_file" | "ReadFile" => {
                                        if let Some(input) = block.get("input") {
                                            if let Some(path) = input.get("file_path")
                                                .or_else(|| input.get("path"))
                                                .or_else(|| input.get("file"))
                                                .or_else(|| input.get("AbsolutePath"))
                                                .and_then(|p| p.as_str())
                                            {
                                                lines.push(format!("[Tool: Read {}]", path));
                                            } else {
                                                lines.push("[Tool: Read <unknown>]".to_string());
                                            }
                                        } else {
                                            lines.push("[Tool: Read <unknown>]".to_string());
                                        }
                                    }
                                    "Edit" | "edit" | "ReplaceFileContent" | "replace_file_content" | "multi_replace_file_content" => {
                                        if let Some(input) = block.get("input") {
                                            if let Some(path) = input.get("file_path")
                                                .or_else(|| input.get("path"))
                                                .or_else(|| input.get("TargetFile"))
                                                .or_else(|| input.get("file"))
                                                .and_then(|p| p.as_str())
                                            {
                                                lines.push(format!("[Tool: Edited {}]", path));
                                            } else {
                                                lines.push("[Tool: Edited <unknown>]".to_string());
                                            }
                                        } else {
                                            lines.push("[Tool: Edited <unknown>]".to_string());
                                        }
                                    }
                                    "Write" | "write" | "write_to_file" | "WriteToFile" => {
                                        if let Some(input) = block.get("input") {
                                            if let Some(path) = input.get("file_path")
                                                .or_else(|| input.get("path"))
                                                .or_else(|| input.get("TargetFile"))
                                                .or_else(|| input.get("file"))
                                                .and_then(|p| p.as_str())
                                            {
                                                lines.push(format!("[Tool: Wrote {}]", path));
                                            } else {
                                                lines.push("[Tool: Wrote <unknown>]".to_string());
                                            }
                                        } else {
                                            lines.push("[Tool: Wrote <unknown>]".to_string());
                                        }
                                    }
                                    "Bash" | "bash" | "RunCommand" | "run_command" | "terminal" | "execute_command" => {
                                        if let Some(input) = block.get("input") {
                                            if let Some(cmd) = input.get("command")
                                                .or_else(|| input.get("cmd"))
                                                .or_else(|| input.get("CommandLine"))
                                                .and_then(|c| c.as_str())
                                            {
                                                lines.push(format!("[Tool: Ran command: {}]", truncate_text(cmd, 100)));
                                            } else {
                                                lines.push("[Tool: Ran command: <unknown>]".to_string());
                                            }
                                        } else {
                                            lines.push("[Tool: Ran command: <unknown>]".to_string());
                                        }
                                    }
                                    "Grep" | "grep" | "grep_search" | "Glob" | "glob" => {
                                        if let Some(input) = block.get("input") {
                                            if let Some(pat) = input.get("pattern")
                                                .or_else(|| input.get("query"))
                                                .or_else(|| input.get("Query"))
                                                .and_then(|p| p.as_str())
                                            {
                                                lines.push(format!("[Tool: Searched: {}]", truncate_text(pat, 60)));
                                            } else {
                                                lines.push("[Tool: Searched files]".to_string());
                                            }
                                        } else {
                                            lines.push("[Tool: Searched files]".to_string());
                                        }
                                    }
                                    _ => {
                                        lines.push(format!("[Tool: {}]", truncate_text(name, 50)));
                                    }
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }

    lines.join("\n")
}

/// Truncate a string to the given character count.
fn truncate_text(s: &str, max_len: usize) -> String {
    if s.len() <= max_len {
        s.to_string()
    } else {
        format!("{}…", &s[..max_len])
    }
}

/// Routing configuration for summarizer requests.
#[derive(Debug, Clone)]
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

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ─── parse_summarizer_response tests ────────────────────────────────────

    #[test]
    fn test_parse_response_single_line() {
        let result = parse_summarizer_response("Read src/lib.rs");
        assert_eq!(result, vec!["Read src/lib.rs"]);
    }

    #[test]
    fn test_parse_response_multiple_lines() {
        let input = "Read src/lib.rs\nEdited main.rs\nRan command: cargo build";
        let result = parse_summarizer_response(input);
        assert_eq!(result.len(), 3);
        assert_eq!(result[0], "Read src/lib.rs");
        assert_eq!(result[1], "Edited main.rs");
        assert_eq!(result[2], "Ran command: cargo build");
    }

    #[test]
    fn test_parse_response_strips_bullets() {
        let input = "- Read src/lib.rs\n- Edited main.rs\n- Ran command: cargo build";
        let result = parse_summarizer_response(input);
        assert_eq!(result.len(), 3);
        assert_eq!(result[0], "Read src/lib.rs");
        assert_eq!(result[1], "Edited main.rs");
        assert_eq!(result[2], "Ran command: cargo build");
    }

    #[test]
    fn test_parse_response_strips_asterisks() {
        let input = "* Read src/lib.rs\n* Edited main.rs";
        let result = parse_summarizer_response(input);
        assert_eq!(result, vec!["Read src/lib.rs", "Edited main.rs"]);
    }

    #[test]
    fn test_parse_response_filters_empty_lines() {
        let input = "Read src/lib.rs\n\nEdited main.rs\n\nRan command: cargo build";
        let result = parse_summarizer_response(input);
        assert_eq!(result.len(), 3);
        assert_eq!(result[0], "Read src/lib.rs");
        assert_eq!(result[1], "Edited main.rs");
        assert_eq!(result[2], "Ran command: cargo build");
    }

    #[test]
    fn test_parse_response_filters_code_fences() {
        let input = "```rust\nRead src/lib.rs\n```";
        let result = parse_summarizer_response(input);
        assert_eq!(result, vec!["Read src/lib.rs"]);
    }

    #[test]
    fn test_parse_response_empty_input() {
        let result = parse_summarizer_response("");
        assert!(result.is_empty());
    }

    #[test]
    fn test_parse_response_whitespace_only() {
        let result = parse_summarizer_response("   \n  \n   ");
        assert!(result.is_empty());
    }

    // ─── format_messages_for_summarization tests ────────────────────────────

    #[test]
    fn test_format_user_text_message() {
        let messages = vec![serde_json::json!({
            "role": "user",
            "content": "Hello, can you help me?"
        })];
        let formatted = format_messages_for_summarization(&messages);
        assert!(formatted.contains("[User]: Hello, can you help me?"));
    }

    #[test]
    fn test_format_user_text_truncation() {
        let long_text = "x".repeat(300);
        let messages = vec![serde_json::json!({
            "role": "user",
            "content": long_text
        })];
        let formatted = format_messages_for_summarization(&messages);
        assert!(formatted.contains("…"));
    }

    #[test]
    fn test_format_assistant_tool_use_read() {
        let messages = vec![serde_json::json!({
            "role": "assistant",
            "content": [{
                "type": "tool_use",
                "name": "Read",
                "input": {"file_path": "src/lib.rs"}
            }]
        })];
        let formatted = format_messages_for_summarization(&messages);
        assert!(formatted.contains("[Tool: Read src/lib.rs]"));
    }

    #[test]
    fn test_format_assistant_tool_use_edit() {
        let messages = vec![serde_json::json!({
            "role": "assistant",
            "content": [{
                "type": "tool_use",
                "name": "Edit",
                "input": {"file_path": "src/main.rs"}
            }]
        })];
        let formatted = format_messages_for_summarization(&messages);
        assert!(formatted.contains("[Tool: Edited src/main.rs]"));
    }

    #[test]
    fn test_format_assistant_tool_use_run_command() {
        let messages = vec![serde_json::json!({
            "role": "assistant",
            "content": [{
                "type": "tool_use",
                "name": "RunCommand",
                "input": {"command": "cargo build --release"}
            }]
        })];
        let formatted = format_messages_for_summarization(&messages);
        assert!(formatted.contains("[Tool: Ran command: cargo build --release]"));
    }

    #[test]
    fn test_format_user_tool_result() {
        let messages = vec![serde_json::json!({
            "role": "user",
            "content": [{
                "type": "tool_result",
                "isError": false
            }]
        })];
        let formatted = format_messages_for_summarization(&messages);
        assert!(formatted.contains("[Tool Result passed]"));
    }

    #[test]
    fn test_format_user_tool_result_error() {
        let messages = vec![serde_json::json!({
            "role": "user",
            "content": [{
                "type": "tool_result",
                "isError": true
            }]
        })];
        let formatted = format_messages_for_summarization(&messages);
        assert!(formatted.contains("[Tool Result FAILED]"));
    }

    #[test]
    fn test_format_mixed_messages() {
        let messages = vec![
            serde_json::json!({
                "role": "assistant",
                "content": [{
                    "type": "tool_use",
                    "name": "Read",
                    "input": {"file_path": "src/lib.rs"}
                }]
            }),
            serde_json::json!({
                "role": "user",
                "content": [{"type": "tool_result", "isError": false}]
            }),
            serde_json::json!({
                "role": "assistant",
                "content": [{
                    "type": "text",
                    "text": "I've read the file."
                }]
            }),
        ];
        let formatted = format_messages_for_summarization(&messages);
        assert!(formatted.contains("[Tool: Read src/lib.rs]"));
        assert!(formatted.contains("[Tool Result passed]"));
        assert!(formatted.contains("[Assistant]: I've read the file."));
    }

    #[test]
    fn test_format_empty_messages() {
        let formatted = format_messages_for_summarization(&[]);
        assert!(formatted.is_empty());
    }

    // ─── build_summarizer_request tests ─────────────────────────────────────

    #[test]
    fn test_build_summarizer_request_structure() {
        let req = build_summarizer_request("some text", "test-model");
        assert_eq!(req["model"], "test-model");
        assert_eq!(req["messages"].as_array().unwrap().len(), 2);
        assert_eq!(req["messages"][0]["role"], "system");
        assert_eq!(req["messages"][1]["role"], "user");
        assert_eq!(req["max_tokens"], 500);
        assert_eq!(req["temperature"], 0.0);
    }

    #[test]
    fn test_build_summarizer_request_includes_dropped_text() {
        let req = build_summarizer_request("Read src/lib.rs\nEdit main.rs", "test-model");
        let user_content = req["messages"][1]["content"].as_str().unwrap();
        assert!(user_content.contains("Read src/lib.rs"));
        assert!(user_content.contains("Edit main.rs"));
    }

    // ─── resolve_summarizer_route tests ─────────────────────────────────────

    #[test]
    fn test_resolve_route_preferred_model_running() {
        let running = vec![
            ("primary-model".to_string(), 9081),
            ("secondary-model".to_string(), 9082),
        ];
        let route = resolve_summarizer_route(
            Some("secondary-model"),
            &[("secondary-model", "Secondary")],
            &running,
            Some(9081),
        );
        assert!(route.is_some());
        let r = route.unwrap();
        assert_eq!(r.port, 9082);
        assert_eq!(r.model_id, "secondary-model");
    }

    #[test]
    fn test_resolve_route_preferred_model_not_running_falls_back() {
        let running = vec![("primary-model".to_string(), 9081)];
        let route = resolve_summarizer_route(
            Some("nonexistent-model"),
            &[("nonexistent-model", "Nonexistent")],
            &running,
            Some(9081),
        );
        assert!(route.is_some());
        let r = route.unwrap();
        assert_eq!(r.port, 9081);
        assert_eq!(r.model_id, "primary");
    }

    #[test]
    fn test_resolve_route_none_prefers_primary() {
        let running = vec![("primary-model".to_string(), 9081)];
        let route = resolve_summarizer_route(
            None,
            &[],
            &running,
            Some(9081),
        );
        assert!(route.is_some());
        let r = route.unwrap();
        assert_eq!(r.port, 9081);
    }

    #[test]
    fn test_resolve_route_no_model_available() {
        let route = resolve_summarizer_route(
            None,
            &[],
            &[],
            None,
        );
        assert!(route.is_none());
    }

    // ─── summarize_dropped_slice tests ──────────────────────────────────────

    #[test]
    fn test_summarize_fallback_without_any_model() {
        // No models running → should fall back to deterministic extraction.
        let dropped = vec![
            serde_json::json!({
                "role": "assistant",
                "content": [{
                    "type": "tool_use",
                    "name": "Read",
                    "input": {"file_path": "src/lib.rs"}
                }]
            }),
            serde_json::json!({
                "role": "user",
                "content": "Can you check the config?"
            }),
        ];

        let lines = summarize_dropped_slice(
            &dropped,
            None,
            &[],
            &[],
            None,
        );
        assert!(!lines.is_empty());
        assert!(lines.contains(&"Read src/lib.rs".to_string()));
    }

    #[test]
    fn test_summarize_fallback_with_running_model() {
        // Model running but not configured as summarizer → falls back to primary.
        let dropped = vec![serde_json::json!({
            "role": "user",
            "content": "Hello world"
        })];

        let lines = summarize_dropped_slice(
            &dropped,
            None,
            &[("primary", "Primary Model")],
            &[("primary".to_string(), 9081)],
            Some(9081),
        );
        // The LLM call will fail (no server on port 9081 in tests), so we get fallback.
        assert!(!lines.is_empty());
    }

    // ─── truncate_text tests ────────────────────────────────────────────────

    #[test]
    fn test_truncate_text_short_string() {
        assert_eq!(truncate_text("hello", 10), "hello");
    }

    #[test]
    fn test_truncate_text_long_string() {
        let long = "x".repeat(200);
        let truncated = truncate_text(&long, 50);
        assert!(truncated.ends_with("…"));
        assert_eq!(truncated.chars().count(), 51); // 50 chars + "…"
    }

    #[test]
    fn test_truncate_text_exact_length() {
        let exact = "hello";
        assert_eq!(truncate_text(exact, 5), "hello");
    }
}
