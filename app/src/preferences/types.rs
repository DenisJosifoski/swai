#![allow(dead_code, unused)]
use std::path::PathBuf;

/// The values from the preferences form.
#[derive(Debug, Clone)]
pub struct PreferencesValues {
    pub log_dir: Option<PathBuf>,
    pub proxy_port: Option<u16>,
    pub auto_restart_on_context_full: bool,
    pub auto_follow_logs: bool,
    pub enable_notifications: bool,
    pub notify_on_switch: bool,
    pub autostart_on_login: bool,
    pub max_concurrent_models: usize,
    #[allow(dead_code)]
    pub checkpoint_summarizer_model: Option<String>,
}
