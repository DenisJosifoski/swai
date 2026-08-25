use super::*;
use crate::council::CouncilPipelineConfig;

#[test]
fn test_port_check_on_free_port() {
    // Port 9999 should be free
    let state = PortState::from_port_check(9999);
    assert_eq!(state, PortState::Free);
}

#[test]
fn test_extract_model_name_openai_format() {
    let body = r#"{"data": [{"id": "gpt-3.5-turbo", "object": "model"}]}"#;
    assert_eq!(
        extract_model_name_from_response(body),
        Some("gpt-3.5-turbo".to_string())
    );
}

#[test]
fn test_extract_model_name_openai_empty_data() {
    let body = r#"{"data": []}"#;
    assert_eq!(extract_model_name_from_response(body), None);
}

#[test]
fn test_extract_model_name_ollama_format() {
    let body = r#"{"models": [{"name": "llama3:8b", "size": 4700000000}]}"#;
    assert_eq!(
        extract_model_name_from_response(body),
        Some("llama3:8b".to_string())
    );
}

#[test]
fn test_extract_model_name_ollama_empty_models() {
    let body = r#"{"models": []}"#;
    assert_eq!(extract_model_name_from_response(body), None);
}

#[test]
fn test_extract_model_name_invalid_json() {
    let body = "not json at all";
    assert_eq!(extract_model_name_from_response(body), None);
}

#[test]
fn test_extract_model_name_empty_id() {
    let body = r#"{"data": [{"id": "", "object": "model"}]}"#;
    assert_eq!(extract_model_name_from_response(body), None);
}

#[test]
fn test_extract_model_name_no_data_key() {
    let body = r#"{"something_else": true}"#;
    assert_eq!(extract_model_name_from_response(body), None);
}

#[test]
fn test_extract_model_name_ollama_empty_name() {
    let body = r#"{"models": [{"name": ""}]}"#;
    assert_eq!(extract_model_name_from_response(body), None);
}

#[test]
fn test_unmanaged_server_info_display() {
    let info = UnmanagedModelInfo {
        port: 11434,
        model_name: "llama3:8b".to_string(),
    };
    assert_eq!(info.port, 11434);
    assert_eq!(info.model_name, "llama3:8b");
}

#[test]
fn test_probe_filters_configured_ports() {
    // Create a config with port 8080 configured.
    let config = Config {
        schema_version: 1,
        models: vec![crate::config::ModelConfig {
            id: "test-model".to_string(),
            name: "Test Model".to_string(),
            script_path: std::path::PathBuf::from("/dev/null"),
            port: 8080,
            health_timeout_sec: 30,
            ctx_size: 65_536,
        }],
        global: crate::config::GlobalSettings::default(),
        preferences: crate::config::PreferencesConfig::default(),
        council: CouncilPipelineConfig::default(),
    };

    let reconciler = Reconciler::new(config);
    // Port 8080 should be skipped; result is empty since no servers are running.
    let results = reconciler.probe_unmanaged_servers();
    for r in &results {
        assert_ne!(r.port, 8080);
    }
}

#[test]
fn test_probe_returns_empty_when_no_servers_running() {
    // With no servers on common ports, probing should return an empty vec.
    let config = Config {
        schema_version: 1,
        models: vec![],
        global: crate::config::GlobalSettings::default(),
        preferences: crate::config::PreferencesConfig::default(),
        council: CouncilPipelineConfig::default(),
    };

    let reconciler = Reconciler::new(config);
    let results = reconciler.probe_unmanaged_servers();
    assert!(results.is_empty());
}

#[test]
fn test_common_ports_constant() {
    // Verify the common ports list includes expected LLM server ports.
    assert!(COMMON_LLM_PORTS.contains(&8000));
    assert!(COMMON_LLM_PORTS.contains(&8080));
    assert!(COMMON_LLM_PORTS.contains(&8081));
    assert!(COMMON_LLM_PORTS.contains(&11434));
}

#[test]
fn test_extract_model_name_mixed_format_fallback() {
    // A response with no `data` key but valid Ollama format should still work.
    let body = r#"{"models": [{"name": "qwen2:7b"}]}"#;
    assert_eq!(
        extract_model_name_from_response(body),
        Some("qwen2:7b".to_string())
    );
}

#[test]
fn test_extract_model_name_openai_with_extra_fields() {
    // Response with extra fields should still parse correctly.
    let body = r#"{
        "object": "list",
        "data": [
            {"id": "gpt-4", "object": "model", "owned_by": "openai"},
            {"id": "gpt-3.5-turbo", "object": "model", "owned_by": "openai"}
        ]
    }"#;
    assert_eq!(
        extract_model_name_from_response(body),
        Some("gpt-4".to_string())
    );
}
