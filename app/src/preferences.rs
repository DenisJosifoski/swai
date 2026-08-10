//! Preferences dialog for editing global settings and viewing Gateway configuration info.

use gtk4 as gtk;
use gtk::prelude::*;
use gtk::{FileChooserAction, Orientation, ResponseType, Window};

use adw::prelude::*;
use adw::{EntryRow, ExpanderRow, SwitchRow};

use swai_core::config::Config;

use std::path::PathBuf;
use std::process::Command;

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
    autostart_switch: SwitchRow,
}

/// The values from the preferences form.
#[allow(dead_code)]
pub struct PreferencesValues {
    pub log_dir: Option<PathBuf>,
    pub proxy_port: Option<u16>,
    pub auto_restart_on_context_full: bool,
    pub auto_follow_logs: bool,
    pub enable_notifications: bool,
    pub notify_on_switch: bool,
    pub autostart_on_login: bool,
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
        let autostart = self.autostart_switch.is_active();

        PreferencesValues {
            log_dir,
            proxy_port,
            auto_restart_on_context_full: auto_restart,
            auto_follow_logs: auto_follow,
            enable_notifications,
            notify_on_switch,
            autostart_on_login: autostart,
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

        // System section header.
        let system_header = gtk::Label::builder()
            .label("<b>System</b>")
            .use_markup(true)
            .halign(gtk::Align::Start)
            .margin_top(18)
            .margin_bottom(6)
            .build();
        content_box.append(&system_header);

        // Autostart on login switch.
        let autostart_switch = Self::add_autostart_on_login_row(&content_box, config);

        // Phase 20: "Check for Updates" button.
        let check_update_btn = gtk::Button::builder()
            .label("Check for Updates…")
            .css_classes(vec!["suggested-action"])
            .halign(gtk::Align::End)
            .margin_top(12)
            .build();

        let parent_win = widget.clone();
        check_update_btn.connect_clicked(move |btn| {
            btn.set_sensitive(false);
            btn.set_label("Checking…");

            // Perform update check synchronously (quick HTTP request).
            let result = crate::update_checker::check_for_updates_blocking(
                "DenisJosifoski/swai",
                env!("CARGO_PKG_VERSION"),
            );

            match result {
                crate::update_checker::UpdateCheckResult::UpdateAvailable { version, .. } => {
                    let dlg = gtk::MessageDialog::new(
                        Some(&parent_win),
                        gtk::DialogFlags::MODAL,
                        gtk::MessageType::Info,
                        gtk::ButtonsType::None,
                        &format!(
                            "SWAI v{} is available!\n\n\
                             Would you like to download and install it?",
                            version,
                        ),
                    );
                    dlg.set_title(Some("SWAI - Update Available"));
                    dlg.add_button("_Later", ResponseType::Cancel);
                    dlg.add_button("_Download & Install", ResponseType::Ok);

                    let dialog_parent = parent_win.clone();
                    dlg.connect_response(move |d, response| {
                        if response == ResponseType::Ok {
                            let install_result =
                                crate::update_installer::install_update(
                                    "DenisJosifoski/swai",
                                    &version,
                                );
                            match install_result {
                                crate::update_installer::UpdateInstallResult::Success {
                                    new_version,
                                } => {
                                    let notif = gtk::MessageDialog::new(
                                        Some(&dialog_parent),
                                        gtk::DialogFlags::MODAL,
                                        gtk::MessageType::Info,
                                        gtk::ButtonsType::None,
                                        &format!(
                                            "SWAI updated to v{} successfully!\n\n\
                                             Click 'Restart Now' to apply the update.",
                                            new_version,
                                        ),
                                    );
                                    notif.set_title(Some("SWAI - Update Complete"));
                                    notif.add_button("_Later", ResponseType::Cancel);
                                    notif.add_button("_Restart Now", ResponseType::Ok);

                                    notif.connect_response(|n, response| {
                                        if response == ResponseType::Ok {
                                            if let Ok(exe) = std::env::current_exe() {
                                                let _ = std::process::Command::new(exe).spawn();
                                            } else {
                                                let _ = std::process::Command::new("swai").spawn();
                                            }
                                            std::process::exit(0);
                                        }
                                        n.destroy();
                                    });
                                    notif.present();
                                }
                                crate::update_installer::UpdateInstallResult::Error(e) => {
                                    let err = gtk::MessageDialog::new(
                                        Some(&dialog_parent),
                                        gtk::DialogFlags::MODAL,
                                        gtk::MessageType::Error,
                                        gtk::ButtonsType::Close,
                                        &format!("Update failed:\n\n{}", e),
                                    );
                                    err.set_title(Some("SWAI - Update Error"));
                                    err.connect_response(|e_dlg, _| e_dlg.destroy());
                                    err.present();
                                }
                            }
                        }
                        d.destroy();
                    });
                    dlg.present();
                }
                crate::update_checker::UpdateCheckResult::NoUpdate => {
                    let dlg = gtk::MessageDialog::new(
                        Some(&parent_win),
                        gtk::DialogFlags::MODAL,
                        gtk::MessageType::Info,
                        gtk::ButtonsType::Close,
                        "You are running the latest version of SWAI.",
                    );
                    dlg.set_title(Some("SWAI - Up to Date"));
                    dlg.connect_response(|d, _| d.destroy());
                    dlg.present();
                }
                crate::update_checker::UpdateCheckResult::Error(e) => {
                    let dlg = gtk::MessageDialog::new(
                        Some(&parent_win),
                        gtk::DialogFlags::MODAL,
                        gtk::MessageType::Error,
                        gtk::ButtonsType::Close,
                        &format!("Failed to check for updates:\n\n{}", e),
                    );
                    dlg.set_title(Some("SWAI - Update Check Failed"));
                    dlg.connect_response(|d, _| d.destroy());
                    dlg.present();
                }
            }

            btn.set_sensitive(true);
            btn.set_label("Check for Updates…");
        });
        content_box.append(&check_update_btn);

        // Gateway information section (Phase 12.2).
        Self::add_gateway_section(&content_box, config);

        widget.content_area().append(&content_box);

        widget.add_button("_Cancel", ResponseType::Cancel);
        widget.add_button("_Save", ResponseType::Ok);

        let widget_clone = widget.clone();
        widget.connect_close_request(move |_| {
            widget_clone.response(ResponseType::Cancel);
            glib::Propagation::Proceed
        });

        Self {
            widget,
            log_dir_entry,
            proxy_port_entry,
            auto_restart_switch,
            auto_follow_switch,
            enable_notifications_switch,
            notify_on_switch_switch,
            autostart_switch,
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

        let mut config = Config::load().map_err(|e| format!("Failed to load config: {}", e))?;
        config.global.log_dir = log_dir;
        config.global.proxy_port = proxy_port;
        config.global.auto_restart_on_context_full = Some(auto_restart);
        config.global.auto_follow_logs = Some(auto_follow);
        config.preferences.enable_notifications = enable_notifications;
        config.preferences.notify_on_switch = notify_on_switch;
        config.preferences.autostart_on_login = autostart;

        Config::validate(&config, config_path).map_err(|e| format!("Config validation error: {}", e))?;

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

    fn add_autostart_on_login_row(parent: &gtk::Box, config: &Config) -> SwitchRow {
        let row = SwitchRow::builder()
            .title("Start SWAI automatically on login")
            .build();

        let autostart = config.autostart_on_login();
        row.set_active(autostart);

        parent.append(&row);
        row
    }

    // ─── Gateway Information Section (Phase 12.2 + Phase 17) ────────────────

    /// Add the Gateway information section at the bottom of the preferences dialog.
    ///
    /// Displays the proxy base URL with a copy button, auth key guidance, and
    /// three collapsible `AdwExpanderRow` accordions for client setup guides:
    /// Claude Code CLI, Claude Desktop, and OpenAI Codex CLI.
    ///
    /// The three expanders form an accordion: opening one collapses the others.
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

        // ── Collapsible client setup guides (Phase 17) ──────────────────
        // Container for the accordion rows so we can wire mutual exclusion.
        let accordion_box = gtk::Box::new(Orientation::Vertical, 0);
        accordion_box.set_margin_start(0);
        accordion_box.set_margin_end(0);

        let claude_cli_expander = Self::build_claude_cli_expander(config);
        let claude_desktop_expander = Self::build_claude_desktop_expander(config);
        let codex_expander = Self::build_codex_expander(config);

        accordion_box.append(&claude_cli_expander);
        accordion_box.append(&claude_desktop_expander);
        accordion_box.append(&codex_expander);

        // Wire accordion mutual exclusion: when one expands, collapse the others.
        let expander_vec: Vec<ExpanderRow> = vec![
            claude_cli_expander.clone(),
            claude_desktop_expander.clone(),
            codex_expander.clone(),
        ];
        Self::wire_accordion_exclusion(&accordion_box, &expander_vec);

        parent.append(&accordion_box);
    }

    /// Wire accordion behavior: only one expander row may be expanded at a time.
    ///
    /// Uses `glib::idle_add` to defer sibling collapse until after the current
    /// click/expansion completes, avoiding interference with GTK's internal
    /// `ExpanderRow` click handling.
    fn wire_accordion_exclusion(
        container: &gtk::Box,
        expanders: &[ExpanderRow],
    ) {
        for expander in expanders {
            let siblings: Vec<ExpanderRow> = expanders.to_vec();

            expander.connect_notify_local(Some("expanded"), move |widget, _param| {
                let is_expanded: bool = widget.property("expanded");
                if !is_expanded {
                    return;
                }

                // Defer collapse to avoid interfering with GTK's internal click handling.
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

        // Keep `container` alive alongside the closures.
        container.connect_destroy(move |_| {});
    }

    /// Build the Claude Code CLI setup guide expander row.
    fn build_claude_cli_expander(config: &Config) -> ExpanderRow {
        let port = config.proxy_port();
        let base_url = format!("http://127.0.0.1:{port}/v1", port = port);

        let expander = ExpanderRow::builder()
            .title("<span foreground=\"#DA7756\" weight=\"bold\">Claude Code CLI</span>")
            .build();

        let content = gtk::Box::new(Orientation::Vertical, 4);
        content.set_margin_start(12);
        content.set_margin_end(12);
        content.set_margin_top(6);
        content.set_margin_bottom(6);

        // Config file path row.
        let config_path_row = gtk::Box::new(Orientation::Horizontal, 6);
        let config_path_label = gtk::Label::builder()
            .label("Config file")
            .halign(gtk::Align::Start)
            .css_classes(vec!["dim-label"])
            .build();
        config_path_row.append(&config_path_label);

        let config_path_value = gtk::Label::builder()
            .label("~/.bashrc (or ~/.zshrc)")
            .xalign(0.0)
            .selectable(true)
            .build();
        config_path_row.append(&config_path_value);

        content.append(&config_path_row);

        // Base URL row with copy button.
        let url_row = gtk::Box::new(Orientation::Horizontal, 6);
        let url_label = gtk::Label::builder()
            .label("Base URL")
            .halign(gtk::Align::Start)
            .css_classes(vec!["dim-label"])
            .build();
        url_row.append(&url_label);

        let url_value = gtk::Label::builder()
            .label(&base_url)
            .xalign(0.0)
            .selectable(true)
            .build();
        url_row.append(&url_value);

        let copy_url_btn = gtk::Button::builder()
            .label("Copy")
            .css_classes(vec!["flat"])
            .build();
        let base_url_clone = base_url.clone();
        copy_url_btn.connect_clicked(move |_| {
            Self::copy_to_clipboard(&base_url_clone);
        });
        url_row.append(&copy_url_btn);

        content.append(&url_row);

        // API Key row with copy button.
        let key_row = gtk::Box::new(Orientation::Horizontal, 6);
        let key_label = gtk::Label::builder()
            .label("API Key")
            .halign(gtk::Align::Start)
            .css_classes(vec!["dim-label"])
            .build();
        key_row.append(&key_label);

        let key_value = gtk::Label::builder()
            .label("local")
            .xalign(0.0)
            .selectable(true)
            .build();
        key_row.append(&key_value);

        let copy_key_btn = gtk::Button::builder()
            .label("Copy")
            .css_classes(vec!["flat"])
            .build();
        copy_key_btn.connect_clicked(move |_| {
            Self::copy_to_clipboard("local");
        });
        key_row.append(&copy_key_btn);

        content.append(&key_row);

        // Explanatory header label.
        let header_label = gtk::Label::builder()
            .label("Add the following function to your shell profile (~/.bashrc or ~/.zshrc):")
            .halign(gtk::Align::Start)
            .margin_top(6)
            .build();
        content.append(&header_label);

        // Monospace bash function display label.
        let func_display = format!(
            "claude-local() {{\n  export ANTHROPIC_BASE_URL=http://127.0.0.1:{port}/v1\n  \
             export ANTHROPIC_AUTH_TOKEN=local\n  export ANTHROPIC_API_KEY=\"\"\n  local live_model\n  \
             live_model=$(curl -s http://127.0.0.1:{port}/v1/models | grep -o '\"id\":\"[^\"]*\"' | head -1 | cut -d'\"' -f4 | sed 's/^claude-//')\n  \
             export ANTHROPIC_MODEL=\"${{live_model:-unknown}}[1m]\"\n  \
             export ANTHROPIC_SMALL_FAST_MODEL=\"$ANTHROPIC_MODEL\"\n  claude \"${{@}}\"\n}}",
            port = port,
        );
        let instructions = gtk::Label::builder()
            .label(&func_display)
            .use_markup(false)
            .wrap(true)
            .xalign(0.0)
            .css_classes(vec!["monospace"])
            .margin_top(4)
            .build();
        content.append(&instructions);

        // Explanatory footer label.
        let footer_label = gtk::Label::builder()
            .label("Then reload with `source ~/.bashrc` (or `source ~/.zshrc`) and run `claude-local` in your terminal to start Claude Code.")
            .halign(gtk::Align::Start)
            .wrap(true)
            .margin_top(6)
            .build();
        content.append(&footer_label);

        // Button row: "Copy Config Block" and "Open Config File".
        let btn_box = gtk::Box::new(Orientation::Horizontal, 6);
        btn_box.set_halign(gtk::Align::End);
        btn_box.set_margin_top(6);

        // Copy bash function button (primary action).
        let func = func_display;
        let copy_func_btn = gtk::Button::builder()
            .label("Copy Config Block")
            .css_classes(vec!["flat", "suggested-action"])
            .build();
        let func_clone = func.clone();
        copy_func_btn.connect_clicked(move |_| {
            Self::copy_to_clipboard(&func_clone);
        });
        btn_box.append(&copy_func_btn);

        // Open Config File button.
        let open_cfg_btn = gtk::Button::builder()
            .label("Open Config File")
            .css_classes(vec!["flat"])
            .build();
        open_cfg_btn.connect_clicked(move |_| {
            Self::open_claude_cli_config();
        });
        btn_box.append(&open_cfg_btn);

        content.append(&btn_box);

        expander.add_row(&content);
        expander
    }

    /// Build the Claude Desktop setup guide expander row.
    fn build_claude_desktop_expander(config: &Config) -> ExpanderRow {
        let port = config.proxy_port();
        let base_url = format!("http://127.0.0.1:{port}/v1", port = port);

        let expander = ExpanderRow::builder()
            .title("<span foreground=\"#DA7756\" weight=\"bold\">Claude Desktop</span>")
            .build();

        let content = gtk::Box::new(Orientation::Vertical, 4);
        content.set_margin_start(12);
        content.set_margin_end(12);
        content.set_margin_top(6);
        content.set_margin_bottom(6);

        // Gateway URL row.
        let url_row = gtk::Box::new(Orientation::Horizontal, 6);
        let url_label = gtk::Label::builder()
            .label("Gateway URL")
            .halign(gtk::Align::Start)
            .css_classes(vec!["dim-label"])
            .build();
        url_row.append(&url_label);

        let url_value = gtk::Label::builder()
            .label(&base_url)
            .xalign(0.0)
            .selectable(true)
            .build();
        url_row.append(&url_value);

        let copy_url_btn = gtk::Button::builder()
            .label("Copy")
            .css_classes(vec!["flat"])
            .build();
        let base_url_clone = base_url.clone();
        copy_url_btn.connect_clicked(move |_| {
            Self::copy_to_clipboard(&base_url_clone);
        });
        url_row.append(&copy_url_btn);

        content.append(&url_row);

        // API Key row.
        let key_row = gtk::Box::new(Orientation::Horizontal, 6);
        let key_label = gtk::Label::builder()
            .label("API Key")
            .halign(gtk::Align::Start)
            .css_classes(vec!["dim-label"])
            .build();
        key_row.append(&key_label);

        let key_value = gtk::Label::builder()
            .label("swai-local")
            .xalign(0.0)
            .selectable(true)
            .build();
        key_row.append(&key_value);

        let copy_key_btn = gtk::Button::builder()
            .label("Copy")
            .css_classes(vec!["flat"])
            .build();
        copy_key_btn.connect_clicked(move |_| {
            Self::copy_to_clipboard("swai-local");
        });
        key_row.append(&copy_key_btn);

        content.append(&key_row);

        // Instructions label.
        let instructions = gtk::Label::builder()
            .label(
                &format!(
                    "To connect Claude Desktop:\n\
                     1. Enable Developer Mode in Claude Desktop's Help menu.\n\
                     2. Open Developer Menu → Configure Third-Party Inference → Gateway.\n\
                     3. Set Gateway Base URL to http://127.0.0.1:{port}/ and API Key to \"swai-local\".\n\
                     4. Under Models → Model list, click \"+ Add model\": set Model ID to \"claude\" and Display name to \"SWAI\". Toggle 1M-context ON.",
                    port = port,
                ),
            )
            .use_markup(false)
            .wrap(true)
            .xalign(0.0)
            .margin_top(6)
            .build();
        content.append(&instructions);

        expander.add_row(&content);
        expander
    }

    /// Build the OpenAI Codex CLI setup guide expander row.
    fn build_codex_expander(config: &Config) -> ExpanderRow {
        let port = config.proxy_port();
        let base_url = format!("http://127.0.0.1:{port}/v1", port = port);

        let expander = ExpanderRow::builder()
            .title("<span foreground=\"#7297FF\" weight=\"bold\">OpenAI Codex CLI</span>")
            .build();

        let content = gtk::Box::new(Orientation::Vertical, 4);
        content.set_margin_start(12);
        content.set_margin_end(12);
        content.set_margin_top(6);
        content.set_margin_bottom(6);

        // Config file path row.
        let config_path_row = gtk::Box::new(Orientation::Horizontal, 6);
        let config_path_label = gtk::Label::builder()
            .label("Config file")
            .halign(gtk::Align::Start)
            .css_classes(vec!["dim-label"])
            .build();
        config_path_row.append(&config_path_label);

        let config_path_value = gtk::Label::builder()
            .label("~/.codex/config.toml")
            .xalign(0.0)
            .selectable(true)
            .build();
        config_path_row.append(&config_path_value);

        content.append(&config_path_row);

        // Base URL row.
        let url_row = gtk::Box::new(Orientation::Horizontal, 6);
        let url_label = gtk::Label::builder()
            .label("Base URL")
            .halign(gtk::Align::Start)
            .css_classes(vec!["dim-label"])
            .build();
        url_row.append(&url_label);

        let url_value = gtk::Label::builder()
            .label(&base_url)
            .xalign(0.0)
            .selectable(true)
            .build();
        url_row.append(&url_value);

        let copy_url_btn = gtk::Button::builder()
            .label("Copy")
            .css_classes(vec!["flat"])
            .build();
        let base_url_clone = base_url.clone();
        copy_url_btn.connect_clicked(move |_| {
            Self::copy_to_clipboard(&base_url_clone);
        });
        url_row.append(&copy_url_btn);

        content.append(&url_row);

        // API Key row.
        let key_row = gtk::Box::new(Orientation::Horizontal, 6);
        let key_label = gtk::Label::builder()
            .label("API Key")
            .halign(gtk::Align::Start)
            .css_classes(vec!["dim-label"])
            .build();
        key_row.append(&key_label);

        let key_value = gtk::Label::builder()
            .label("swai-local")
            .xalign(0.0)
            .selectable(true)
            .build();
        key_row.append(&key_value);

        let copy_key_btn = gtk::Button::builder()
            .label("Copy")
            .css_classes(vec!["flat"])
            .build();
        copy_key_btn.connect_clicked(move |_| {
            Self::copy_to_clipboard("swai-local");
        });
        key_row.append(&copy_key_btn);

        content.append(&key_row);

        // Instructions label.
        let instructions = gtk::Label::builder()
            .label(
                &format!(
                    "Add the following to `~/.codex/config.toml`:\n\n\
                     model_provider = \"swai\"\n\n\
                     [model_providers.swai]\n\
                     name = \"SWAI Local AI\"\n\
                     base_url = \"{base_url}\"\n\
                     wire_api = \"responses\"\n\
                     api_key = \"local\"\n\n\
                     Restart Codex CLI after editing the config file.",
                    base_url = base_url,
                ),
            )
            .use_markup(false)
            .wrap(true)
            .xalign(0.0)
            .margin_top(6)
            .build();
        content.append(&instructions);

        // Button row: "Copy Config Block" and "Open Config File".
        let btn_box = gtk::Box::new(Orientation::Horizontal, 6);
        btn_box.set_halign(gtk::Align::End);
        btn_box.set_margin_top(6);

        let config_block = format!(
            "model_provider = \"swai\"\n\n[model_providers.swai]\nname = \"SWAI Local AI\"\nbase_url = \"{base_url}\"\nwire_api = \"responses\"\napi_key = \"local\"",
            base_url = base_url,
        );
        let copy_config_btn = gtk::Button::builder()
            .label("Copy Config Block")
            .css_classes(vec!["flat", "suggested-action"])
            .build();
        let config_block_clone = config_block.clone();
        copy_config_btn.connect_clicked(move |_| {
            Self::copy_to_clipboard(&config_block_clone);
        });
        btn_box.append(&copy_config_btn);

        // Open Config File button.
        let open_cfg_btn = gtk::Button::builder()
            .label("Open Config File")
            .css_classes(vec!["flat"])
            .build();
        open_cfg_btn.connect_clicked(move |_| {
            Self::open_codex_config();
        });
        btn_box.append(&open_cfg_btn);

        content.append(&btn_box);

        expander.add_row(&content);
        expander
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

    /// Open `~/.codex/config.toml` in the system default text editor.
    ///
    /// If the file does not exist, creates `~/.codex/` and writes a default
    /// `[proxy]` block before opening.
    fn open_codex_config() {
        let home = std::env::var("HOME").unwrap_or_default();
        let codex_dir = PathBuf::from(&home).join(".codex");
        let config_path = codex_dir.join("config.toml");

        if !config_path.exists() {
            // Create parent directory.
            if let Err(e) = std::fs::create_dir_all(&codex_dir) {
                tracing::warn!("failed to create ~/.codex/: {e}");
                return;
            }
            // Write a default config block so the file is non-empty.
            let default_block = "model_provider = \"swai\"\n\n[model_providers.swai]\nname = \"SWAI Local AI\"\nbase_url = \"http://127.0.0.1:8765/v1\"\nwire_api = \"responses\"\napi_key = \"local\"\n";
            if let Err(e) = std::fs::write(&config_path, default_block) {
                tracing::warn!("failed to write default config: {e}");
                return;
            }
            tracing::info!("created default ~/.codex/config.toml");
        }

        // Open in the system default text editor via xdg-open.
        let uri = format!("file://{}", config_path.display());
        let _ = Command::new("xdg-open")
            .arg(&uri)
            .spawn();
    }

    /// Open `~/.bashrc` (or `~/.zshrc`) in the system default text editor.
    fn open_claude_cli_config() {
        let home = std::env::var("HOME").unwrap_or_default();
        let bashrc_path = PathBuf::from(&home).join(".bashrc");

        // Prefer .bashrc; fall back to .zshrc if it doesn't exist.
        let config_path = if bashrc_path.exists() {
            bashrc_path
        } else {
            PathBuf::from(&home).join(".zshrc")
        };

        let uri = format!("file://{}", config_path.display());
        let _ = Command::new("xdg-open")
            .arg(&uri)
            .spawn();
    }
}
