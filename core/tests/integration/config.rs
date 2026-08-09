use swai_core::config::*;
use std::path::PathBuf;

#[test]
fn default_config() {
    let cfg = Config::default();
    assert_eq!(cfg.port, 8080);
    assert!(cfg.host.is_empty());
    assert!(!cfg.api_key.is_empty());
    assert!(!cfg.proxy_id.is_empty());
    assert!(!cfg.session_id.is_empty());
    assert!(!cfg.model_name.is_empty());
    assert!(!cfg.client_user_agent.is_empty());
}

#[test]
fn config_from_env() {
    std::env::set_var("SWAI_HOST", "0.0.0.0");
    std::env::set_var("SWAI_PORT", "9090");
    std::env::set_var("SWAI_API_KEY", "key-from-env");

    let cfg = Config::from_env();
    assert_eq!(cfg.host, "0.0.0.0");
    assert_eq!(cfg.port, 9090);
    assert_eq!(cfg.api_key, "key-from-env");

    std::env::remove_var("SWAI_HOST");
    std::env::remove_var("SWAI_PORT");
    std::env::remove_var("SWAI_API_KEY");
}

#[test]
fn config_to_toml_roundtrip() {
    let cfg = Config::default();
    let toml_str = cfg.to_toml();
    let deserialized = Config::from_toml(&toml_str).unwrap();
    assert_eq!(deserialized.port, cfg.port);
    assert_eq!(deserialized.host, cfg.host);
}

#[test]
fn config_loads_from_file() {
    let dir = std::env::temp_dir().join("swai-test-config");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("config.toml");
    let cfg = Config::default();
    std::fs::write(&path, cfg.to_toml()).unwrap();

    let loaded = Config::from_file(&path).unwrap();
    assert_eq!(loaded.port, cfg.port);

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn config_from_file_missing() {
    let result = Config::from_file(PathBuf::from("/nonexistent/path/config.toml"));
    assert!(result.is_err());
}

#[test]
fn config_from_toml_invalid() {
    let result = Config::from_toml("this is not [[valid]] toml");
    assert!(result.is_err());
}
