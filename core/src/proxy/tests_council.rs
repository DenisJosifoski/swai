#[cfg(test)]
mod tests {
    use super::super::router::{
        build_council_sse_events, escape_sse_text, extract_model_from_body,
        extract_prompt_from_body, is_council_model, parse_pipeline_header,
    };
    use crate::council::types::{CouncilPipelineConfig, DebateOutcome, DebateTranscript};

    #[test]
    fn test_is_council_model_exact() {
        assert!(is_council_model("council"));
        assert!(!is_council_model("COUNCIL"));
        assert!(!is_council_model("counci"));
        assert!(!is_council_model("councilary"));
    }

    #[test]
    fn test_is_council_model_prefixed() {
        assert!(is_council_model("council:debate"));
        assert!(is_council_model("council:multi-agent"));
        assert!(is_council_model("council:custom-pipeline"));
        assert!(!is_council_model("councilary"));
        assert!(!is_council_model("my-council-model"));
    }

    #[test]
    fn test_extract_model_from_body() {
        let body = br#"{"model": "gpt-4o", "messages": [{"role": "user", "content": "hi"}]}"#;
        assert_eq!(extract_model_from_body(body), Some("gpt-4o".to_string()));

        let council_body = br#"{"model": "council:debate", "messages": []}"#;
        assert_eq!(
            extract_model_from_body(council_body),
            Some("council:debate".to_string())
        );

        let empty_body = b"";
        assert_eq!(extract_model_from_body(empty_body), None);

        let no_model = br#"{"messages": []}"#;
        assert_eq!(extract_model_from_body(no_model), None);
    }

    #[test]
    fn test_extract_prompt_from_body() {
        let body = br#"{"messages": [{"role": "user", "content": "What is Rust?"}]}"#;
        assert_eq!(
            extract_prompt_from_body(body),
            Some("What is Rust?".to_string())
        );

        // Multi-turn: extracts first user message.
        let multi_turn = br#"{"messages": [
            {"role": "system", "content": "be helpful"},
            {"role": "user", "content": "hello there"}
        ]}"#;
        assert_eq!(
            extract_prompt_from_body(multi_turn),
            Some("hello there".to_string())
        );

        // Empty messages array.
        let empty = br#"{"messages": []}"#;
        assert_eq!(extract_prompt_from_body(empty), None);
    }

    #[test]
    fn test_parse_pipeline_header_valid() {
        let header = r#"{"stages": [{"model_id": "qwen-32b", "temperature": 0.7, "top_p": 0.9}]}"#;
        let config = parse_pipeline_header(header);
        assert!(config.is_some());
        let config = config.unwrap();
        assert_eq!(config.stages.len(), 1);
        assert_eq!(config.stages[0].model_id, "qwen-32b");
    }

    #[test]
    fn test_parse_pipeline_header_invalid() {
        let invalid = r#"not json at all"#;
        assert!(parse_pipeline_header(invalid).is_none());

        let empty = "";
        assert!(parse_pipeline_header(empty).is_none());
    }

    #[test]
    fn test_build_council_sse_events_success() {
        let outcome = DebateOutcome::Success {
            final_response: "Rust is awesome".to_string(),
            transcript: DebateTranscript::new(
                "s1".into(),
                "prompt".into(),
                CouncilPipelineConfig::default(),
            ),
        };

        let events = build_council_sse_events(&outcome, "council:debate");
        let all = events
            .iter()
            .map(|e| String::from_utf8_lossy(e).to_string())
            .collect::<Vec<_>>()
            .join("");

        assert!(all.contains("event: debate.started"));
        assert!(all.contains("\"model\":\"council:debate\""));
        assert!(all.contains("event: content_block_delta"));
        assert!(all.contains("Rust is awesome"));
        assert!(all.contains("event: message_stop"));
    }

    #[test]
    fn test_build_council_sse_events_failed() {
        let outcome = DebateOutcome::Aborted {
            reason: "all models timed out".to_string(),
            transcript: DebateTranscript::new(
                "s1".into(),
                "prompt".into(),
                CouncilPipelineConfig::default(),
            ),
        };

        let events = build_council_sse_events(&outcome, "council");
        let all = events
            .iter()
            .map(|e| String::from_utf8_lossy(e).to_string())
            .collect::<Vec<_>>()
            .join("");

        assert!(all.contains("event: debate.started"));
        assert!(all.contains("Debate aborted: all models timed out"));
        assert!(all.contains("event: message_stop"));
    }

    #[test]
    fn test_escape_sse_text() {
        assert_eq!(escape_sse_text("hello"), "hello");
        assert_eq!(escape_sse_text("line1\nline2"), "line1\\nline2");
        assert_eq!(escape_sse_text(r#"say "hi""#), r#"say \"hi\""#);
        assert_eq!(escape_sse_text(r#"back\slash"#), r#"back\\slash"#);
    }
}
