#![allow(dead_code, unused)]
use gtk::prelude::*;
use gtk::{DropDown, Notebook, Orientation, ResponseType, SpinButton, Window};
use gtk4 as gtk;

use adw::{EntryRow, SwitchRow};

use swai_core::config::Config;

use std::sync::{Arc, Mutex};
use std::path::PathBuf;

use super::council_tab::{build_council_tab, CouncilTabState};
use super::gateway_tab::add_gateway_section;
use super::general_tab::{
    add_auto_follow_logs_row, add_auto_restart_row, add_autostart_on_login_row,
    add_enable_notifications_row, add_log_dir_row, add_max_concurrent_models_row,
    add_notify_on_switch_row, add_proxy_port_row, add_summarizer_model_row,
};
use super::types::PreferencesValues;

/// A modal dialog for editing global configuration.
#[derive(Clone)]
pub struct PreferencesDialog {
    pub widget: gtk::Dialog,
    notebook: Notebook,
    log_dir_entry: EntryRow,
    proxy_port_entry: EntryRow,
    auto_restart_switch: SwitchRow,
    auto_follow_switch: SwitchRow,
    enable_notifications_switch: SwitchRow,
    notify_on_switch_switch: SwitchRow,
    autostart_switch: SwitchRow,
    max_concurrent_spin: SpinButton,
    summarizer_model_combo: DropDown,
    council_tab_state: Arc<Mutex<CouncilTabState>>,
}

impl PreferencesDialog {
    /// Extract the selected summarizer model id from the dropdown.
    fn extract_summarizer_model(&self) -> Option<String> {
        let selected = self.summarizer_model_combo.selected();
        if selected == 0 {
            return None;
        }
        let config = swai_core::config::Config::load().ok()?;
        let idx = (selected as usize).saturating_sub(1);
        config
            .configured_models()
            .get(idx)
            .map(|(id, _)| id.to_string())
    }

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
        let autostart = self.autostart_switch.is_active();
        let max_concurrent = self.max_concurrent_spin.value() as usize;
        let summarizer_model = self.extract_summarizer_model();

        PreferencesValues {
            log_dir,
            proxy_port,
            auto_restart_on_context_full: auto_restart,
            auto_follow_logs: auto_follow,
            enable_notifications,
            notify_on_switch,
            autostart_on_login: autostart,
            max_concurrent_models: max_concurrent,
            checkpoint_summarizer_model: summarizer_model,
        }
    }

    /// Extract the council pipeline config from the tab.
    pub fn council_config(&self) -> swai_core::council::CouncilPipelineConfig {
        self.council_tab_state.lock().unwrap().to_config()
    }

    /// Create a new preferences dialog transient to the given parent window.
    pub fn new<T: IsA<Window>>(parent: &T, config: &Config) -> Self {
        let widget = gtk::Dialog::builder()
            .title("Preferences")
            .transient_for(parent)
            .modal(true)
            .build();

        // Use a notebook for tabbed interface.
        let notebook = Notebook::new();
        notebook.set_margin_start(12);
        notebook.set_margin_end(12);
        notebook.set_margin_top(12);
        notebook.set_margin_bottom(12);

        // Tab 1: General settings.
        let general_page = gtk::Box::new(Orientation::Vertical, 12);
        general_page.set_margin_start(24);
        general_page.set_margin_end(24);
        general_page.set_margin_top(24);
        general_page.set_margin_bottom(24);

        // Log directory row.
        let log_dir_entry = add_log_dir_row(&general_page, parent, config);

        // Proxy port row.
        let proxy_port_entry = add_proxy_port_row(&general_page, config);

        // Auto-restart row.
        let auto_restart_switch = add_auto_restart_row(&general_page, config);

        // Auto-follow logs row.
        let auto_follow_switch = add_auto_follow_logs_row(&general_page, config);

        // Notification settings section header.
        let notif_header = gtk::Label::builder()
            .label("<b>Notifications</b>")
            .use_markup(true)
            .halign(gtk::Align::Start)
            .margin_top(18)
            .margin_bottom(6)
            .build();
        general_page.append(&notif_header);

        // Enable notifications switch.
        let enable_notifications_switch = add_enable_notifications_row(&general_page, config);

        // Notify on switch switch.
        let notify_on_switch_switch = add_notify_on_switch_row(&general_page, config);

        // System section header.
        let system_header = gtk::Label::builder()
            .label("<b>System</b>")
            .use_markup(true)
            .halign(gtk::Align::Start)
            .margin_top(18)
            .margin_bottom(6)
            .build();
        general_page.append(&system_header);

        // Autostart on login switch.
        let autostart_switch = add_autostart_on_login_row(&general_page, config);

        // Max concurrent models spin button.
        let max_concurrent_spin = add_max_concurrent_models_row(&general_page, config);

        // Checkpoint summarizer model dropdown.
        let summarizer_model_combo = add_summarizer_model_row(&general_page, config);

        // Gateway information section (Phase 12.2).
        add_gateway_section(&general_page, config);

        let general_scrolled = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vscrollbar_policy(gtk::PolicyType::Automatic)
            .child(&general_page)
            .build();

        notebook.append_page(&general_scrolled, Some(&gtk::Label::builder().label("General").build()));

        // Tab 2: Council Pipeline.
        let council_tab_state = Arc::new(Mutex::new(CouncilTabState::new(config)));
        let council_page = build_council_tab(config, &council_tab_state);
        notebook.append_page(
            &council_page,
            Some(&gtk::Label::builder().label("Council Pipeline").build()),
        );

        widget.content_area().append(&notebook);

        widget.add_button("_Cancel", ResponseType::Cancel);
        widget.add_button("_Save", ResponseType::Ok);

        let widget_clone = widget.clone();
        widget.connect_close_request(move |_| {
            widget_clone.response(ResponseType::Cancel);
            glib::Propagation::Proceed
        });

        Self {
            widget,
            notebook,
            log_dir_entry,
            proxy_port_entry,
            auto_restart_switch,
            auto_follow_switch,
            enable_notifications_switch,
            notify_on_switch_switch,
            autostart_switch,
            max_concurrent_spin,
            summarizer_model_combo,
            council_tab_state,
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
        let autostart = self.autostart_switch.is_active();
        let max_concurrent = self.max_concurrent_spin.value() as usize;
        let summarizer_model = self.extract_summarizer_model();

        let mut config = Config::load().map_err(|e| format!("Failed to load config: {}", e))?;
        config.global.log_dir = log_dir;
        config.global.proxy_port = proxy_port;
        config.global.auto_restart_on_context_full = Some(auto_restart);
        config.global.auto_follow_logs = Some(auto_follow);
        config.preferences.enable_notifications = enable_notifications;
        config.preferences.notify_on_switch = notify_on_switch;
        config.preferences.autostart_on_login = autostart;
        config.preferences.max_concurrent_models = max_concurrent;
        config.preferences.checkpoint_summarizer_model = summarizer_model;

        // Save council pipeline config.
        config.council = self.council_config();

        Config::validate(&config, config_path)
            .map_err(|e| format!("Config validation error: {}", e))?;

        let content = toml::to_string_pretty(&config)
            .map_err(|e| format!("Failed to serialize config: {}", e))?;
        std::fs::write(config_path, &content)
            .map_err(|e| format!("Failed to write config file: {}", e))?;

        // Sync autostart state with the filesystem.
        if autostart {
            swai_core::autostart::enable_autostart()
                .map_err(|e| format!("Failed to enable autostart: {}", e))?;
        } else {
            swai_core::autostart::disable_autostart()
                .map_err(|e| format!("Failed to disable autostart: {}", e))?;
        }

        Ok(())
    }
}
