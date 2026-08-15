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
/// - Persist session checkpoints with cumulative deduplication
/// - Inject full cumulative checkpoint summaries and anti-hallucination reminders
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

    // Only trigger compaction when the request payload is actually large enough to threaten context limits (> 120KB)
    if let Some(messages_arr) = json_val.get("messages").and_then(|m| m.as_array()).cloned() {
        if request_body_len > 120_000 {
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

            // 2. If messages were dropped, update session checkpoint with deduplication
            if !summary_lines.is_empty() {
                let session_name = state.lock().ok().and_then(|s| {
                    s.active_models.iter().find(|(_, &p)| p == target_port).map(|(id, _)| id.clone())
                }).unwrap_or_else(|| if !model_id.is_empty() { model_id } else { "swai-session".to_string() });

                let writer = crate::checkpoint::CheckpointWriter::new(&session_name).ok();
                
                // Load existing session checkpoint to check for duplicates and accumulate entries
                let mut session = writer.as_ref()
                    .and_then(|w| w.load_session())
                    .unwrap_or_else(|| crate::checkpoint::SessionCheckpoint::new(session_name.clone()));

                if let Some(ref obj) = initial_objective {
                    if session.initial_objective.is_none() {
                        session.set_initial_objective(obj.clone());
                    }
                }

                // Per-line deduplication across all existing checkpoint entries
                let mut existing_lines = std::collections::HashSet::new();
                for entry in &session.entries {
                    for line in &entry.summary_lines {
                        existing_lines.insert(line.clone());
                    }
                }

                // Filter out any lines that have already been recorded in previous checkpoints
                let new_unique_lines: Vec<String> = summary_lines.into_iter()
                    .filter(|l| !existing_lines.contains(l))
                    .collect();

                if !new_unique_lines.is_empty() {
                    let next_idx = session.entries.len() + 1;
                    let entry = crate::checkpoint::CheckpointEntry {
                        index: next_idx,
                        timestamp: chrono::Utc::now().to_rfc3339(),
                        summary_lines: new_unique_lines.clone(),
                    };
                    if let Some(ref w) = writer {
                        let _ = w.write_entry_with_objective(&entry, initial_objective.as_deref());
                    }
                    session.entries.push(entry);
                }

                // Inject the FULL cumulative checkpoint formatted with anti-hallucination guard
                if let Some(checkpoint_text) = session.format_for_injection() {
                    crate::compaction::inject_checkpoint_into_payload(json_val, &checkpoint_text);
                }
            }
        }
    }
}
