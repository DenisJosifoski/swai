use super::model::deserialize_optional_pathbuf;
use serde::Deserialize;
use std::path::PathBuf;

/// Preferences section — UI-only toggles that don't belong in config.toml.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct PreferencesConfig {
    #[serde(default = "default_auto_follow_logs")]
    pub auto_follow_logs: bool,

    #[serde(default = "default_enable_notifications")]
    pub enable_notifications: bool,

    #[serde(default = "default_notify_on_switch")]
    pub notify_on_switch: bool,

    #[serde(default = "default_autostart_on_login")]
    pub autostart_on_login: bool,

    #[serde(default = "default_max_concurrent_models")]
    pub max_concurrent_models: usize,

    #[serde(default)]
    pub checkpoint_summarizer_model: Option<String>,

    #[serde(default = "default_enable_checkpointing")]
    pub enable_checkpointing: bool,

    #[serde(default = "default_enable_council")]
    pub enable_council: bool,

    /// Compaction trigger threshold as a percentage of the model's context window.
    /// Range: 50% (aggressive compaction) to 85% (retains more history).
    /// Default: 70%.
    #[serde(default = "default_compaction_threshold_pct")]
    pub compaction_threshold_pct: u8,
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
            enable_checkpointing: true,
            enable_council: true,
            compaction_threshold_pct: crate::compaction::DEFAULT_THRESHOLD_PCT,
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

fn default_enable_checkpointing() -> bool {
    true
}

fn default_enable_council() -> bool {
    true
}

fn default_compaction_threshold_pct() -> u8 {
    crate::compaction::DEFAULT_THRESHOLD_PCT
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
