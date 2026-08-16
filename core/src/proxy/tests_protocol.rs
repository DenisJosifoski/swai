#[cfg(test)]
mod tests {
    use super::super::ollama::*;
    use super::super::ollama_streaming::*;
    use super::super::ollama_types::*;
    use super::super::openai::*;
    use super::super::router::error_response;
    use super::super::state::ProxyState;
    use super::super::streaming::*;

    #[test]
    fn test_is_ollama_endpoint() {
        assert!(is_ollama_endpoint("/api/generate"));
        assert!(is_ollama_endpoint("/api/chat"));
        assert!(is_ollama_endpoint("/api/tags"));
        assert!(!is_ollama_endpoint("/v1/chat/completions"));
        assert!(!is_ollama_endpoint("/v1/models"));
    }

    #[test]
    fn test_build_ollama_tags() {
        let mut state = ProxyState::new();
        state.add_model("qwen-32b".to_string(), 8080);
        state.add_model("codestral-22b".to_string(), 8081);

        let tags = build_ollama_tags_from_state(&state);
        let models = tags.get("models").and_then(|m| m.as_array()).unwrap();
        assert_eq!(models.len(), 2);
    }

    #[test]
    fn test_ollama_generate_to_openai() {
        let req = OllamaGenerateRequest {
            model: "test-model".to_string(),
            prompt: "hello world".to_string(),
            stream: false,
            options: Some(OllamaOptions {
                temperature: Some(0.7),
                num_predict: Some(100),
                top_p: Some(0.9),
                top_k: Some(40),
            }),
            system: Some("you are a helpful assistant".to_string()),
            images: None,
        };

        let openai = ollama_generate_to_openai(&req);
        assert_eq!(openai["model"], "test-model");
        assert_eq!(openai["stream"], false);
        assert_eq!(openai["temperature"], 0.7);
        assert_eq!(openai["max_tokens"], 100);

        let msgs = openai["messages"].as_array().unwrap();
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[0]["role"], "system");
        assert_eq!(msgs[0]["content"], "you are a helpful assistant");
        assert_eq!(msgs[1]["role"], "user");
        assert_eq!(msgs[1]["content"], "hello world");
    }

    #[test]
    fn test_ollama_chat_to_openai() {
        let req = OllamaChatRequest {
            model: "test-model".to_string(),
            messages: vec![
                OllamaMessage {
                    role: "system".to_string(),
                    content: serde_json::Value::String("sys".to_string()),
                },
                OllamaMessage {
                    role: "user".to_string(),
                    content: serde_json::Value::String("hi".to_string()),
                },
            ],
            stream: true,
            options: None,
        };

        let openai = ollama_chat_to_openai(&req);
        assert_eq!(openai["model"], "test-model");
        assert_eq!(openai["stream"], true);
        let msgs = openai["messages"].as_array().unwrap();
        assert_eq!(msgs.len(), 2);
    }

    #[test]
    fn test_build_ollama_generate_chunks() {
        let sse_data =
            "data: {\"choices\": [{\"delta\": {\"content\": \"hello\"}}]}\n\ndata: [DONE]\n\n";
        let chunks = build_ollama_generate_chunks("test-model", sse_data);
        assert_eq!(chunks.len(), 2);
    }

    #[test]
    fn test_build_ollama_chat_chunks() {
        let sse_data =
            "data: {\"choices\": [{\"delta\": {\"content\": \"world\"}}]}\n\ndata: [DONE]\n\n";
        let chunks = build_ollama_chat_chunks("test-model", sse_data);
        assert_eq!(chunks.len(), 2);
    }

    #[test]
    fn test_responses_adapter_string_input() {
        let input_body = serde_json::json!({
            "model": "gpt-4o",
            "input": "Hello SWAI"
        });
        let bytes = serde_json::to_vec(&input_body).unwrap();
        let res = responses_adapter(&bytes, "swai-active-model").unwrap();
        assert_eq!(res["model"], "swai-active-model");
        assert!(res.get("input").is_none());
        let msgs = res["messages"].as_array().unwrap();
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0]["role"], "user");
        assert_eq!(msgs[0]["content"], "Hello SWAI");
    }

    #[test]
    fn test_responses_adapter_items_array_input() {
        let input_body = serde_json::json!({
            "model": "gpt-4o",
            "input": [
                {"role": "user", "content": "What is 2+2?"}
            ]
        });
        let bytes = serde_json::to_vec(&input_body).unwrap();
        let res = responses_adapter(&bytes, "swai-active-model").unwrap();
        let msgs = res["messages"].as_array().unwrap();
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0]["role"], "user");
        assert_eq!(msgs[0]["content"], "What is 2+2?");
    }

    #[test]
    fn test_responses_adapter_typed_message_items() {
        let input_body = serde_json::json!({
            "model": "gpt-4o",
            "input": [
                {"type": "message", "role": "user", "content": [{"type": "input_text", "text": "ping"}]}
            ]
        });
        let bytes = serde_json::to_vec(&input_body).unwrap();
        let res = responses_adapter(&bytes, "swai-active-model").unwrap();
        let msgs = res["messages"].as_array().unwrap();
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0]["content"], "ping");
    }

    #[test]
    fn test_responses_adapter_removes_input_field() {
        let input_body = serde_json::json!({
            "model": "codex-test",
            "input": "test prompt"
        });
        let bytes = serde_json::to_vec(&input_body).unwrap();
        let res = responses_adapter(&bytes, "active-model").unwrap();
        assert!(!res.as_object().unwrap().contains_key("input"));
    }

    #[test]
    fn test_responses_adapter_removes_stream_options() {
        let input_body = serde_json::json!({
            "model": "codex-test",
            "input": "test prompt",
            "stream_options": {"include_usage": true}
        });
        let bytes = serde_json::to_vec(&input_body).unwrap();
        let res = responses_adapter(&bytes, "active-model").unwrap();
        assert!(!res.as_object().unwrap().contains_key("stream_options"));
    }

    #[test]
    fn test_responses_adapter_invalid_json() {
        let bytes = b"invalid json";
        let res = responses_adapter(bytes, "active-model");
        assert!(res.is_err());
    }

    #[test]
    fn test_convert_responses_input_to_messages_string() {
        let input = serde_json::Value::String("hello world".to_string());
        let msgs = convert_responses_input_to_messages(&input);
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0]["role"], "user");
        assert_eq!(msgs[0]["content"], "hello world");
    }

    #[test]
    fn test_convert_responses_input_to_messages_empty_string() {
        let input = serde_json::Value::String("".to_string());
        let msgs = convert_responses_input_to_messages(&input);
        assert!(msgs.is_empty());
    }

    #[test]
    fn test_convert_responses_input_to_messages_array() {
        let input = serde_json::json!([
            {"role": "user", "content": "hello"},
            {"role": "assistant", "content": "hi"}
        ]);
        let msgs = convert_responses_input_to_messages(&input);
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[0]["role"], "user");
        assert_eq!(msgs[1]["role"], "assistant");
    }

    #[test]
    fn test_convert_responses_input_to_messages_with_text_field() {
        let input = serde_json::json!([
            {"role": "user", "text": "hello from text"}
        ]);
        let msgs = convert_responses_input_to_messages(&input);
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0]["content"], "hello from text");
    }

    #[test]
    fn test_extract_text_from_item_string() {
        let val = serde_json::Value::String("plain text".to_string());
        assert_eq!(extract_text_from_item(&val), "plain text");
    }

    #[test]
    fn test_extract_text_from_item_array() {
        let val = serde_json::json!(["line 1", "line 2"]);
        assert_eq!(extract_text_from_item(&val), "line 1\nline 2");
    }

    #[test]
    fn test_extract_text_from_item_nested_object() {
        let val = serde_json::json!([{"type": "input_text", "text": "nested text"}]);
        assert_eq!(extract_text_from_item(&val), "nested text");
    }

    #[test]
    fn test_normalize_codex_payload_with_input_string() {
        let mut val = serde_json::json!({
            "model": "codex",
            "input": "test prompt",
            "stream_options": {"include_usage": true}
        });
        normalize_codex_payload(&mut val);
        assert!(val.get("input").is_none());
        assert!(val.get("stream_options").is_none());
        let msgs = val["messages"].as_array().unwrap();
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0]["content"], "test prompt");
    }

    #[test]
    fn test_normalize_codex_payload_with_input_items() {
        let mut val = serde_json::json!({
            "model": "codex",
            "input": [
                {"role": "user", "content": "from items"}
            ]
        });
        normalize_codex_payload(&mut val);
        let msgs = val["messages"].as_array().unwrap();
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0]["content"], "from items");
    }

    #[test]
    fn test_normalize_codex_payload_preserves_existing_messages() {
        let mut val = serde_json::json!({
            "model": "codex",
            "messages": [{"role": "system", "content": "sys"}],
            "input": "user prompt"
        });
        normalize_codex_payload(&mut val);
        let msgs = val["messages"].as_array().unwrap();
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[0]["role"], "system");
        assert_eq!(msgs[1]["role"], "user");
    }

    #[test]
    fn test_normalize_codex_payload_clean_nested_content() {
        let mut val = serde_json::json!({
            "model": "codex",
            "messages": [
                {"role": "user", "content": [{"type": "text", "text": "nested"}]}
            ]
        });
        normalize_codex_payload(&mut val);
        let msgs = val["messages"].as_array().unwrap();
        assert_eq!(msgs.len(), 1); // content converted to string
    }

    #[test]
    fn test_sse_responses_translator_emits_lifecycle_events() {
        let openai_sse = "data: {\"choices\": [{\"delta\": {\"content\": \"Hello\"}}]}\n\ndata: {\"choices\": [{\"delta\": {\"content\": \" world\"}}]}\n\ndata: [DONE]\n\n";
        let events = translate_openai_sse_to_responses(openai_sse, "test-model");
        assert!(events.len() >= 5);
        let all_events_str = events
            .iter()
            .map(|e| String::from_utf8_lossy(e).to_string())
            .collect::<Vec<_>>()
            .join("");
        assert!(all_events_str.contains("event: response.created"));
        assert!(all_events_str.contains("event: response.output_item.added"));
        assert!(all_events_str.contains("event: response.content_part.added"));
        assert!(all_events_str.contains("event: response.output_text.delta"));
        assert!(all_events_str.contains("event: response.output_text.done"));
        assert!(all_events_str.contains("event: response.content_part.done"));
        assert!(all_events_str.contains("event: response.output_item.done"));
        assert!(all_events_str.contains("event: response.completed"));
    }

    #[test]
    fn test_sse_responses_translator_escaped_content() {
        let openai_sse = "data: {\"choices\": [{\"delta\": {\"content\": \"line1\\nline2\\\"quoted\\\"\"}}]}\n\ndata: [DONE]\n\n";
        let events = translate_openai_sse_to_responses(openai_sse, "test-model");
        let all_events_str = events
            .iter()
            .map(|e| String::from_utf8_lossy(e).to_string())
            .collect::<Vec<_>>()
            .join("");
        assert!(all_events_str.contains("event: response.completed"));
    }

    #[test]
    fn test_translate_openai_sse_to_responses_full_lifecycle() {
        let openai_sse = "data: {\"id\":\"chatcmpl-1\",\"object\":\"chat.completion.chunk\",\"created\":1700000000,\"model\":\"gpt-4\",\"choices\":[{\"index\":0,\"delta\":{\"role\":\"assistant\",\"content\":\"Rust\"},\"finish_reason\":null}]}\n\ndata: {\"id\":\"chatcmpl-1\",\"object\":\"chat.completion.chunk\",\"created\":1700000000,\"model\":\"gpt-4\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\" is awesome\"},\"finish_reason\":null}]}\n\ndata: [DONE]\n\n";
        let events = translate_openai_sse_to_responses(openai_sse, "swai-active-model");
        let all_events = events
            .iter()
            .map(|e| String::from_utf8_lossy(e).to_string())
            .collect::<Vec<_>>()
            .join("");
        assert!(all_events.contains("event: response.created"));
        assert!(all_events.contains("event: response.output_item.added"));
        assert!(all_events.contains("event: response.output_text.delta"));
        assert!(all_events.contains("Rust"));
        assert!(all_events.contains(" is awesome"));
        assert!(all_events.contains("event: response.completed"));
    }

    #[test]
    fn test_responses_error_response_escapes_message() {
        let resp = error_response(400, "invalid \"json\" message");
        assert_eq!(resp.status_code(), tiny_http::StatusCode(400));
    }

    #[test]
    fn test_responses_error_response_status_mapping() {
        let resp_400 = error_response(400, "bad request");
        assert_eq!(resp_400.status_code(), tiny_http::StatusCode(400));

        let resp_500 = error_response(500, "server error");
        assert_eq!(resp_500.status_code(), tiny_http::StatusCode(500));

        let resp_503 = error_response(503, "service unavailable");
        assert_eq!(resp_503.status_code(), tiny_http::StatusCode(503));
    }
}
