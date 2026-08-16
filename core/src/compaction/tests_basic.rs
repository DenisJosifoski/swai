#[cfg(test)]
mod tests {
    use crate::compaction::*;
    use crate::compaction::*;
    use crate::compaction::*;
    use serde_json::Value;

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
}
