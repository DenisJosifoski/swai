use std::sync::{Arc, Mutex};
use super::state::ProxyState;

/// Check if a text message is a synthetic client prompt (recap, reminder) that should be skipped.
pub fn is_synthetic_prompt(text: &str) -> bool {
    let t = text.trim();
    t.starts_with("<system-reminder>")
        || t.starts_with("<system_reminder>")
        || t.starts_with("The user stepped away")
        || t.starts_with("As you answer the user")
        || t.is_empty()
}

/// Process Anthropic `/v1/messages` payloads:
/// - Remap model ID to claude-sonnet-4-5
/// - Perform context compaction when payload threatens context budget
/// - Persist session checkpoints
/// - Inject checkpoint summaries and anti-hallucination reminders
pub fn process_anthropic_payload(
    json_val: &mut serde_json::Value,
    request_body_len: usize,
    state: &Arc<Mutex<ProxyState>>,
    target_port: u16,
) {
    let mut model_id = String::new();
    if let Some(obj) = json_val.as_object_mut() {
        if let Some(m) = obj.get("model").and_then(|v| v.as_str()) {
            model_id = m.to_string();
        }
        // Remap model field so llama-server doesn't reject custom model IDs
        obj.insert("model".to_string(), serde_json::Value::String("claude-sonnet-4-5".to_string()));
    }

    // Only trigger compaction when the request payload is ACTUALLY large enough to threaten context limits (> 30KB or > 10 messages AND > 15KB).
    // Do NOT compact tiny polling turns or small conversational messages just because message count > 4!
    if let Some(messages_arr) = json_val.get("messages").and_then(|m| m.as_array()).cloned() {
        if request_body_len > 30_000 || (messages_arr.len() > 10 && request_body_len > 15_000) {
            // Extract initial user prompt objective (skipping any synthetic client blocks)
            let initial_objective: Option<String> = messages_arr.iter()
                .filter(|m| m.get("role").and_then(|r| r.as_str()) == Some("user"))
                .find_map(|m| {
                    if let Some(arr) = m.get("content").and_then(|c| c.as_array()) {
                        for b in arr {
                            if b.get("type").and_then(|t| t.as_str()) == Some("text") {
                                if let Some(t) = b.get("text").and_then(|s| s.as_str()) {
                                    let trimmed = t.trim();
                                    if !is_synthetic_prompt(trimmed) {
                                        return Some(trimmed.chars().take(300).collect());
                                    }
                                }
                            }
                        }
                        None
                    } else if let Some(s) = m.get("content").and_then(|c| c.as_str()) {
                        let trimmed = s.trim();
                        if let Some(end_tag) = trimmed.find("</system-reminder>") {
                            let remainder = trimmed[end_tag + "</system-reminder>".len()..].trim();
                            if !is_synthetic_prompt(remainder) {
                                return Some(remainder.chars().take(300).collect());
                            }
                        }
                        if !is_synthetic_prompt(trimmed) {
                            Some(trimmed.chars().take(300).collect())
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                });

            let compaction_cfg = crate::compaction::CompactionConfig::default();
            let (summary_lines, remaining) = crate::compaction::compact_messages_anthropic(&messages_arr, &compaction_cfg);
            
            // 1. Always update messages array with the compacted/truncated remaining messages
            if let Some(obj) = json_val.as_object_mut() {
                obj.insert("messages".to_string(), serde_json::Value::Array(remaining));
            }

            // 2. If messages were dropped, write checkpoint to disk and inject checkpoint text
            if !summary_lines.is_empty() {
                let session_name = state.lock().ok().and_then(|s| {
                    s.active_models.iter().find(|(_, &p)| p == target_port).map(|(id, _)| id.clone())
                }).unwrap_or_else(|| if !model_id.is_empty() { model_id } else { "swai-session".to_string() });

                if let Ok(writer) = crate::checkpoint::CheckpointWriter::new(&session_name) {
                    let next_idx = writer.next_checkpoint_index();
                    let entry = crate::checkpoint::CheckpointEntry {
                        index: next_idx,
                        timestamp: chrono::Utc::now().to_rfc3339(),
                        summary_lines: summary_lines.clone(),
                    };
                    let _ = writer.write_entry_with_objective(&entry, initial_objective.as_deref());
                }

                let mut checkpoint_lines = Vec::new();
                checkpoint_lines.push("[Session checkpoint — earlier work in this conversation, condensed]".to_string());
                if let Some(ref obj) = initial_objective {
                    checkpoint_lines.push(format!("Initial Objective: {}", obj));
                }
                checkpoint_lines.push("Note: this is a condensed action log, not literal file content. If you need exact field names, types, function signatures, or other precise code details from a file listed below, re-read that file — do not reconstruct it from memory.".to_string());
                for (i, l) in summary_lines.iter().enumerate() {
                    checkpoint_lines.push(format!("{}. {}", i + 1, l));
                }
                checkpoint_lines.push("[End checkpoint — continuing below]".to_string());
                let checkpoint_text = checkpoint_lines.join("\n");

                crate::compaction::inject_checkpoint_into_payload(json_val, &checkpoint_text);
            }
        }
    }
}
