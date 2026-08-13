use swai_core::compaction::*;
use serde_json::json;

#[test]
fn message_serde_roundtrip() {
    let msg = Message {
        role: "user".to_string(),
        content: vec![serde_json::Value::String("Hello".to_string())],
    };

    let serialized = serde_json::to_string(&msg).unwrap();
    let deserialized: Message = serde_json::from_str(&serialized).unwrap();

    assert_eq!(deserialized.role, "user");
    assert_eq!(deserialized.content.len(), 1);
}

#[test]
fn message_from_anthropic_format() {
    // Anthropic API format: {"role": "user", "content": "text"} or array
    let obj = json!({
        "role": "assistant",
        "content": [{"type": "text", "text": "Hello world"}]
    });

    let msg: Message = serde_json::from_value(obj).unwrap();
    assert_eq!(msg.role, "assistant");
}

#[test]
fn compaction_config_defaults() {
    let cfg = CompactionConfig::default();
    assert!(!cfg.enabled);
    assert!(cfg.max_tokens > 0);
    assert!(cfg.summary_length > 0);
}

#[test]
fn compaction_config_from_toml() {
    let toml_str = r#"
        enabled = true
        max_tokens = 50000
        summary_length = 2000
    "#;

    let cfg: CompactionConfig = toml::from_str(toml_str).unwrap();
    assert!(cfg.enabled);
    assert_eq!(cfg.max_tokens, 50000);
    assert_eq!(cfg.summary_length, 2000);
}

#[test]
fn compaction_config_from_toml_invalid() {
    let result: Result<CompactionConfig, _> = toml::from_str("not valid toml [[[");
    assert!(result.is_err());
}
