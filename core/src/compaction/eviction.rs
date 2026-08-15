use super::extractor::build_eviction_units;
use serde_json::Value;
use super::extractor::serialize_dropped_slice;
use super::types::CompactionConfig;

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
