//! SWAI — Proxy Preferences Tab.
//!
//! Network routing and process context watchdog settings.

use adw::prelude::*;
use adw::{EntryRow, PreferencesGroup, PreferencesPage, SwitchRow};

use swai_core::config::Config;

/// Widget handles for the Proxy tab.
pub struct ProxyWidgets {
    pub proxy_port_entry: EntryRow,
    pub auto_restart_switch: SwitchRow,
}

/// Build the Proxy preferences page.
pub fn build_proxy_tab(config: &Config) -> (PreferencesPage, ProxyWidgets) {
    let page = PreferencesPage::new();
    page.set_title("Proxy");

    // Network & Port group
    let network_group = PreferencesGroup::new();
    network_group.set_title("Network &amp; Routing");

    let proxy_port_entry = add_proxy_port_row(&network_group, config);
    page.add(&network_group);

    // Watchdog group
    let watchdog_group = PreferencesGroup::new();
    watchdog_group.set_title("Process &amp; Context Watchdog");

    let auto_restart_switch = add_auto_restart_row(&watchdog_group, config);
    page.add(&watchdog_group);

    let widgets = ProxyWidgets {
        proxy_port_entry,
        auto_restart_switch,
    };

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
