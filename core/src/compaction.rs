//! Message compaction and checkpoint injection for Anthropic Messages API.
//!
//! When conversations grow beyond the model's context window, older messages
//! must be evicted (compacted). This module:
//!
//! 1. Extracts plain-text action summaries from dropped message slices
//!    (e.g., "Read src/lib.rs", "Edited main.rs", "Ran command: cargo build").
//! 2. Provides a deterministic fallback synthesizer (`serialize_dropped_slice`)
//!    that compiles bullet points even if LLM summarization is skipped.
//! 3. Injects formatted `[Session checkpoint]` blocks into Anthropic Messages
//!    API payloads immediately after the system prompt, so the model retains
//!    awareness of earlier work without re-sending full history.
//!
//! ## Data flow
//!
//! ```text
//! messages → compact_messages_anthropic() → [checkpoint entries, remaining messages]
//! checkpoint → format_for_injection() → "[Session checkpoint — ...]" string
//! remaining + checkpoint → inject_checkpoint_into_payload() → modified JSON payload
//! ```

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// A single Anthropic Messages API message.
///
/// The `content` field mirrors the Anthropic API format: either a plain string
/// or an array of content blocks (text, image, tool use, etc.).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub role: String,
    #[serde(default)]
    pub content: Vec<Value>,
}

impl Message {
    /// Extract the first plain-text string from a message's content.
    ///
    /// Anthropic messages can have content as either:
    /// - A plain string: `"Hello world"`
    /// - An array of content blocks: `[{"type": "text", "text": "Hello"}]`
    pub fn first_text(&self) -> Option<String> {
        // Content is a single string value.
        if self.content.len() == 1 {
            if let Some(s) = self.content[0].as_str() {
                return Some(s.to_string());
            }
        }

        // Content is an array — find the first text block.
        for item in &self.content {
            if let Some(text) = item.get("text").and_then(|t| t.as_str()) {
                return Some(text.to_string());
            }
        }

        None
    }

    /// Check if this message contains a tool_use block.
    pub fn has_tool_use(&self) -> bool {
        self.content.iter().any(|item| {
            item.get("type").and_then(|t| t.as_str()) == Some("tool_use")
        })
    }

    /// Extract the tool name from a tool_use block if present.
    pub fn tool_use_name(&self) -> Option<String> {
        for item in &self.content {
            if item.get("type").and_then(|t| t.as_str()) == Some("tool_use") {
                return item.get("name").and_then(|n| n.as_str()).map(String::from);
            }
        }
        None
    }

    /// Check if this message is a tool_result.
    pub fn is_tool_result(&self) -> bool {
        self.role == "user" && self.content.iter().any(|item| {
            item.get("type").and_then(|t| t.as_str()) == Some("tool_result")
        })
    }

    /// Extract the first tool result status (success/failure).
    pub fn tool_result_status(&self) -> Option<String> {
        for item in &self.content {
            if item.get("type").and_then(|t| t.as_str()) == Some("tool_result") {
                // Check for explicit content with isError field.
                if let Some(content) = item.get("content") {
                    if let Some(arr) = content.as_array() {
                        for block in arr {
                            if let Some(is_error) = block.get("isError").and_then(|v| v.as_bool()) {
                                return Some(if is_error { "failed" } else { "passed" }.to_string());
                            }
                        }
                    }
                }
                // If no content block with isError, treat as success.
                return Some("passed".to_string());
            }
        }
        None
    }

    /// Extract the command string from a RunCommand tool_use block if present.
    pub fn run_command_string(&self) -> Option<String> {
        for item in &self.content {
            if item.get("type").and_then(|t| t.as_str()) == Some("tool_use") {
                if let Some(name) = item.get("name").and_then(|n| n.as_str()) {
                    if name == "RunCommand" || name == "run_command" {
                        if let Some(input) = item.get("input") {
                            return input.get("command").and_then(|c| c.as_str()).map(String::from);
                        }
                    }
                }
            }
        }
        None
    }

    /// Extract the file path from a Read/ViewFile tool_use block if present.
    pub fn read_file_path(&self) -> Option<String> {
        for item in &self.content {
            if item.get("type").and_then(|t| t.as_str()) == Some("tool_use") {
                if let Some(name) = item.get("name").and_then(|n| n.as_str()) {
                    match name {
                        "Read" | "view_file" | "ViewFile" => {
                            if let Some(input) = item.get("input") {
                                return input
                                    .get("file_path")
                                    .or_else(|| input.get("path"))
                                    .and_then(|p| p.as_str())
                                    .map(String::from);
                            }
                        }
                        _ => {}
                    }
                }
            }
        }
        None
    }

    /// Extract the file path from an Edit/ReplaceFileContent tool_use block if present.
    pub fn edit_file_path(&self) -> Option<String> {
        for item in &self.content {
            if item.get("type").and_then(|t| t.as_str()) == Some("tool_use") {
                if let Some(name) = item.get("name").and_then(|n| n.as_str()) {
                    match name {
                        "Edit" | "replace_file_content" | "ReplaceFileContent" => {
                            if let Some(input) = item.get("input") {
                                return input
                                    .get("file_path")
                                    .or_else(|| input.get("path"))
                                    .and_then(|p| p.as_str())
                                    .map(String::from);
                            }
                        }
                        _ => {}
                    }
                }
            }
        }
        None
    }
}

/// Configuration for message compaction behavior.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompactionConfig {
    /// Whether compaction is enabled.
    #[serde(default)]
    pub enabled: bool,
    /// Maximum token count before triggering compaction.
    #[serde(default = "default_max_tokens")]
    pub max_tokens: usize,
    /// Target summary length in characters per dropped slice.
    #[serde(default = "default_summary_length")]
    pub summary_length: usize,
}

fn default_max_tokens() -> usize {
    100_000
}

fn default_summary_length() -> usize {
    2_000
}

impl Default for CompactionConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            max_tokens: default_max_tokens(),
            summary_length: default_summary_length(),
        }
    }
}

/// Extract plain-text action lines from a slice of dropped Anthropic messages.
///
/// This is the core summarization step that converts evicted messages into
/// concise text lines the model can understand:
///
/// - `user` message with text → extracted (capped at 200 chars)
/// - `assistant` with `Read`/`ViewFile` tool_use → "Read <file_path>"
/// - `assistant` with `Edit`/`ReplaceFileContent` tool_use → "Edited <file_path>"
/// - `assistant` with `RunCommand`/`run_command` tool_use → "Ran command: <cmd>"
/// - `user` with `tool_result` → "Result: passed" or "Result: failed: <error>"
///
/// Falls back to generic descriptions for unrecognized message patterns.
pub fn extract_action_lines(messages: &[Value]) -> Vec<String> {
    let mut lines = Vec::new();

    for msg in messages {
        let role = msg.get("role").and_then(|r| r.as_str()).unwrap_or("unknown");
        let content = msg.get("content");

        match (role, content) {
            ("user", Some(Value::String(text))) if !text.is_empty() => {
                // Truncate long user messages to 200 chars.
                let truncated: String = text.chars().take(200).collect();
                lines.push(truncated);
            }

            ("user", Some(Value::Array(blocks))) => {
                for block in blocks {
                    if block.get("type").and_then(|t| t.as_str()) == Some("tool_result") {
                        // Check for isError on the block itself.
                        if let Some(is_error) = block.get("isError").and_then(|v| v.as_bool()) {
                            if is_error {
                                lines.push("Result: failed".to_string());
                            } else {
                                lines.push("Result: passed".to_string());
                            }
                            continue;
                        }

                        // Check for isError inside content blocks.
                        if let Some(content) = block.get("content") {
                            if let Some(arr) = content.as_array() {
                                let mut handled = false;
                                for inner in arr {
                                    if let Some(is_error) = inner.get("isError").and_then(|v| v.as_bool()) {
                                        if is_error {
                                            // Extract first line of error text.
                                            if let Some(err_text) = inner.get("text").and_then(|t| t.as_str()) {
                                                let first_line: String = err_text.chars().take(100).collect();
                                                lines.push(format!("Result: failed: {}", first_line));
                                            } else {
                                                lines.push("Result: failed".to_string());
                                            }
                                        } else {
                                            lines.push("Result: passed".to_string());
                                        }
                                        handled = true;
                                        break;
                                    }
                                }
                                if handled {
                                    continue;
                                }
                            }
                        }

                        // No isError field found → assume success.
                        lines.push("Result: passed".to_string());
                    }
                }
            }

            ("assistant", Some(Value::Array(blocks))) => {
                let mut has_tool_use = false;
                for block in blocks {
                    if block.get("type").and_then(|t| t.as_str()) == Some("tool_use") {
                        has_tool_use = true;
                        if let Some(name) = block.get("name").and_then(|n| n.as_str()) {
                            match name {
                                "Read" | "read" | "ViewFile" | "view_file" | "read_file" | "ReadFile" => {
                                    if let Some(input) = block.get("input") {
                                        if let Some(path) = input
                                            .get("file_path")
                                            .or_else(|| input.get("path"))
                                            .or_else(|| input.get("file"))
                                            .or_else(|| input.get("AbsolutePath"))
                                            .and_then(|p| p.as_str())
                                        {
                                            lines.push(format!("Read {}", path));
                                        } else {
                                            lines.push("Read <unknown file>".to_string());
                                        }
                                    } else {
                                        lines.push("Read <unknown file>".to_string());
                                    }
                                }
                                "Edit" | "edit" | "ReplaceFileContent" | "replace_file_content" | "multi_replace_file_content" => {
                                    if let Some(input) = block.get("input") {
                                        if let Some(path) = input
                                            .get("file_path")
                                            .or_else(|| input.get("path"))
                                            .or_else(|| input.get("TargetFile"))
                                            .or_else(|| input.get("file"))
                                            .and_then(|p| p.as_str())
                                        {
                                            lines.push(format!("Edited {}", path));
                                        } else {
                                            lines.push(
                                                "Edited <unknown file>".to_string(),
                                            );
                                        }
                                    } else {
                                        lines.push("Edited <unknown file>".to_string());
                                    }
                                }
                                "Write" | "write" | "write_to_file" | "WriteToFile" => {
                                    if let Some(input) = block.get("input") {
                                        if let Some(path) = input
                                            .get("file_path")
                                            .or_else(|| input.get("path"))
                                            .or_else(|| input.get("TargetFile"))
                                            .or_else(|| input.get("file"))
                                            .and_then(|p| p.as_str())
                                        {
                                            lines.push(format!("Wrote {}", path));
                                        } else {
                                            lines.push("Wrote <unknown file>".to_string());
                                        }
                                    } else {
                                        lines.push("Wrote <unknown file>".to_string());
                                    }
                                }
                                "Bash" | "bash" | "RunCommand" | "run_command" | "terminal" | "execute_command" => {
                                    if let Some(input) = block.get("input") {
                                        if let Some(cmd) = input
                                            .get("command")
                                            .or_else(|| input.get("cmd"))
                                            .or_else(|| input.get("CommandLine"))
                                            .and_then(|c| c.as_str())
                                        {
                                            let truncated: String = cmd.chars().take(100).collect();
                                            lines.push(format!("Ran command: {}", truncated));
                                        } else {
                                            lines.push("Ran command: <unknown>".to_string());
                                        }
                                    } else {
                                        lines.push("Ran command: <unknown>".to_string());
                                    }
                                }
                                "Grep" | "grep" | "grep_search" | "Glob" | "glob" => {
                                    if let Some(input) = block.get("input") {
                                        if let Some(pat) = input
                                            .get("pattern")
                                            .or_else(|| input.get("query"))
                                            .or_else(|| input.get("Query"))
                                            .and_then(|p| p.as_str())
                                        {
                                            let truncated: String = pat.chars().take(60).collect();
                                            lines.push(format!("Searched: {}", truncated));
                                        } else {
                                            lines.push("Searched files".to_string());
                                        }
                                    } else {
                                        lines.push("Searched files".to_string());
                                    }
                                }
                                _ => {
                                    // Unknown tool — record generically.
                                    let truncated: String = name.chars().take(50).collect();
                                    lines.push(format!("Used tool: {}", truncated));
                                }
                            }
                        }
                    } else if block.get("type").and_then(|t| t.as_str()) == Some("text") {
                        // Assistant text response (non-tool).
                        if let Some(text) = block.get("text").and_then(|t| t.as_str()) {
                            if !text.is_empty() {
                                let truncated: String = text.chars().take(150).collect();
                                lines.push(format!("Responded: {}", truncated));
                            }
                        }
                    }
                }
                // If the assistant had tool_use but no text, add a generic line.
                if has_tool_use && lines.is_empty() {
                    lines.push("Used tools".to_string());
                }
            }

            _ => {
                // Unrecognized pattern — skip silently.
            }
        }
    }

    lines
}

/// Deterministic fallback synthesizer for dropped message slices.
///
/// Provides a zero-inference baseline that compiles bullet points even if
/// LLM summarization is skipped or fails. This ensures compaction always
/// produces useful output, regardless of whether a summarization LLM is
/// available.
///
/// Group messages into atomic eviction units: either a single non-tool turn,
/// or an `(assistant tool_use, user tool_result)` pair that must always be
/// dropped or kept together to preserve structural protocol validity.
pub fn build_eviction_units(messages: &[Value]) -> Vec<(usize, usize)> {
    let mut units = Vec::new();
    let mut i = 0;
    while i < messages.len() {
        let has_tool_use = messages[i]
            .get("content")
            .and_then(|c| c.as_array())
            .map(|arr| arr.iter().any(|b| b.get("type").and_then(|t| t.as_str()) == Some("tool_use")))
            .unwrap_or(false);

        if has_tool_use && i + 1 < messages.len() {
            let next_has_tool_result = messages[i + 1]
                .get("content")
                .and_then(|c| c.as_array())
                .map(|arr| arr.iter().any(|b| b.get("type").and_then(|t| t.as_str()) == Some("tool_result")))
                .unwrap_or(false);
            if next_has_tool_result {
                units.push((i, i + 1));
                i += 2;
                continue;
            }
        }
        units.push((i, i));
        i += 1;
    }
    units
}

/// Deterministic fallback synthesizer for dropped message slices.
pub fn serialize_dropped_slice(dropped: &[Value]) -> Vec<String> {
    extract_action_lines(dropped)
}

/// Compact Anthropic Messages API messages by dropping the oldest slice.
///
/// Dynamically evicts older messages until total payload size fits safely within
/// the context budget (targeting ~40k tokens in a 64k window), while truncating
/// oversized individual tool results so no single multi-file read can crash the server.
///
/// Returns a tuple of:
/// - `(summary_lines, remaining_messages)` where `summary_lines` is the
///   extracted action text from dropped messages, and `remaining_messages`
///   is the JSON payload with the oldest messages removed.
pub fn compact_messages_anthropic(
    messages: &[Value],
    config: &CompactionConfig,
) -> (Vec<String>, Vec<Value>) {
    if !config.enabled || messages.is_empty() {
        return (Vec::new(), messages.to_vec());
    }

    // ~40k tokens budget (leaves 24k headroom for system prompt, tools & completion)
    let max_budget_chars = 140_000;

    let msg_len = |m: &Value| -> usize {
        serde_json::to_string(m).map(|s| s.len()).unwrap_or(0)
    };

    let mut total_chars: usize = messages.iter().map(msg_len).sum();

    // Collect all files that were edited anywhere in the conversation
    let mut edited_files = std::collections::HashSet::new();
    for msg in messages {
        if let Some(arr) = msg.get("content").and_then(|c| c.as_array()) {
            for block in arr {
                if block.get("type").and_then(|t| t.as_str()) == Some("tool_use") {
                    if let Some(name) = block.get("name").and_then(|n| n.as_str()) {
                        match name {
                            "Edit" | "edit" | "ReplaceFileContent" | "replace_file_content" | "multi_replace_file_content" | "Write" | "write" | "write_to_file" | "WriteToFile" => {
                                if let Some(input) = block.get("input") {
                                    if let Some(path) = input.get("file_path").or_else(|| input.get("path")).or_else(|| input.get("TargetFile")).and_then(|p| p.as_str()) {
                                        edited_files.insert(path.to_string());
                                    }
                                }
                            }
                            _ => {}
                        }
                    }
                }
            }
        }
    }

    // Helper: checks if a message is associated with an edited file
    let is_edited_file_msg = |m: &Value| -> bool {
        if let Some(arr) = m.get("content").and_then(|c| c.as_array()) {
            for block in arr {
                if block.get("type").and_then(|t| t.as_str()) == Some("tool_use") {
                    if let Some(input) = block.get("input") {
                        if let Some(path) = input.get("file_path").or_else(|| input.get("path")).or_else(|| input.get("TargetFile")).and_then(|p| p.as_str()) {
                            if edited_files.contains(path) {
                                return true;
                            }
                        }
                    }
                }
            }
        }
        false
    };

    let units = build_eviction_units(messages);
    let mut dropped_indices = std::collections::HashSet::new();

    // Helper: checks if a unit contains an edited file tool_use
    let is_edited_unit = |unit: &(usize, usize)| -> bool {
        for idx in unit.0..=unit.1 {
            if is_edited_file_msg(&messages[idx]) {
                return true;
            }
        }
        false
    };

    // Preserve unit 0 (the original user prompt/goal) if messages[0] is a user text message
    // and we have more than 2 units to evict from.
    let is_initial_user_prompt = messages.first().map(|m| {
        m.get("role").and_then(|r| r.as_str()) == Some("user")
            && !m.get("content").and_then(|c| c.as_array()).map(|arr| {
                arr.iter().any(|b| b.get("type").and_then(|t| t.as_str()) == Some("tool_result"))
            }).unwrap_or(false)
    }).unwrap_or(false);

    let start_u_idx = if is_initial_user_prompt && units.len() > 2 { 1 } else { 0 };
    let max_droppable_units = if units.len() > 1 { units.len() - 1 } else { 0 };

    // Pass 1: Evict unedited file units and general message units first
    if total_chars > max_budget_chars {
        for u_idx in start_u_idx..max_droppable_units {
            if total_chars <= max_budget_chars {
                break;
            }
            let unit = &units[u_idx];
            if !is_edited_unit(unit) {
                for idx in unit.0..=unit.1 {
                    dropped_indices.insert(idx);
                    total_chars = total_chars.saturating_sub(msg_len(&messages[idx]));
                }
            }
        }
    }

    // Pass 2: Fallback to oldest-first units if budget is still exceeded
    if total_chars > max_budget_chars {
        for u_idx in start_u_idx..max_droppable_units {
            if total_chars <= max_budget_chars {
                break;
            }
            let unit = &units[u_idx];
            if !dropped_indices.contains(&unit.0) {
                for idx in unit.0..=unit.1 {
                    dropped_indices.insert(idx);
                    total_chars = total_chars.saturating_sub(msg_len(&messages[idx]));
                }
            }
        }
    }

    // If message count alone is high and nothing was dropped, drop first half of intermediate units
    if dropped_indices.is_empty() && messages.len() >= 10 && units.len() >= 2 {
        let half_count = (units.len() / 2).min(max_droppable_units.saturating_sub(start_u_idx));
        for u_idx in start_u_idx..(start_u_idx + half_count) {
            let unit = &units[u_idx];
            for idx in unit.0..=unit.1 {
                dropped_indices.insert(idx);
            }
        }
    }

    let mut dropped = Vec::new();
    let mut remaining = Vec::new();
    for (idx, msg) in messages.iter().enumerate() {
        if dropped_indices.contains(&idx) {
            dropped.push(msg.clone());
        } else {
            remaining.push(msg.clone());
        }
    }

    let summary_lines = if !dropped.is_empty() {
        serialize_dropped_slice(&dropped)
    } else {
        Vec::new()
    };

    // Step 2: In remaining messages, truncate any tool_result (string or array of text blocks) that exceeds 12,000 chars (~3.5k tokens)
    for msg in &mut remaining {
        if let Some(arr) = msg.get_mut("content").and_then(|c| c.as_array_mut()) {
            for block in arr {
                if block.get("type").and_then(|t| t.as_str()) == Some("tool_result") {
                    if let Some(content_val) = block.get_mut("content") {
                        match content_val {
                            Value::String(s) => {
                                if s.len() > 12_000 {
                                    *s = truncate_head_tail(s, 12_000);
                                }
                            }
                            Value::Array(blocks) => {
                                for inner in blocks.iter_mut() {
                                    if let Some(text) = inner.get("text").and_then(|t| t.as_str()) {
                                        if text.len() > 12_000 {
                                            let truncated = truncate_head_tail(text, 12_000);
                                            if let Some(obj) = inner.as_object_mut() {
                                                obj.insert("text".to_string(), Value::String(truncated));
                                            }
                                        }
                                    }
                                }
                            }
                            _ => {}
                        }
                    }
                }
            }
        }
    }

    (summary_lines, remaining)
}

/// Truncate a long text string preserving the head and tail, with a center indicator.
fn truncate_head_tail(s: &str, limit: usize) -> String {
    if s.len() <= limit {
        s.to_string()
    } else {
        let head_len = limit / 2;
        let tail_len = limit / 4;
        let head: String = s.chars().take(head_len).collect();
        let tail: String = s.chars().skip(s.chars().count().saturating_sub(tail_len)).collect();
        let omitted = s.len().saturating_sub(head.len() + tail.len());
        format!("{}\n\n[... truncated {} characters for context budget ...]\n\n{}", head, omitted, tail)
    }
}

/// Inject a checkpoint block into an Anthropic Messages API JSON payload.
///
/// Inserts a synthetic `user` message at index 0 (or immediately after the
/// first system message) containing the formatted checkpoint text. This gives
/// the model context about earlier work that was compacted out of the
/// conversation history.
///
/// Returns the modified payload, or the original if no checkpoint is available.
pub fn inject_checkpoint_into_payload(
    payload: &mut Value,
    checkpoint_text: &str,
) -> bool {
    // Find the "messages" array in the payload.
    let messages = match payload.get_mut("messages") {
        Some(Value::Array(messages)) => messages,
        _ => return false,
    };

    if messages.is_empty() {
        return false;
    }

    // Build the checkpoint message.
    let checkpoint_msg = serde_json::json!({
        "role": "user",
        "content": checkpoint_text,
    });

    // Find the last system message index to insert after.
    let insert_at = messages
        .iter()
        .rev()
        .position(|m| {
            m.get("role").and_then(|r| r.as_str()) == Some("system")
        })
        .map(|rev_pos| messages.len() - rev_pos)
        .unwrap_or(0);

    // Insert the checkpoint message at the appropriate position.
    messages.insert(insert_at, checkpoint_msg);

    true
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ─── Message struct tests ───────────────────────────────────────────────

    #[test]
    fn test_message_serde_roundtrip() {
        let msg = Message {
            role: "user".to_string(),
            content: vec![Value::String("Hello".to_string())],
        };

        let serialized = serde_json::to_string(&msg).unwrap();
        let deserialized: Message = serde_json::from_str(&serialized).unwrap();

        assert_eq!(deserialized.role, "user");
        assert_eq!(deserialized.content.len(), 1);
    }

    #[test]
    fn test_message_from_anthropic_format() {
        // Anthropic API format: {"role": "assistant", "content": [{"type": "text", "text": "..."}]}
        let obj = serde_json::json!({
            "role": "assistant",
            "content": [{"type": "text", "text": "Hello world"}]
        });

        let msg: Message = serde_json::from_value(obj).unwrap();
        assert_eq!(msg.role, "assistant");
    }

    #[test]
    fn test_message_first_text_string_content() {
        let msg = Message {
            role: "user".to_string(),
            content: vec![Value::String("Hello world".to_string())],
        };
        assert_eq!(msg.first_text(), Some("Hello world".to_string()));
    }

    #[test]
    fn test_message_first_text_array_content() {
        let msg = Message {
            role: "assistant".to_string(),
            content: vec![
                serde_json::json!({"type": "text", "text": "Hello"}),
                serde_json::json!({"type": "text", "text": "World"}),
            ],
        };
        assert_eq!(msg.first_text(), Some("Hello".to_string()));
    }

    #[test]
    fn test_message_first_text_empty() {
        let msg = Message {
            role: "user".to_string(),
            content: vec![],
        };
        assert_eq!(msg.first_text(), None);
    }

    #[test]
    fn test_message_has_tool_use() {
        let msg_with_tool = Message {
            role: "assistant".to_string(),
            content: vec![
                serde_json::json!({"type": "text", "text": "Let me check"}),
                serde_json::json!({
                    "type": "tool_use",
                    "name": "Read",
                    "input": {"file_path": "src/main.rs"}
                }),
            ],
        };
        assert!(msg_with_tool.has_tool_use());

        let msg_no_tool = Message {
            role: "assistant".to_string(),
            content: vec![serde_json::json!({"type": "text", "text": "Hello"})],
        };
        assert!(!msg_no_tool.has_tool_use());
    }

    #[test]
    fn test_message_tool_use_name() {
        let msg = Message {
            role: "assistant".to_string(),
            content: vec![serde_json::json!({
                "type": "tool_use",
                "name": "Read",
                "input": {"file_path": "src/main.rs"}
            })],
        };
        assert_eq!(msg.tool_use_name(), Some("Read".to_string()));
    }

    #[test]
    fn test_message_is_tool_result() {
        let msg = Message {
            role: "user".to_string(),
            content: vec![serde_json::json!({
                "type": "tool_result",
                "content": [{"type": "text", "text": "output"}]
            })],
        };
        assert!(msg.is_tool_result());

        let msg_user_text = Message {
            role: "user".to_string(),
            content: vec![Value::String("Hello".to_string())],
        };
        assert!(!msg_user_text.is_tool_result());
    }

    #[test]
    fn test_message_read_file_path() {
        let msg = Message {
            role: "assistant".to_string(),
            content: vec![serde_json::json!({
                "type": "tool_use",
                "name": "Read",
                "input": {"file_path": "src/main.rs"}
            })],
        };
        assert_eq!(msg.read_file_path(), Some("src/main.rs".to_string()));
    }

    #[test]
    fn test_message_edit_file_path() {
        let msg = Message {
            role: "assistant".to_string(),
            content: vec![serde_json::json!({
                "type": "tool_use",
                "name": "Edit",
                "input": {"file_path": "src/main.rs"}
            })],
        };
        assert_eq!(msg.edit_file_path(), Some("src/main.rs".to_string()));
    }

    #[test]
    fn test_message_run_command_string() {
        let msg = Message {
            role: "assistant".to_string(),
            content: vec![serde_json::json!({
                "type": "tool_use",
                "name": "RunCommand",
                "input": {"command": "cargo build"}
            })],
        };
        assert_eq!(msg.run_command_string(), Some("cargo build".to_string()));
    }

    // ─── CompactionConfig tests ─────────────────────────────────────────────

    #[test]
    fn test_compaction_config_defaults() {
        let cfg = CompactionConfig::default();
        assert!(cfg.enabled);
        assert!(cfg.max_tokens > 0);
        assert!(cfg.summary_length > 0);
    }

    #[test]
    fn test_compaction_config_from_toml() {
        let toml_str = r#"
            enabled = true
            max_tokens = 50000
            summary_length = 2000
        "#;

        let cfg: CompactionConfig = toml::from_str(toml_str).unwrap();
        assert!(cfg.enabled);
        assert_eq!(cfg.max_tokens, 50_000);
        assert_eq!(cfg.summary_length, 2000);
    }

    #[test]
    fn test_compaction_config_from_toml_invalid() {
        let result: Result<CompactionConfig, _> = toml::from_str("not valid toml [[[");
        assert!(result.is_err());
    }

    // ─── extract_action_lines tests ─────────────────────────────────────────

    #[test]
    fn test_extract_action_lines_user_text() {
        let messages = vec![serde_json::json!({
            "role": "user",
            "content": "Hello, can you help me with this code?"
        })];

        let lines = extract_action_lines(&messages);
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0], "Hello, can you help me with this code?");
    }

    #[test]
    fn test_extract_action_lines_user_text_truncation() {
        let long_text = "x".repeat(300);
        let messages = vec![serde_json::json!({
            "role": "user",
            "content": long_text
        })];

        let lines = extract_action_lines(&messages);
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].len(), 200);
    }

    #[test]
    fn test_extract_action_lines_read_tool() {
        let messages = vec![serde_json::json!({
            "role": "assistant",
            "content": [{
                "type": "text",
                "text": "Let me read the file."
            }, {
                "type": "tool_use",
                "name": "Read",
                "input": {"file_path": "src/lib.rs"}
            }]
        })];

        let lines = extract_action_lines(&messages);
        assert!(lines.contains(&"Read src/lib.rs".to_string()));
    }

    #[test]
    fn test_extract_action_lines_edit_tool() {
        let messages = vec![serde_json::json!({
            "role": "assistant",
            "content": [{
                "type": "tool_use",
                "name": "Edit",
                "input": {"file_path": "src/main.rs"}
            }]
        })];

        let lines = extract_action_lines(&messages);
        assert!(lines.contains(&"Edited src/main.rs".to_string()));
    }

    #[test]
    fn test_extract_action_lines_run_command_tool() {
        let messages = vec![serde_json::json!({
            "role": "assistant",
            "content": [{
                "type": "tool_use",
                "name": "RunCommand",
                "input": {"command": "cargo build --release"}
            }]
        })];

        let lines = extract_action_lines(&messages);
        assert!(lines.contains(&"Ran command: cargo build --release".to_string()));
    }

    #[test]
    fn test_extract_action_lines_tool_result_passed() {
        let messages = vec![serde_json::json!({
            "role": "user",
            "content": [{
                "type": "tool_result",
                "content": [{"type": "text", "text": "Build succeeded"}]
            }]
        })];

        let lines = extract_action_lines(&messages);
        assert!(lines.contains(&"Result: passed".to_string()));
    }

    #[test]
    fn test_extract_action_lines_tool_result_failed() {
        let messages = vec![serde_json::json!({
            "role": "user",
            "content": [{
                "type": "tool_result",
                "content": [
                    {"type": "text", "text": "error[E0308]: mismatched types", "isError": true}
                ]
            }]
        })];

        let lines = extract_action_lines(&messages);
        assert!(lines.iter().any(|l| l.starts_with("Result: failed")));
    }

    #[test]
    fn test_extract_action_lines_empty_input() {
        let lines = extract_action_lines(&[]);
        assert!(lines.is_empty());
    }

    #[test]
    fn test_extract_action_lines_mixed_messages() {
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
                "content": [{"type": "tool_result", "content": [{"type": "text", "text": "ok"}]}]
            }),
            serde_json::json!({
                "role": "assistant",
                "content": [{
                    "type": "tool_use",
                    "name": "Edit",
                    "input": {"file_path": "src/main.rs"}
                }]
            }),
        ];

        let lines = extract_action_lines(&messages);
        assert_eq!(lines.len(), 3);
        assert_eq!(lines[0], "Read src/lib.rs");
        assert_eq!(lines[1], "Result: passed");
        assert_eq!(lines[2], "Edited src/main.rs");
    }

    // ─── serialize_dropped_slice tests ──────────────────────────────────────

    #[test]
    fn test_serialize_dropped_slice_produces_lines() {
        let dropped = vec![
            serde_json::json!({
                "role": "assistant",
                "content": [{
                    "type": "tool_use",
                    "name": "Read",
                    "input": {"file_path": "src/config.rs"}
                }]
            }),
            serde_json::json!({
                "role": "user",
                "content": "Can you check the config?"
            }),
        ];

        let lines = serialize_dropped_slice(&dropped);
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0], "Read src/config.rs");
        assert_eq!(lines[1], "Can you check the config?");
    }

    #[test]
    fn test_serialize_dropped_slice_empty() {
        let lines = serialize_dropped_slice(&[]);
        assert!(lines.is_empty());
    }

    // ─── compact_messages_anthropic tests ───────────────────────────────────

    #[test]
    fn test_compact_disabled_returns_unchanged() {
        let messages = vec![
            serde_json::json!({"role": "user", "content": "msg1"}),
            serde_json::json!({"role": "assistant", "content": "msg2"}),
            serde_json::json!({"role": "user", "content": "msg3"}),
        ];

        let config = CompactionConfig {
            enabled: false,
            max_tokens: 100,
            summary_length: 100,
        };

        let (summary, remaining) = compact_messages_anthropic(&messages, &config);
        assert!(summary.is_empty());
        assert_eq!(remaining.len(), 3);
    }

    #[test]
    fn test_compact_enabled_drops_half() {
        let messages: Vec<Value> = (0..10)
            .map(|i| serde_json::json!({"role": "user", "content": format!("msg{}", i)}))
            .collect();

        let config = CompactionConfig {
            enabled: true,
            max_tokens: 100,
            summary_length: 100,
        };

        let (summary, remaining) = compact_messages_anthropic(&messages, &config);
        assert_eq!(remaining.len(), 5); // Dropped first 5
        assert!(!summary.is_empty()); // Should have extracted some action lines
    }

    #[test]
    fn test_compact_empty_messages() {
        let messages: Vec<Value> = vec![];
        let config = CompactionConfig {
            enabled: true,
            max_tokens: 100,
            summary_length: 100,
        };

        let (summary, remaining) = compact_messages_anthropic(&messages, &config);
        assert!(summary.is_empty());
        assert!(remaining.is_empty());
    }

    // ─── inject_checkpoint_into_payload tests ───────────────────────────────

    #[test]
    fn test_inject_after_system_message() {
        let mut payload = serde_json::json!({
            "messages": [
                {"role": "system", "content": "You are a helpful assistant."},
                {"role": "user", "content": "Hello!"}
            ]
        });

        let checkpoint_text = "[Session checkpoint — earlier work\n1. Read src/lib.rs\n[End checkpoint]";

        let injected = inject_checkpoint_into_payload(&mut payload, checkpoint_text);
        assert!(injected);

        let messages = payload["messages"].as_array().unwrap();
        assert_eq!(messages.len(), 3);
        assert_eq!(messages[0]["role"], "system");
        assert_eq!(messages[1]["role"], "user"); // checkpoint inserted after system
        assert_eq!(messages[1]["content"], checkpoint_text);
        assert_eq!(messages[2]["role"], "user"); // original user message shifted
    }

    #[test]
    fn test_inject_at_start_when_no_system() {
        let mut payload = serde_json::json!({
            "messages": [
                {"role": "user", "content": "Hello!"}
            ]
        });

        let checkpoint_text = "[Session checkpoint — earlier work\n[End checkpoint]";

        let injected = inject_checkpoint_into_payload(&mut payload, checkpoint_text);
        assert!(injected);

        let messages = payload["messages"].as_array().unwrap();
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0]["role"], "user"); // checkpoint at index 0
        assert_eq!(messages[0]["content"], checkpoint_text);
    }

    #[test]
    fn test_inject_empty_messages_returns_false() {
        let mut payload = serde_json::json!({
            "messages": []
        });

        let injected = inject_checkpoint_into_payload(&mut payload, "checkpoint text");
        assert!(!injected);
    }

    #[test]
    fn test_inject_missing_messages_field_returns_false() {
        let mut payload = serde_json::json!({});

        let injected = inject_checkpoint_into_payload(&mut payload, "checkpoint text");
        assert!(!injected);
    }

    #[test]
    fn test_inject_preserves_original_order() {
        let mut payload = serde_json::json!({
            "messages": [
                {"role": "system", "content": "System prompt"},
                {"role": "user", "content": "First user message"},
                {"role": "assistant", "content": "First assistant response"},
                {"role": "user", "content": "Second user message"},
            ]
        });

        let checkpoint_text = "[Checkpoint]\n[End checkpoint]";

        inject_checkpoint_into_payload(&mut payload, checkpoint_text);

        let messages = payload["messages"].as_array().unwrap();
        assert_eq!(messages.len(), 5); // original 4 + 1 checkpoint
        assert_eq!(messages[0]["role"], "system");
        assert_eq!(messages[1]["content"], checkpoint_text);
        assert_eq!(messages[2]["content"], "First user message");
        assert_eq!(messages[3]["content"], "First assistant response");
        assert_eq!(messages[4]["content"], "Second user message");
    }

    #[test]
    fn test_inject_multiple_system_messages() {
        let mut payload = serde_json::json!({
            "messages": [
                {"role": "system", "content": "System 1"},
                {"role": "system", "content": "System 2"},
                {"role": "user", "content": "User message"},
            ]
        });

        let checkpoint_text = "[Checkpoint]\n[End checkpoint]";

        inject_checkpoint_into_payload(&mut payload, checkpoint_text);

        let messages = payload["messages"].as_array().unwrap();
        assert_eq!(messages.len(), 4);
        // Should be inserted after the LAST system message (index 1 + 1 = 2).
        assert_eq!(messages[0]["role"], "system");
        assert_eq!(messages[1]["role"], "system");
        assert_eq!(messages[2]["content"], checkpoint_text);
        assert_eq!(messages[3]["role"], "user");
    }

    #[test]
    fn test_eviction_prefers_dropping_unedited_files() {
        // Create 4 messages:
        // 0: Read foo.rs (never edited) — 80k chars
        // 1: Read bar.rs (will be edited) — 80k chars
        // 2: Edit bar.rs — 10k chars
        // 3: Assistant reply — 10k chars
        // Total chars > 140k. Only 1 message needs to be dropped to get under 140k.
        let big_content = "x".repeat(80_000);
        let messages = vec![
            serde_json::json!({
                "role": "assistant",
                "content": [{
                    "type": "tool_use",
                    "id": "1",
                    "name": "Read",
                    "input": {"file_path": "foo.rs"}
                }, {
                    "type": "text",
                    "text": big_content
                }]
            }),
            serde_json::json!({
                "role": "assistant",
                "content": [{
                    "type": "tool_use",
                    "id": "2",
                    "name": "Read",
                    "input": {"file_path": "bar.rs"}
                }, {
                    "type": "text",
                    "text": big_content
                }]
            }),
            serde_json::json!({
                "role": "assistant",
                "content": [{
                    "type": "tool_use",
                    "id": "3",
                    "name": "Edit",
                    "input": {"file_path": "bar.rs"}
                }]
            }),
            serde_json::json!({
                "role": "assistant",
                "content": "Finished editing bar.rs"
            }),
        ];

        let config = CompactionConfig::default();
        let (summary, remaining) = compact_messages_anthropic(&messages, &config);

        // foo.rs (unedited) should be dropped in summary, bar.rs should be kept in remaining!
        assert!(summary.iter().any(|s| s.contains("Read foo.rs")));
        assert!(!summary.iter().any(|s| s.contains("bar.rs")));
        assert_eq!(remaining.len(), 3);
        assert!(remaining.iter().any(|m| m.to_string().contains("bar.rs")));
    }

    #[test]
    fn test_eviction_falls_back_when_all_files_edited() {
        // Both foo.rs and bar.rs are edited, but total exceeds 140k.
        let big_content = "x".repeat(80_000);
        let messages = vec![
            serde_json::json!({
                "role": "assistant",
                "content": [{
                    "type": "tool_use",
                    "id": "1",
                    "name": "Read",
                    "input": {"file_path": "foo.rs"}
                }, {
                    "type": "text",
                    "text": big_content
                }]
            }),
            serde_json::json!({
                "role": "assistant",
                "content": [{
                    "type": "tool_use",
                    "id": "2",
                    "name": "Read",
                    "input": {"file_path": "bar.rs"}
                }, {
                    "type": "text",
                    "text": big_content
                }]
            }),
            serde_json::json!({
                "role": "assistant",
                "content": [{
                    "type": "tool_use",
                    "id": "3",
                    "name": "Edit",
                    "input": {"file_path": "foo.rs"}
                }]
            }),
            serde_json::json!({
                "role": "assistant",
                "content": [{
                    "type": "tool_use",
                    "id": "4",
                    "name": "Edit",
                    "input": {"file_path": "bar.rs"}
                }]
            }),
        ];

        let config = CompactionConfig::default();
        let (summary, remaining) = compact_messages_anthropic(&messages, &config);

        // Fallback to oldest-first should successfully drop message 0 to meet budget
        assert!(!summary.is_empty());
        assert_eq!(remaining.len(), 3);
    }

    #[test]
    fn test_eviction_preserves_tool_use_and_tool_result_pairing() {
        let big_content = "y".repeat(150_000);
        let messages = vec![
            // Pair 1: Read foo.rs (unedited)
            serde_json::json!({
                "role": "assistant",
                "content": [{
                    "type": "tool_use",
                    "id": "tool_1",
                    "name": "Read",
                    "input": {"file_path": "foo.rs"}
                }]
            }),
            serde_json::json!({
                "role": "user",
                "content": [{
                    "type": "tool_result",
                    "tool_use_id": "tool_1",
                    "content": big_content
                }]
            }),
            // Pair 2: Read bar.rs (edited later)
            serde_json::json!({
                "role": "assistant",
                "content": [{
                    "type": "tool_use",
                    "id": "tool_2",
                    "name": "Read",
                    "input": {"file_path": "bar.rs"}
                }]
            }),
            serde_json::json!({
                "role": "user",
                "content": [{
                    "type": "tool_result",
                    "tool_use_id": "tool_2",
                    "content": "bar file content"
                }]
            }),
            // Pair 3: Edit bar.rs
            serde_json::json!({
                "role": "assistant",
                "content": [{
                    "type": "tool_use",
                    "id": "tool_3",
                    "name": "Edit",
                    "input": {"file_path": "bar.rs"}
                }]
            }),
            serde_json::json!({
                "role": "user",
                "content": [{
                    "type": "tool_result",
                    "tool_use_id": "tool_3",
                    "content": "Edit succeeded"
                }]
            }),
        ];

        let config = CompactionConfig::default();
        let (summary, remaining) = compact_messages_anthropic(&messages, &config);

        // Pair 1 (foo.rs) should be dropped as an atomic unit
        assert!(!summary.is_empty());
        assert!(summary.iter().any(|s| s.contains("Read foo.rs")));

        // Verify remaining messages have complete matching pairs
        let mut tool_use_ids = std::collections::HashSet::new();
        let mut tool_result_ids = std::collections::HashSet::new();

        for msg in &remaining {
            if let Some(arr) = msg.get("content").and_then(|c| c.as_array()) {
                for block in arr {
                    if block.get("type").and_then(|t| t.as_str()) == Some("tool_use") {
                        if let Some(id) = block.get("id").and_then(|i| i.as_str()) {
                            tool_use_ids.insert(id.to_string());
                        }
                    }
                    if block.get("type").and_then(|t| t.as_str()) == Some("tool_result") {
                        if let Some(id) = block.get("tool_use_id").and_then(|i| i.as_str()) {
                            tool_result_ids.insert(id.to_string());
                        }
                    }
                }
            }
        }

        // Exact 1-to-1 match: every tool_use in remaining has its tool_result, and no orphans!
        assert_eq!(tool_use_ids, tool_result_ids);
        assert!(tool_use_ids.contains("tool_2"));
        assert!(tool_use_ids.contains("tool_3"));
        assert!(!tool_use_ids.contains("tool_1"));
    }
}
