//! Configuration parsing and validation for SWAI.
//!
//! Loads `config.toml` from the XDG config directory, validates it, and
//! provides defaults for optional settings.

use serde::Deserialize;
use std::path::{Path, PathBuf};
use thiserror::Error;

/// Error types for configuration operations.
#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("I/O error reading config: {0}")]
    Io(#[from] std::io::Error),

    #[error("TOML parse error: {0}")]
    TomlParse(#[from] toml::de::Error),

    #[error("configuration error: {0}")]
    Validation(String),

    #[error("no config file found at any expected location")]
    NotFound,
}

impl From<toml::ser::Error> for ConfigError {
    fn from(e: toml::ser::Error) -> Self {
        ConfigError::TomlParse(toml::de::Error::custom(e.to_string()))
    }
}

// Need serde::de::Error trait in scope for `custom` method
use serde::de::Error as _;

/// A single model configuration entry.
#[derive(Debug, Clone, Deserialize, serde::Serialize)]
pub struct ModelConfig {
    pub id: String,
    pub name: String,
    #[serde(deserialize_with = "deserialize_path")]
    pub script_path: PathBuf,
    pub port: u16,
    #[serde(default = "default_health_timeout")]
    pub health_timeout_sec: u16,
}

fn default_health_timeout() -> u16 {
    30
}

fn deserialize_path<'de, D>(deserializer: D) -> Result<PathBuf, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let s = String::deserialize(deserializer)?;
    Ok(PathBuf::from(s))
}

/// Deserialize an optional PathBuf where an empty string ("") is treated as
/// None, allowing the accessor's default to kick in. This supports the
/// documented convention of `log_dir = ""` meaning "use default".
fn deserialize_optional_pathbuf<'de, D>(deserializer: D) -> Result<Option<PathBuf>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let opt = Option::<String>::deserialize(deserializer)?;
    match opt {
        Some(s) if s.is_empty() => Ok(None),  // "" → None, triggers default
        Some(s) => Ok(Some(PathBuf::from(s))),
        None => Ok(None),
    }
}

/// Preferences section — UI-only toggles that don't belong in config.toml.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct PreferencesConfig {
    /// When enabled, any open LogViewerWindow automatically switches its
    /// dropdown selection to the newly active model whenever a model switch
    /// occurs in SWAI.
    #[serde(default = "default_auto_follow_logs")]
    pub auto_follow_logs: bool,

    /// When enabled, SWAI fires desktop toast notifications on model lifecycle
    /// events (start, stop, switch, error).
    #[serde(default = "default_enable_notifications")]
    pub enable_notifications: bool,

    /// When enabled, fire a notification specifically when the active model
    /// changes. When disabled, only start/stop/error events still fire.
    #[serde(default = "default_notify_on_switch")]
    pub notify_on_switch: bool,

    /// When enabled, SWAI launches automatically on desktop session login
    /// via an XDG autostart `.desktop` file.
    #[serde(default = "default_autostart_on_login")]
    pub autostart_on_login: bool,

    /// Maximum number of model servers that may run concurrently.
    /// Range: 1–4. Default: 1 (legacy single-model behavior).
    #[serde(default = "default_max_concurrent_models")]
    pub max_concurrent_models: usize,

    /// Optional model ID to use as the checkpoint summarizer.
    /// When `None` (default), summarization is routed to the active/primary model.
    /// When set to a configured model id, that model handles summarization
    /// requests so the primary model's context is not consumed by compaction.
    #[serde(default)]
    pub checkpoint_summarizer_model: Option<String>,
}

impl Default for PreferencesConfig {
    fn default() -> Self {
        Self {
            auto_follow_logs: true,
            enable_notifications: true,
            notify_on_switch: true,
            autostart_on_login: false,
            max_concurrent_models: 1,
            checkpoint_summarizer_model: None,
        }
    }
}

fn default_auto_follow_logs() -> bool {
    true
}

fn default_enable_notifications() -> bool {
    true
}

fn default_notify_on_switch() -> bool {
    true
}

fn default_autostart_on_login() -> bool {
    false
}

fn default_max_concurrent_models() -> usize {
    1
}

#[allow(dead_code)]
fn default_checkpoint_summarizer_model() -> Option<String> {
    None
}

/// Global settings section.
#[derive(Debug, Clone, Deserialize, serde::Serialize)]
pub struct GlobalSettings {
  #[serde(default, deserialize_with = "deserialize_optional_pathbuf")]
    pub log_dir: Option<PathBuf>,
    #[serde(default)]
    pub proxy_port: Option<u16>,
    #[serde(default)]
    pub auto_restart_on_context_full: Option<bool>,
    #[serde(default)]
    pub auto_follow_logs: Option<bool>,
}

impl Default for GlobalSettings {
    fn default() -> Self {
        Self {
            log_dir: None,
            proxy_port: None,
            auto_restart_on_context_full: None,
            auto_follow_logs: Some(true),
        }
    }
}

/// Root configuration.
#[derive(Debug, Clone, Deserialize, serde::Serialize)]
pub struct Config {
    pub schema_version: u8,
    #[serde(default)]
    pub models: Vec<ModelConfig>,
    #[serde(default)]
    pub global: GlobalSettings,
    #[serde(default)]
    pub preferences: PreferencesConfig,
}

impl Config {
    /// Resolve the config file path from XDG directories.
    pub fn resolve_path() -> Option<PathBuf> {
        // Try $XDG_CONFIG_HOME/swai/config.toml first
        if let Ok(xdg) = std::env::var("XDG_CONFIG_HOME") {
            let p = PathBuf::from(&xdg).join("swai").join("config.toml");
            if p.exists() {
                return Some(p);
            }
        }

        // Fallback to ~/.config/swai/config.toml
        if let Ok(home) = std::env::var("HOME") {
            let p = PathBuf::from(&home).join(".config").join("swai").join("config.toml");
            if p.exists() {
                return Some(p);
            }
        }

        None
    }

    /// Load and validate configuration from the resolved path.
    pub fn load() -> Result<Self, ConfigError> {
        let path = Self::resolve_path().ok_or(ConfigError::NotFound)?;
        let content = std::fs::read_to_string(&path)?;

        let raw: Self = toml::from_str(&content).map_err(ConfigError::TomlParse)?;
        Self::validate(&raw, &path)
    }

    /// Validate a loaded config.
    pub fn validate(config: &Self, _config_path: &Path) -> Result<Self, ConfigError> {
        let mut seen_ports: std::collections::HashSet<u16> =
            std::collections::HashSet::new();
        for model in &config.models {
            if !seen_ports.insert(model.port) {
                return Err(ConfigError::Validation(format!(
                    "duplicate port {} for models '{}' and '{}'",
                    model.port,
                    config
                        .models
                        .iter()
                        .find(|m| m.port == model.port && m.id != model.id)
                        .map(|m| &m.id)
                        .unwrap_or(&model.id),
                    model.id
                )));
            }
        }

        for model in &config.models {
            if !model.script_path.exists() {
                return Err(ConfigError::Validation(format!(
                    "script not found: {} (configured for model '{}')",
                    model.script_path.display(),
                    model.id,
                )));
            }
        }

        Ok(config.clone())
    }

    /// Get the default log directory.
    pub fn default_log_dir() -> PathBuf {
        if let Ok(home) = std::env::var("HOME") {
            PathBuf::from(home)
                .join(".local/share/swai/logs/")
        } else {
            PathBuf::from("logs/")
        }
    }

    /// Get the default proxy port.
    pub fn default_proxy_port() -> u16 {
        9080
    }

    /// Get the default auto-restart setting.
    pub fn default_auto_restart_on_context_full() -> bool {
        true
    }

    /// Get the effective log directory.
    pub fn log_dir(&self) -> PathBuf {
        self.global
            .log_dir
            .clone()
            .unwrap_or_else(Self::default_log_dir)
    }

    /// Get the effective proxy port.
    pub fn proxy_port(&self) -> u16 {
        self.global.proxy_port.unwrap_or_else(Self::default_proxy_port)
    }

    /// Get the effective auto-restart setting.
    pub fn auto_restart_on_context_full(&self) -> bool {
        self.global
            .auto_restart_on_context_full
            .unwrap_or_else(Self::default_auto_restart_on_context_full)
    }

    /// Get the effective auto-follow-logs preference.
    pub fn auto_follow_logs(&self) -> bool {
        self.preferences.auto_follow_logs
    }

    /// Get the effective enable-notifications preference.
    pub fn enable_notifications(&self) -> bool {
        self.preferences.enable_notifications
    }

    /// Get the effective notify-on-switch preference.
    pub fn notify_on_switch(&self) -> bool {
        self.preferences.notify_on_switch
    }

    /// Get the effective autostart-on-login preference.
    pub fn autostart_on_login(&self) -> bool {
        self.preferences.autostart_on_login
    }

    /// Get the effective max-concurrent-models preference.
    pub fn max_concurrent_models(&self) -> usize {
        self.preferences.max_concurrent_models
    }

    /// Get the configured checkpoint summarizer model id (if any).
    ///
    /// When `None`, summarization is routed to the active/primary model.
    /// When set, that specific model handles summarization requests so the
    /// primary model's context is not consumed by compaction overhead.
    pub fn checkpoint_summarizer_model(&self) -> Option<&str> {
        self.preferences.checkpoint_summarizer_model.as_deref()
    }

    /// Get all configured models as a list of (id, name) pairs.
    ///
    /// Used by the Preferences UI to populate dropdown selectors that let
    /// users choose a model for a specific role (e.g., checkpoint summarizer).
    pub fn configured_models(&self) -> Vec<(&str, &str)> {
        self.models.iter().map(|m| (m.id.as_str(), m.name.as_str())).collect()
    }
}

/// Returns an example config.toml for first-run reference.
pub fn example_config() -> &'static str {
    include_str!("../../config.toml.example")
}

#[cfg(test)]
mod tests {
    use super::*;
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
    fn make_temp_config(
        tmp: &tempfile::TempDir,
        models: &[(&str, &str, u16)],
    ) -> PathBuf {
        let config_path = tmp.path().join("config.toml");
        let mut content = String::new();
        content.push_str("[models]\n");
        for (i, (id, name, port)) in models.iter().enumerate() {
            content.push_str(&format!(
                "[models.model_{}]\nid = \"{}\"\nname = \"{}\"\nscript_path = \"{}\"\nport = {}\nschema_version = 1\n",
                i, id, name, tmp.path().join(format!("{}.sh", id)).display(), port
            ));
        }
        content.push_str("[global]\nlog_dir = \"\"\nproxy_port = 8080\nauto_restart_on_context_full = true\n");
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
                },
                ModelConfig {
                    id: "m2".to_string(),
                    name: "Model 2".to_string(),
                    script_path: script2.clone(),
                    port: 8081, // duplicate
                    health_timeout_sec: 30,
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
        assert_eq!(deserialized.checkpoint_summarizer_model(), Some("ornith-35b"));
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
                },
                ModelConfig {
                    id: "m2".to_string(),
                    name: "Model Two".to_string(),
                    script_path: std::path::PathBuf::from("/dev/null"),
                    port: 8082,
                    health_timeout_sec: 30,
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
