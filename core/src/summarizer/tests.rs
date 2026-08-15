#[cfg(test)]
mod tests {
    use crate::config::Config;
    use crate::process_manager::ProcessManager;
    use crate::proxy::ProxyState;
    use crate::summarizer::*;
    use serde_json::Value;
    use std::sync::{Arc, Mutex};


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
