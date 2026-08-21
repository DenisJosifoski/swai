//! SWAI — Proxy Preferences Tab.
//!
//! Auto-restart on context full toggle with fixed 98% KV-cache watchdog threshold.


use adw::prelude::*;
use adw::{PreferencesGroup, PreferencesPage, SwitchRow};

use swai_core::config::Config;

/// Widget handles for the Proxy tab.
pub struct ProxyWidgets {
    pub auto_restart_switch: SwitchRow,
}

/// Build the Proxy preferences page.
pub fn build_proxy_tab(config: &Config) -> (PreferencesPage, ProxyWidgets) {
    let page = PreferencesPage::new();
    page.set_title("Proxy");

    let group = PreferencesGroup::new();
    group.set_title("Proxy Behavior");

    let auto_restart_switch = add_auto_restart_row(&group, config);

    page.add(&group);

    let widgets = ProxyWidgets { auto_restart_switch };

    (page, widgets)
}

/// Add the auto-restart on context full switch row.
pub fn add_auto_restart_row(parent: &PreferencesGroup, config: &Config) -> SwitchRow {
    let row = SwitchRow::builder()
        .title("Auto-restart on context full")
        .subtitle("Automatically restarts llama-server when KV-cache reaches 98% to prevent out-of-memory crashes")
        .build();

    let auto_restart = config.auto_restart_on_context_full();
    row.set_active(auto_restart);

    parent.add(&row);
    row
}
