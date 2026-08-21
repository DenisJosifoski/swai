//! SWAI — Gateway Preferences Tab.
//!
//! Proxy port configuration.

use adw::prelude::*;
use adw::{EntryRow, PreferencesGroup, PreferencesPage};

use std::path::PathBuf;
use std::process::Command;
use swai_core::config::Config;

/// Widget handles for the Gateway tab.
pub struct GatewayWidgets {
    pub proxy_port_entry: EntryRow,
}

/// Copy text to the system clipboard.
pub fn copy_to_clipboard(text: &str) {
    if let Some(display) = adw::gdk::Display::default() {
        let clipboard = display.clipboard();
        clipboard.set_text(text);
        tracing::debug!("copied to clipboard: {}", text);
    }
}

/// Open the Claude CLI config file (~/.bashrc or ~/.zshrc).
pub fn open_claude_cli_config() {
    let home = std::env::var("HOME").unwrap_or_default();
    let bashrc_path = PathBuf::from(&home).join(".bashrc");
    let config_path = if bashrc_path.exists() {
        bashrc_path
    } else {
        PathBuf::from(&home).join(".zshrc")
    };
    let uri = format!("file://{}", config_path.display());
    let _ = Command::new("xdg-open").arg(&uri).spawn();
}

/// Open the Codex config file (~/.codex/config.toml).
pub fn open_codex_config() {
    let home = std::env::var("HOME").unwrap_or_default();
    let codex_dir = PathBuf::from(&home).join(".codex");
    let config_path = codex_dir.join("config.toml");

    if !config_path.exists() {
        if let Err(e) = std::fs::create_dir_all(&codex_dir) {
            tracing::warn!("failed to create ~/.codex/: {e}");
            return;
        }
        let default_block = "model_provider = \"swai\"\n\n[model_providers.swai]\nname = \"SWAI Local AI\"\nbase_url = \"http://127.0.0.1:8765/v1\"\nwire_api = \"responses\"\napi_key = \"local\"\n";
        if let Err(e) = std::fs::write(&config_path, default_block) {
            tracing::warn!("failed to write default config: {e}");
            return;
        }
        tracing::info!("created default ~/.codex/config.toml");
    }

    let uri = format!("file://{}", config_path.display());
    let _ = Command::new("xdg-open").arg(&uri).spawn();
}

/// Build the Gateway preferences page (proxy port only).
pub fn build_gateway_tab(config: &Config) -> (PreferencesPage, GatewayWidgets) {
    let page = PreferencesPage::new();
    page.set_title("Gateway");

    let group = PreferencesGroup::new();
    group.set_title("Proxy Configuration");

    let proxy_port_entry = add_proxy_port_row(&group, config);

    page.add(&group);

    let widgets = GatewayWidgets { proxy_port_entry };

    (page, widgets)
}

/// Add a proxy port entry row.
pub fn add_proxy_port_row(parent: &PreferencesGroup, config: &Config) -> EntryRow {
    let row = EntryRow::builder().title("Proxy port").build();

    let proxy_port = config.proxy_port();
    row.set_text(&proxy_port.to_string());

    parent.add(&row);
    row
}
