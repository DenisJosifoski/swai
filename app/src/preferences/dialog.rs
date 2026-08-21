#![allow(dead_code, unused)]
use gtk::prelude::*;
use gtk::{DropDown, Orientation, ResponseType, SpinButton, Window};
use gtk4 as gtk;

use adw::{EntryRow, SwitchRow};

use swai_core::config::Config;

use std::sync::{Arc, Mutex};
use std::path::PathBuf;

use super::checkpoint_tab::{build_checkpoint_tab, CheckpointWidgets};
use super::council_tab::{build_council_tab, CouncilTabState};
use super::general_tab::{build_general_tab, GeneralWidgets};
use super::gateway_tab::{build_gateway_tab, GatewayWidgets};
use super::guides_tab::{build_guides_tab, GuidesWidgets};
use super::notifications_tab::{build_notifications_tab, NotificationsWidgets};
use super::proxy_tab::{build_proxy_tab, ProxyWidgets};
use super::types::PreferencesValues;

/// A modal dialog for editing global configuration with sidebar navigation.
#[derive(Clone)]
pub struct PreferencesDialog {
    pub widget: gtk::Dialog,
    log_dir_entry: EntryRow,
    proxy_port_entry: EntryRow,
    auto_restart_switch: SwitchRow,
    auto_follow_switch: SwitchRow,
    enable_notifications_switch: SwitchRow,
    notify_on_switch_switch: SwitchRow,
    autostart_switch: SwitchRow,
    max_concurrent_spin: SpinButton,
    summarizer_model_combo: DropDown,
    enable_checkpointing_switch: SwitchRow,
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
        let enable_checkpointing = self.enable_checkpointing_switch.is_active();

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
            enable_checkpointing,
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
            .default_width(780)
            .default_height(560)
            .build();

        // Build sidebar with gtk::Stack and gtk::StackSidebar
        let stack = gtk::Stack::new();
        stack.set_transition_type(gtk::StackTransitionType::Crossfade);
        stack.set_transition_duration(150);
        stack.set_hexpand(true);
        stack.set_vexpand(true);

        let sidebar = gtk::StackSidebar::new();
        sidebar.set_stack(&stack);
        sidebar.set_size_request(190, -1);

        let separator = gtk::Separator::new(Orientation::Vertical);

        // Build all preference pages and add to stack
        let (general_page, general_widgets) = build_general_tab(config);
        let (gateway_page, gateway_widgets) = build_gateway_tab(config);
        let (proxy_page, proxy_widgets) = build_proxy_tab(config);
        let (checkpoint_page, checkpoint_widgets) = build_checkpoint_tab(config);
        let (notifications_page, notifications_widgets) = build_notifications_tab(config);

        let council_tab_state = Arc::new(Mutex::new(CouncilTabState::new(config)));
        let council_page = build_council_tab(config, &council_tab_state);

        let (guides_page, _) = build_guides_tab(config);

        // Add pages to stack with titles
        stack.add_titled(&general_page, Some("general"), "General");
        stack.add_titled(&gateway_page, Some("gateway"), "Gateway");
        stack.add_titled(&proxy_page, Some("proxy"), "Proxy");
        stack.add_titled(&checkpoint_page, Some("checkpoint"), "Checkpointing");
        stack.add_titled(&notifications_page, Some("notifications"), "Notifications");
        stack.add_titled(&council_page, Some("council"), "Council Pipeline");
        stack.add_titled(&guides_page, Some("guides"), "Guides");

        let scrolled_stack = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vscrollbar_policy(gtk::PolicyType::Automatic)
            .child(&stack)
            .hexpand(true)
            .vexpand(true)
            .build();

        // Layout: sidebar on left, separator in middle, scrollable stack on right
        let main_box = gtk::Box::new(Orientation::Horizontal, 0);
        main_box.set_hexpand(true);
        main_box.set_vexpand(true);
        main_box.append(&sidebar);
        main_box.append(&separator);
        main_box.append(&scrolled_stack);

        widget.content_area().append(&main_box);

        widget.add_button("_Cancel", ResponseType::Cancel);
        widget.add_button("_Save", ResponseType::Ok);

        let widget_clone = widget.clone();
        widget.connect_close_request(move |_| {
            widget_clone.response(ResponseType::Cancel);
            glib::Propagation::Proceed
        });

        // Extract real widget handles from the tab builders
        Self {
            widget,
            log_dir_entry: general_widgets.log_dir_entry,
            proxy_port_entry: gateway_widgets.proxy_port_entry,
            auto_restart_switch: proxy_widgets.auto_restart_switch,
            auto_follow_switch: notifications_widgets.auto_follow_switch,
            enable_notifications_switch: notifications_widgets.enable_notifications_switch,
            notify_on_switch_switch: notifications_widgets.notify_on_switch_switch,
            autostart_switch: general_widgets.autostart_switch,
            max_concurrent_spin: general_widgets.max_concurrent_spin,
            summarizer_model_combo: checkpoint_widgets.summarizer_model_combo,
            enable_checkpointing_switch: checkpoint_widgets.enable_checkpointing_switch,
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
        let enable_checkpointing = self.enable_checkpointing_switch.is_active();

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
        config.preferences.enable_checkpointing = enable_checkpointing;

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
