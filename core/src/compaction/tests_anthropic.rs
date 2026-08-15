#[cfg(test)]
mod tests {
    use crate::compaction::*;
    use crate::compaction::*;
    use crate::compaction::*;
    use serde_json::Value;


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
        assert!(remaining.len() < messages.len()); // Dropped older messages to meet budget
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

    #[test]
    fn test_plan_file_strictly_preserved_across_all_passes() {
        let mut messages: Vec<Value> = Vec::new();

        // 1. Initial user prompt
        messages.push(serde_json::json!({
            "role": "user",
            "content": "Implement Phase 24"
        }));

        // 2. Assistant reads PLAN/PHASE24.md
        messages.push(serde_json::json!({
            "role": "assistant",
            "content": [{
                "type": "tool_use",
                "name": "Read",
                "input": {"file_path": "PLAN/PHASE24.md"}
            }]
        }));
        messages.push(serde_json::json!({
            "role": "user",
            "content": [{
                "type": "tool_result",
                "content": "Spec content here"
            }]
        }));

        // 3. Add heavy non-plan exploratory turns that exceed budget
        for i in 0..15 {
            messages.push(serde_json::json!({
                "role": "assistant",
                "content": [{
                    "type": "tool_use",
                    "name": "RunCommand",
                    "input": {"command": format!("find src -name '*.rs' {}", i)}
                }]
            }));
            messages.push(serde_json::json!({
                "role": "user",
                "content": [{
                    "type": "tool_result",
                    "content": format!("Large output data string {} with lots of text to blow past budget...", i)
                }]
            }));
        }

        let config = CompactionConfig {
            enabled: true,
            max_tokens: 50, // Tiny budget forces Pass 1 and Pass 2
            summary_length: 100,
        };

        let (_summary, remaining) = compact_messages_anthropic(&messages, &config);

        // Verify that PLAN/PHASE24.md read was NOT dropped despite tiny budget!
        let has_plan_read = remaining.iter().any(|msg| {
            if let Some(arr) = msg.get("content").and_then(|c| c.as_array()) {
                arr.iter().any(|b| {
                    b.get("type").and_then(|t| t.as_str()) == Some("tool_use")
                        && b.get("name").and_then(|n| n.as_str()) == Some("Read")
                        && b.get("input").and_then(|i| i.get("file_path")).and_then(|p| p.as_str()) == Some("PLAN/PHASE24.md")
                })
            } else {
                false
            }
        });

        assert!(has_plan_read, "PLAN/PHASE24.md must be strictly preserved across all eviction passes");
    }
}
