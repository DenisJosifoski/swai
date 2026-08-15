#[cfg(test)]
mod tests {
    use super::super::router::*;
    use super::super::state::ProxyState;

    #[test]
    fn test_proxy_state_default() {
        let state = ProxyState::default();
        assert!(state.primary_port.is_none());
        assert!(state.active_models.is_empty());
        assert!(!state.is_loading);
    }

    #[test]
    fn test_proxy_state_set_target() {
        let mut state = ProxyState::new();
        state.set_target(8080);
        assert_eq!(state.primary_port, Some(8080));
        assert!(!state.is_loading);
    }

    #[test]
    fn test_proxy_state_set_loading() {
        let mut state = ProxyState::new();
        state.set_target(8080);
        assert!(!state.is_loading);
        state.set_loading();
        assert!(state.is_loading);
    }

    #[test]
    fn test_proxy_state_clear() {
        let mut state = ProxyState::new();
        state.set_target(8080);
        state.clear();
        assert!(state.primary_port.is_none());
        assert!(!state.is_loading);
    }

    #[test]
    fn test_proxy_state_multi_model_add_remove() {
        let mut state = ProxyState::new();
        state.add_model("model-a".to_string(), 8080);
        state.add_model("model-b".to_string(), 8081);

        assert_eq!(state.primary_port, Some(8080));
        assert_eq!(state.active_models.len(), 2);
        assert_eq!(state.active_models.get("model-a"), Some(&8080));
        assert_eq!(state.active_models.get("model-b"), Some(&8081));

        let removed = state.remove_model("model-a");
        assert_eq!(removed, Some(8080));
        assert_eq!(state.primary_port, Some(8081));
        assert_eq!(state.active_models.len(), 1);

        let removed_b = state.remove_model("model-b");
        assert_eq!(removed_b, Some(8081));
        assert_eq!(state.primary_port, None);
        assert!(state.active_models.is_empty());
    }

    #[test]
    fn test_proxy_state_find_model_port() {
        let mut state = ProxyState::new();
        state.add_model("qwen-32b".to_string(), 8080);
        state.add_model("codestral-22b".to_string(), 8081);

        assert_eq!(state.find_model_port("qwen-32b"), Some(8080));
        assert_eq!(state.find_model_port("codestral-22b"), Some(8081));
        assert_eq!(state.find_model_port("nonexistent"), None);
    }

    #[test]
    fn test_proxy_state_lifecycle() {
        let mut state = ProxyState::new();
        assert!(state.primary_port.is_none());
        assert!(!state.is_loading);

        state.set_loading();
        assert!(state.is_loading);

        state.set_target(9090);
        assert_eq!(state.primary_port, Some(9090));
        assert!(!state.is_loading);

        state.set_loading();
        assert!(state.is_loading);

        state.clear();
        assert!(state.primary_port.is_none());
        assert!(!state.is_loading);
    }

    #[test]
    fn test_resolve_target_port_matches_running_model() {
        let mut state = ProxyState::new();
        state.add_model("qwen-32b".to_string(), 8080);
        state.add_model("codestral-22b".to_string(), 8081);

        let body = br#"{"model": "codestral-22b", "messages": []}"#;
        assert_eq!(resolve_target_port(&state, body), Some(8081));

        let body_a = br#"{"model": "qwen-32b", "prompt": "hi"}"#;
        assert_eq!(resolve_target_port(&state, body_a), Some(8080));
    }

    #[test]
    fn test_resolve_target_port_falls_back_when_no_match() {
        let mut state = ProxyState::new();
        state.add_model("qwen-32b".to_string(), 8080);

        let body = br#"{"model": "unknown-model", "messages": []}"#;
        assert_eq!(resolve_target_port(&state, body), None);
    }

    #[test]
    fn test_resolve_target_port_empty_body() {
        let state = ProxyState::new();
        assert_eq!(resolve_target_port(&state, b""), None);
    }

    #[test]
    fn test_resolve_target_port_no_model_field() {
        let mut state = ProxyState::new();
        state.add_model("qwen-32b".to_string(), 8080);

        let body = br#"{"messages": [{"role": "user", "content": "hi"}]}"#;
        assert_eq!(resolve_target_port(&state, body), None);
    }

    #[test]
    fn test_resolve_target_port_empty_model_value() {
        let mut state = ProxyState::new();
        state.add_model("qwen-32b".to_string(), 8080);

        let body = br#"{"model": "", "messages": []}"#;
        assert_eq!(resolve_target_port(&state, body), None);
    }

    #[test]
    fn test_resolve_target_port_invalid_json() {
        let mut state = ProxyState::new();
        state.add_model("qwen-32b".to_string(), 8080);

        let body = b"not json at all";
        assert_eq!(resolve_target_port(&state, body), None);
    }

    #[test]
    fn test_is_hop_by_hop_header() {
        assert!(is_hop_by_hop_header("connection"));
        assert!(is_hop_by_hop_header("Connection"));
        assert!(is_hop_by_hop_header("keep-alive"));
        assert!(is_hop_by_hop_header("Keep-Alive"));
        assert!(is_hop_by_hop_header("transfer-encoding"));
        assert!(is_hop_by_hop_header("upgrade"));
        assert!(is_hop_by_hop_header("te"));
        assert!(is_hop_by_hop_header("trailer"));
        assert!(is_hop_by_hop_header("proxy-authorization"));
        assert!(is_hop_by_hop_header("proxy-authenticate"));

        assert!(!is_hop_by_hop_header("content-type"));
        assert!(!is_hop_by_hop_header("Content-Length"));
        assert!(!is_hop_by_hop_header("authorization"));
        assert!(!is_hop_by_hop_header("accept"));
        assert!(!is_hop_by_hop_header("host"));
    }

    #[test]
    fn test_error_response() {
        let resp = error_response(503, "test error");
        assert_eq!(resp.status_code(), tiny_http::StatusCode(503));
    }
}
