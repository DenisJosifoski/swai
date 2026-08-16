#[cfg(test)]
mod tests {
    use crate::config::*;
    use std::path::PathBuf;
    use tempfile::TempDir;

    use std::fs;

    fn make_temp_model_script(tmp: &tempfile::TempDir, name: &str) -> PathBuf {
        let script = tmp.path().join(format!("{}.sh", name));
        fs::write(&script, "#!/bin/sh\necho hello\n").unwrap();
        #[cfg(unix)]
        std::process::Command::new("chmod")
            .arg("+x")
            .arg(&script)
            .status()
            .ok();
        script
    }

    #[allow(dead_code)]
    fn make_temp_config(tmp: &tempfile::TempDir, models: &[(&str, &str, u16)]) -> PathBuf {
        let config_path = tmp.path().join("config.toml");
        let mut content = String::new();
        content.push_str("[models]\n");
        for (i, (id, name, port)) in models.iter().enumerate() {
            content.push_str(&format!(
                "[models.model_{}]\nid = \"{}\"\nname = \"{}\"\nscript_path = \"{}\"\nport = {}\nschema_version = 1\n",
                i, id, name, tmp.path().join(format!("{}.sh", id)).display(), port
            ));
        }
        content.push_str(
            "[global]\nlog_dir = \"\"\nproxy_port = 8080\nauto_restart_on_context_full = true\n",
        );
        fs::write(&config_path, &content).unwrap();
        config_path
    }

    #[test]
    fn test_validate_empty_model_list() {
        let tmp = tempfile::tempdir().unwrap();
        let config = Config {
            schema_version: 1,
            models: vec![],
            global: GlobalSettings::default(),
            preferences: PreferencesConfig::default(),
        };
        let result = Config::validate(&config, tmp.path());
        assert!(result.is_ok());
    }

    #[test]
    fn test_validate_duplicate_ports() {
        let tmp = tempfile::tempdir().unwrap();
        let script1 = make_temp_model_script(&tmp, "model1");
        let script2 = make_temp_model_script(&tmp, "model2");
        let config = Config {
            schema_version: 1,
            models: vec![
                ModelConfig {
                    id: "m1".to_string(),
                    name: "Model 1".to_string(),
                    script_path: script1.clone(),
                    port: 8081,
                    health_timeout_sec: 30,
                    ctx_size: 65_536,
                },
                ModelConfig {
                    id: "m2".to_string(),
                    name: "Model 2".to_string(),
                    script_path: script2.clone(),
                    port: 8081, // duplicate
                    health_timeout_sec: 30,
                    ctx_size: 65_536,
                },
            ],
            global: GlobalSettings::default(),
            preferences: PreferencesConfig::default(),
        };
        let result = Config::validate(&config, tmp.path());
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("duplicate port"));
    }

    #[test]
    fn test_validate_missing_script() {
        let tmp = tempfile::tempdir().unwrap();
        let config = Config {
            schema_version: 1,
            models: vec![ModelConfig {
                id: "m1".to_string(),
                name: "Model 1".to_string(),
                script_path: tmp.path().join("nonexistent.sh"),
                port: 8081,
                health_timeout_sec: 30,
                ctx_size: 65_536,
            }],
            global: GlobalSettings::default(),
            preferences: PreferencesConfig::default(),
        };
        let result = Config::validate(&config, tmp.path());
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("script not found"));
    }

    #[test]
    fn test_default_log_dir() {
        let dir = Config::default_log_dir();
        assert!(dir.to_string_lossy().contains(".local/share/swai/logs"));
    }

    #[test]
    fn test_defaults() {
        assert_eq!(Config::default_proxy_port(), 9080);
        assert!(Config::default_auto_restart_on_context_full());
    }

    #[test]
    fn test_auto_follow_logs_default() {
        let config = Config {
            schema_version: 1,
            models: vec![],
            global: GlobalSettings::default(),
            preferences: PreferencesConfig::default(),
        };
        assert!(config.auto_follow_logs());
    }

    #[test]
    fn test_auto_follow_logs_serialization() {
        // Verify auto_follow_logs round-trips through TOML serialization.
        let config = Config {
            schema_version: 1,
            models: vec![],
            global: GlobalSettings::default(),
            preferences: PreferencesConfig {
                auto_follow_logs: false,
                enable_notifications: true,
                notify_on_switch: true,
                autostart_on_login: false,
                max_concurrent_models: 2,
                checkpoint_summarizer_model: None,
            },
        };

        let serialized = toml::to_string_pretty(&config).unwrap();
        assert!(serialized.contains("auto_follow_logs"));

        let deserialized: Config = toml::from_str(&serialized).unwrap();
        assert!(!deserialized.preferences.auto_follow_logs);
        assert_eq!(deserialized.preferences.max_concurrent_models, 2);
    }

    #[test]
    fn test_auto_follow_logs_missing_uses_default() {
        // When preferences section is absent from TOML, defaults should be used.
        let toml_str = r#"
schema_version = 1

[global]
log_dir = ""
proxy_port = 9080
auto_restart_on_context_full = true
"#;
        let config: Config = toml::from_str(toml_str).unwrap();
        assert!(config.auto_follow_logs());
        assert!(config.enable_notifications());
        assert!(config.notify_on_switch());
    }

    #[test]
    fn test_notification_preferences_defaults() {
        // Verify notification preferences default to true.
        let config = Config {
            schema_version: 1,
            models: vec![],
            global: GlobalSettings::default(),
            preferences: PreferencesConfig::default(),
        };
        assert!(config.enable_notifications());
        assert!(config.notify_on_switch());
    }

    #[test]
    fn test_notification_preferences_serialization() {
        // Verify notification preferences round-trip through TOML serialization.
        let config = Config {
            schema_version: 1,
            models: vec![],
            global: GlobalSettings::default(),
            preferences: PreferencesConfig {
                auto_follow_logs: true,
                enable_notifications: false,
                notify_on_switch: false,
                autostart_on_login: false,
                max_concurrent_models: 3,
                checkpoint_summarizer_model: None,
            },
        };

        let serialized = toml::to_string_pretty(&config).unwrap();
        assert!(serialized.contains("enable_notifications"));
        assert!(serialized.contains("notify_on_switch"));

        let deserialized: Config = toml::from_str(&serialized).unwrap();
        assert!(!deserialized.preferences.enable_notifications);
        assert!(!deserialized.preferences.notify_on_switch);
    }

    #[test]
    fn test_notification_preferences_missing_uses_default() {
        // When notification preferences are absent from TOML, defaults should be used.
        let toml_str = r#"
schema_version = 1

[global]
log_dir = ""
proxy_port = 9080
auto_restart_on_context_full = true
"#;
        let config: Config = toml::from_str(toml_str).unwrap();
        assert!(config.enable_notifications());
        assert!(config.notify_on_switch());
    }

    #[test]
    fn test_max_concurrent_models_default() {
        let config = Config {
            schema_version: 1,
            models: vec![],
            global: GlobalSettings::default(),
            preferences: PreferencesConfig::default(),
        };
        assert_eq!(config.max_concurrent_models(), 1);
    }

    #[test]
    fn test_max_concurrent_models_serialization() {
        let config = Config {
            schema_version: 1,
            models: vec![],
            global: GlobalSettings::default(),
            preferences: PreferencesConfig {
                auto_follow_logs: true,
                enable_notifications: true,
                notify_on_switch: true,
                autostart_on_login: false,
                max_concurrent_models: 3,
                checkpoint_summarizer_model: None,
            },
        };

        let serialized = toml::to_string_pretty(&config).unwrap();
        assert!(serialized.contains("max_concurrent_models"));

        let deserialized: Config = toml::from_str(&serialized).unwrap();
        assert_eq!(deserialized.max_concurrent_models(), 3);
    }

    #[test]
    fn test_max_concurrent_models_missing_uses_default() {
        // When max_concurrent_models is absent from TOML, default should be 1.
        let toml_str = r#"
schema_version = 1

[global]
log_dir = ""
proxy_port = 9080
auto_restart_on_context_full = true
"#;
        let config: Config = toml::from_str(toml_str).unwrap();
        assert_eq!(config.max_concurrent_models(), 1);
    }

    #[test]
    fn test_checkpoint_summarizer_model_default() {
        // When checkpoint_summarizer_model is absent from TOML, default should be None.
        let toml_str = r#"
schema_version = 1

[global]
log_dir = ""
proxy_port = 9080
auto_restart_on_context_full = true
"#;
        let config: Config = toml::from_str(toml_str).unwrap();
        assert_eq!(config.checkpoint_summarizer_model(), None);
    }

    #[test]
    fn test_checkpoint_summarizer_model_serialization() {
        // Verify checkpoint_summarizer_model round-trips through TOML serialization.
        let config = Config {
            schema_version: 1,
            models: vec![],
            global: GlobalSettings::default(),
            preferences: PreferencesConfig {
                auto_follow_logs: true,
                enable_notifications: true,
                notify_on_switch: true,
                autostart_on_login: false,
                max_concurrent_models: 1,
                checkpoint_summarizer_model: Some("ornith-35b".to_string()),
            },
        };

        let serialized = toml::to_string_pretty(&config).unwrap();
        assert!(serialized.contains("checkpoint_summarizer_model"));

        let deserialized: Config = toml::from_str(&serialized).unwrap();
        assert_eq!(
            deserialized.checkpoint_summarizer_model(),
            Some("ornith-35b")
        );
    }

    #[test]
    fn test_configured_models() {
        let config = Config {
            schema_version: 1,
            models: vec![
                ModelConfig {
                    id: "m1".to_string(),
                    name: "Model One".to_string(),
                    script_path: std::path::PathBuf::from("/dev/null"),
                    port: 8081,
                    health_timeout_sec: 30,
                    ctx_size: 65_536,
                },
                ModelConfig {
                    id: "m2".to_string(),
                    name: "Model Two".to_string(),
                    script_path: std::path::PathBuf::from("/dev/null"),
                    port: 8082,
                    health_timeout_sec: 30,
                    ctx_size: 65_536,
                },
            ],
            global: GlobalSettings::default(),
            preferences: PreferencesConfig::default(),
        };

        let models = config.configured_models();
        assert_eq!(models.len(), 2);
        assert_eq!(models[0], ("m1", "Model One"));
        assert_eq!(models[1], ("m2", "Model Two"));
    }
}
