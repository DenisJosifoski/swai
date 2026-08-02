//! Preferences dialog for editing global settings and viewing Gateway configuration info.

use gtk4 as gtk;
use gtk::prelude::*;
use gtk::{FileChooserAction, Orientation, ResponseType, Window};

use adw::prelude::*;
use adw::{EntryRow, SwitchRow};

use swai_core::config::Config;

use std::path::PathBuf;

/// A modal dialog for editing global configuration.
#[derive(Clone)]
pub struct PreferencesDialog {
    pub widget: gtk::Dialog,
    log_dir_entry: EntryRow,
    proxy_port_entry: EntryRow,
    auto_restart_switch: SwitchRow,
    auto_follow_switch: SwitchRow,
    enable_notifications_switch: SwitchRow,
    notify_on_switch_switch: SwitchRow,
}

/// The values from the preferences form.
pub struct PreferencesValues {
    pub log_dir: Option<PathBuf>,
    pub proxy_port: Option<u16>,
    pub auto_restart_on_context_full: bool,
    pub auto_follow_logs: bool,
    pub enable_notifications: bool,
    pub notify_on_switch: bool,
}

impl PreferencesDialog {
    /// Extract the current form values as a serializable struct.
    pub fn values(&self) -> PreferencesValues {
        let log_dir_text = self.log_dir_entry.text();
        let log_dir = if log_dir_text.is_empty() {
            None
        } else {
            Some(PathBuf::from(log_dir_text.as_str()))
        };

        let proxy_port_text = self.proxy_port_entry.text();
        let proxy_port: Option<u16> = proxy_port_text.parse().ok();

        let auto_restart = self.auto_restart_switch.is_active();
        let auto_follow = self.auto_follow_switch.is_active();
        let enable_notifications = self.enable_notifications_switch.is_active();
        let notify_on_switch = self.notify_on_switch_switch.is_active();

        PreferencesValues {
            log_dir,
            proxy_port,
            auto_restart_on_context_full: auto_restart,
            auto_follow_logs: auto_follow,
            enable_notifications,
            notify_on_switch,
        }
    }

    /// Create a new preferences dialog transient to the given parent window.
    pub fn new<T: IsA<Window>>(parent: &T, config: &Config) -> Self {
        let widget = gtk::Dialog::builder()
            .title("Preferences")
            .transient_for(parent)
            .modal(true)
            .build();

        let content_box = gtk::Box::new(Orientation::Vertical, 12);
        content_box.set_margin_start(24);
        content_box.set_margin_end(24);
        content_box.set_margin_top(24);
        content_box.set_margin_bottom(24);

        // Log directory row.
        let log_dir_entry = Self::add_log_dir_row(&content_box, parent, config);

        // Proxy port row.
        let proxy_port_entry = Self::add_proxy_port_row(&content_box, config);

        // Auto-restart row.
        let auto_restart_switch = Self::add_auto_restart_row(&content_box, config);

        // Auto-follow logs row.
        let auto_follow_switch = Self::add_auto_follow_logs_row(&content_box, config);

        // Notification settings section header.
        let notif_header = gtk::Label::builder()
            .label("<b>Notifications</b>")
            .use_markup(true)
            .halign(gtk::Align::Start)
            .margin_top(18)
            .margin_bottom(6)
            .build();
        content_box.append(&notif_header);

        // Enable notifications switch.
        let enable_notifications_switch = Self::add_enable_notifications_row(&content_box, config);

        // Notify on switch switch.
        let notify_on_switch_switch = Self::add_notify_on_switch_row(&content_box, config);

        // Gateway information section (Phase 12.2).
        Self::add_gateway_section(&content_box, config);

        widget.content_area().append(&content_box);

        widget.add_button("_Cancel", ResponseType::Cancel);
        widget.add_button("_Save", ResponseType::Ok);

        Self {
            widget,
            log_dir_entry,
            proxy_port_entry,
            auto_restart_switch,
            auto_follow_switch,
            enable_notifications_switch,
            notify_on_switch_switch,
        }
    }

    /// Destroy the dialog window.
    #[allow(dead_code)]
    pub fn destroy(&self) {
        self.widget.destroy();
    }

    /// Read current values and save back to the config file.
    #[allow(dead_code)]
    pub fn save(&self, config_path: &std::path::Path) -> Result<(), String> {
        let log_dir_text = self.log_dir_entry.text();
        let log_dir = if log_dir_text.is_empty() {
            None
        } else {
            Some(PathBuf::from(log_dir_text.as_str()))
        };

        let proxy_port_text = self.proxy_port_entry.text();
        let proxy_port: Option<u16> = proxy_port_text.parse().ok();

        let auto_restart = self.auto_restart_switch.is_active();
        let auto_follow = self.auto_follow_switch.is_active();
        let enable_notifications = self.enable_notifications_switch.is_active();
        let notify_on_switch = self.notify_on_switch_switch.is_active();

        let mut config = Config::load().map_err(|e| format!("Failed to load config: {}", e))?;
        config.global.log_dir = log_dir;
        config.global.proxy_port = proxy_port;
        config.global.auto_restart_on_context_full = Some(auto_restart);
        config.global.auto_follow_logs = Some(auto_follow);
        config.preferences.enable_notifications = enable_notifications;
        config.preferences.notify_on_switch = notify_on_switch;

        Config::validate(&config, config_path).map_err(|e| format!("Config validation error: {}", e))?;

        let content = toml::to_string_pretty(&config)
            .map_err(|e| format!("Failed to serialize config: {}", e))?;
        std::fs::write(config_path, &content)
            .map_err(|e| format!("Failed to write config file: {}", e))?;

        Ok(())
    }

    fn add_log_dir_row(parent: &impl IsA<gtk::Box>, dialog_parent: &impl IsA<gtk::Window>, config: &Config) -> EntryRow {
        let row = EntryRow::builder()
            .title("Log directory")
            .build();

        let current_path = config.log_dir();
        row.set_text(current_path.to_string_lossy().as_ref());

        // Add a Browse button to the end of the row.
        let entry_clone = row.clone();
        let dialog_parent_clone = dialog_parent.clone();
        let browse_btn = gtk::Button::builder()
            .label("Browse…")
            .css_classes(vec!["flat"])
            .build();
        browse_btn.connect_clicked(move |_| {
            Self::show_folder_chooser(&entry_clone, &dialog_parent_clone);
        });
        row.add_suffix(&browse_btn);

        // Append the completed row to the dialog's content box.
        parent.as_ref().append(&row);
        row
    }

    /// Show a folder chooser dialog using the async run_async pattern.
    fn show_folder_chooser<T: IsA<Window>>(entry: &EntryRow, parent: &T) {
        let chooser = gtk::FileChooserDialog::new(
            Some("Select Log Directory"),
            Some(parent),
            FileChooserAction::SelectFolder,
            &[("_Cancel", ResponseType::Cancel), ("_Select", ResponseType::Ok)],
        );

        if let Ok(path) = std::env::var("HOME") {
            let _ = chooser.set_current_folder(Some(&gio::File::for_path(PathBuf::from(path))));
        }

        let entry_clone = entry.clone();
        chooser.run_async(move |chooser, response| {
            if response == ResponseType::Ok {
                if let Some(folder) = chooser.current_folder() {
                    if let Some(path) = folder.path() {
                        entry_clone.set_text(&path.to_string_lossy());
                    }
                }
            }
            chooser.destroy();
        });
    }

    fn add_proxy_port_row(parent: &gtk::Box, config: &Config) -> EntryRow {
        let row = EntryRow::builder()
            .title("Proxy port")
            .build();

        let proxy_port = config.proxy_port();
        row.set_text(&proxy_port.to_string());

        parent.append(&row);
        row
    }

    fn add_auto_restart_row(parent: &gtk::Box, config: &Config) -> SwitchRow {
        let row = SwitchRow::builder()
            .title("Auto-restart on context full")
            .build();

        let auto_restart = config.auto_restart_on_context_full();
        row.set_active(auto_restart);

        parent.append(&row);
        row
    }

    fn add_auto_follow_logs_row(parent: &gtk::Box, config: &Config) -> SwitchRow {
        let row = SwitchRow::builder()
            .title("Auto-follow active model in logs")
            .build();

        let auto_follow = config.auto_follow_logs();
        row.set_active(auto_follow);

        parent.append(&row);
        row
    }

    fn add_enable_notifications_row(parent: &gtk::Box, config: &Config) -> SwitchRow {
        let row = SwitchRow::builder()
            .title("Enable desktop notifications")
            .build();

        let enable = config.enable_notifications();
        row.set_active(enable);

        parent.append(&row);
        row
    }

    fn add_notify_on_switch_row(parent: &gtk::Box, config: &Config) -> SwitchRow {
        let row = SwitchRow::builder()
            .title("Notify on model switch")
            .build();

        let notify = config.notify_on_switch();
        row.set_active(notify);

        parent.append(&row);
        row
    }

    // ─── Gateway Information Section (Phase 12.2) ───────────────────────────

    /// Add the Gateway information section at the bottom of the preferences dialog.
    ///
    /// Displays the proxy base URL with a copy button, auth key guidance, and
    /// step-by-step instructions for connecting Claude Desktop via the
    /// Third-Party Inference → Gateway mode.
    fn add_gateway_section(parent: &gtk::Box, config: &Config) {
        // Section header label.
        let header = gtk::Label::builder()
            .label("<b>Gateway</b>")
            .use_markup(true)
            .halign(gtk::Align::Start)
            .margin_top(18)
            .margin_bottom(6)
            .build();
        parent.append(&header);

        // Base URL row: shows http://127.0.0.1:{proxy_port}/v1 with a copy button.
        let url_row = gtk::Box::new(Orientation::Horizontal, 6);
        url_row.set_margin_start(6);
        url_row.set_margin_end(6);

        let url_label = gtk::Label::builder()
            .label(format!("http://127.0.0.1:{}/v1", config.proxy_port()))
            .xalign(0.0)
            .selectable(true)
            .build();
        url_row.append(&url_label);

        let copy_btn = gtk::Button::builder()
            .label("Copy")
            .css_classes(vec!["flat"])
            .build();
        let url_text = format!("http://127.0.0.1:{}/v1", config.proxy_port());
        copy_btn.connect_clicked(move |_| {
            Self::copy_to_clipboard(&url_text);
        });
        url_row.append(&copy_btn);

        parent.append(&url_row);

        // Auth key guidance row.
        let auth_row = gtk::Box::new(Orientation::Horizontal, 6);
        auth_row.set_margin_start(6);
        auth_row.set_margin_end(6);

        let auth_label = gtk::Label::builder()
            .label("Gateway API Key:")
            .halign(gtk::Align::Start)
            .build();
        auth_row.append(&auth_label);

        let auth_value = gtk::Label::builder()
            .label("local")
            .css_classes(vec!["dim-label"])
            .xalign(0.0)
            .build();
        auth_row.append(&auth_value);

        parent.append(&auth_row);

        // Setup instructions for Claude Desktop (Third-Party Inference → Gateway).
        let instructions_label = gtk::Label::builder()
            .label(
                "To connect Claude Desktop:\n\
                 1. Enable Developer Mode in Claude Desktop's Help menu.\n\
                 2. Open Developer Menu → Configure Third-Party Inference → Gateway.\n\
                 3. Set Gateway Base URL to http://127.0.0.1:9080/ and API Key to \"local\".\n\
                 4. Under Models → Model list, click \"+ Add model\": set Model ID to \"claude\" and Display name to \"SWAI\". Toggle 1M-context ON.",
            )
            .use_markup(false)
            .wrap(true)
            .xalign(0.0)
            .margin_start(6)
            .margin_end(6)
            .margin_top(6)
            .build();
        parent.append(&instructions_label);
    }

    /// Copy text to the system clipboard using GDK.
    ///
    /// Uses `adw::gdk::Display::default().clipboard().set_text(...)` —
    /// GTK's native clipboard API. Works on both X11 and Wayland.
    fn copy_to_clipboard(text: &str) {
        if let Some(display) = adw::gdk::Display::default() {
            let clipboard = display.clipboard();
            clipboard.set_text(text);
            tracing::debug!("copied to clipboard: {}", text);
        }
    }
}
