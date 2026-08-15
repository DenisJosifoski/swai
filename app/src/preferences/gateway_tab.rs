use gtk4 as gtk;
use gtk::prelude::*;
use gtk::Orientation;

use adw::prelude::*;
use adw::ExpanderRow;

use swai_core::config::Config;
use std::path::PathBuf;
use std::process::Command;

use super::client_expanders::{build_claude_cli_expander, build_claude_desktop_expander, build_codex_expander};

pub fn copy_to_clipboard(text: &str) {
    if let Some(display) = adw::gdk::Display::default() {
        let clipboard = display.clipboard();
        clipboard.set_text(text);
        tracing::debug!("copied to clipboard: {}", text);
    }
}

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

pub fn add_gateway_section(parent: &gtk::Box, config: &Config) {
    let port = config.proxy_port();
    let base_url = format!("http://127.0.0.1:{port}/v1", port = port);

    let gateway_header = gtk::Label::builder()
        .label("<b>Gateway Information</b>")
        .use_markup(true)
        .halign(gtk::Align::Start)
        .margin_top(18)
        .margin_bottom(6)
        .build();
    parent.append(&gateway_header);

    let url_row = gtk::Box::new(Orientation::Horizontal, 8);
    url_row.set_margin_top(4);
    url_row.set_margin_bottom(4);

    let url_label = gtk::Label::builder()
        .label("Proxy Base URL:")
        .halign(gtk::Align::Start)
        .css_classes(vec!["dim-label"])
        .build();
    url_row.append(&url_label);

    let url_value = gtk::Label::builder()
        .label(&base_url)
        .halign(gtk::Align::Start)
        .selectable(true)
        .build();
    url_row.append(&url_value);

    let copy_btn = gtk::Button::builder()
        .label("Copy URL")
        .css_classes(vec!["flat", "suggested-action"])
        .build();
    let url_text = base_url.clone();
    copy_btn.connect_clicked(move |_| {
        copy_to_clipboard(&url_text);
    });
    url_row.append(&copy_btn);

    parent.append(&url_row);

    let accordion_box = gtk::Box::new(Orientation::Vertical, 6);
    accordion_box.set_margin_top(8);

    let claude_cli_expander = build_claude_cli_expander(config);
    let claude_desktop_expander = build_claude_desktop_expander(config);
    let codex_expander = build_codex_expander(config);

    accordion_box.append(&claude_cli_expander);
    accordion_box.append(&claude_desktop_expander);
    accordion_box.append(&codex_expander);

    let expander_vec: Vec<ExpanderRow> = vec![
        claude_cli_expander.clone(),
        claude_desktop_expander.clone(),
        codex_expander.clone(),
    ];
    wire_accordion_exclusion(&accordion_box, &expander_vec);

    parent.append(&accordion_box);
}

fn wire_accordion_exclusion(container: &gtk::Box, expanders: &[ExpanderRow]) {
    for expander in expanders {
        let siblings: Vec<ExpanderRow> = expanders.to_vec();
        expander.connect_notify_local(Some("expanded"), move |widget, _param| {
            let is_expanded: bool = widget.property("expanded");
            if !is_expanded {
                return;
            }
            let siblings_clone = siblings.clone();
            let widget_gptr = widget.as_ptr();
            glib::idle_add_local(move || {
                for sibling in &siblings_clone {
                    if sibling.as_ptr() != widget_gptr {
                        sibling.set_expanded(false);
                    }
                }
                glib::ControlFlow::Break
            });
        });
    }
    container.connect_destroy(move |_| {});
}
