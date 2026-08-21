//! SWAI — Guides Preferences Tab.
//!
//! Read-only reference material for connecting external clients to SWAI.

use adw::prelude::*;
use adw::PreferencesGroup;
use adw::PreferencesPage;

use swai_core::config::Config;

use super::client_expanders::{
    build_claude_cli_expander, build_claude_desktop_expander, build_codex_expander,
};

/// Widget handles for the Guides tab (none needed - read-only).
pub struct GuidesWidgets;

/// Build the Guides preferences page.
pub fn build_guides_tab(config: &Config) -> (PreferencesPage, GuidesWidgets) {
    let page = PreferencesPage::new();
    page.set_title("Guides");

    let group = PreferencesGroup::new();
    group.set_title("External Client Guides");

    let claude_cli_expander = build_claude_cli_expander(config);
    let claude_desktop_expander = build_claude_desktop_expander(config);
    let codex_expander = build_codex_expander(config);

    group.add(&claude_cli_expander);
    group.add(&claude_desktop_expander);
    group.add(&codex_expander);

    page.add(&group);

    let widgets = GuidesWidgets;

    (page, widgets)
}
