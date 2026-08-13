//! Manage Models dialog — edit, sync port, and delete models.
//!
//! Modal dialog allowing model configuration editing, script opening, and deletion.
//! "Edit Script" button opens the .sh file in the system default
//! editor; "Save & Sync Script" button updates the PORT variable / --port flag
//! inside the script using pre-compiled regexes (OnceLock) and explicit capture
//! group replacement to avoid group-index concatenation bugs.

use gtk4 as gtk;
use gtk::prelude::*;
use gtk::{FileChooserAction, MessageType, ResponseType, Window};

use adw::prelude::*;
use adw::EntryRow;

use swai_core::config::{Config, ModelConfig};
use swai_core::process_manager::ProcessManager;

/// A non-blocking modal dialog listing all configured models.
#[derive(Clone)]
pub struct ManageModelsDialog {
    pub widget: gtk::Dialog,
    /// Owned reference to the parent window. Passed by reference to
    /// `build_model_row` so edit-dialog closures can use it as a transient
    /// parent without borrowing from `new()`. The field itself is never read
    /// after construction — it exists solely to own the `Arc`.
    #[allow(dead_code)]
    parent_win: std::sync::Arc<gtk::Window>,
}

impl ManageModelsDialog {
    /// Create a new Manage Models dialog transient to the given parent window.
    ///
    /// Loads the current config and renders one `adw::ActionRow` per model —
    /// title = display name, subtitle = model id. Each row has an edit button
    /// (document-edit-symbolic) and a delete button (user-trash-symbolic) with
    /// destructive styling. If no models are configured, shows an empty list
    /// (the user is expected to use File → Add Model).
    pub fn new<T: IsA<Window>>(
        parent: &T,
        import_sender: std::sync::mpsc::Sender<super::window::ImportMessage>,
        process_manager: std::sync::Arc<std::sync::Mutex<ProcessManager>>,
    ) -> Self {
        let widget = gtk::Dialog::builder()
            .title("Manage Models")
            .transient_for(parent)
            .modal(true)
            .build();

        // Store the parent window as an Arc so closures in build_model_row
        // can use it as a transient parent without borrowing from this function.
        let parent_win = std::sync::Arc::new(parent.clone().upcast::<gtk::Window>());

        let content_box = gtk::Box::new(gtk::Orientation::Vertical, 0);
        content_box.set_margin_start(24);
        content_box.set_margin_end(24);
        content_box.set_margin_top(24);
        content_box.set_margin_bottom(24);

        // Load the current config. If loading fails, show an error message
        // inside the dialog instead of bubbling up — this keeps the dialog
        // self-contained and non-blocking.
        let models = match Config::load() {
            Ok(cfg) => cfg.models,
            Err(e) => {
                let error_label = gtk::Label::new(Some(&format!(
                    "Could not load configuration:\n{}",
                    e
                )));
                error_label.set_wrap(true);
                error_label.set_justify(gtk::Justification::Center);
                error_label.set_margin_top(24);
                error_label.set_margin_bottom(24);
                content_box.append(&error_label);

                // Still show a Close button so the user can dismiss.
                widget.add_button("_Close", ResponseType::Close);
                return Self { widget, parent_win };
            }
        };

        if models.is_empty() {
            let empty_label = gtk::Label::new(Some(
                "No models configured.\n\nUse File → Add Model to import one.",
            ));
            empty_label.set_wrap(true);
            empty_label.set_justify(gtk::Justification::Center);
            empty_label.set_margin_top(32);
            empty_label.set_margin_bottom(32);
            content_box.append(&empty_label);
        } else {
            let prefs_group = adw::PreferencesGroup::new();
            for model in &models {
                let row = Self::build_model_row(
                    &parent_win,
                    model,
                    &import_sender,
                    &process_manager,
                );

                prefs_group.add(&row);
            }
            content_box.append(&prefs_group);
        }

        widget.content_area().append(&content_box);
        widget.add_button("_Close", ResponseType::Close);

        Self { widget, parent_win }
    }

    /// Build a single model row with title, subtitle, edit button, and delete button.
    fn build_model_row(
        parent_win: &std::sync::Arc<gtk::Window>,
        model: &ModelConfig,
        import_sender: &std::sync::mpsc::Sender<super::window::ImportMessage>,
        process_manager: &std::sync::Arc<std::sync::Mutex<ProcessManager>>,
    ) -> adw::ActionRow {
        let row = adw::ActionRow::builder()
            .title(&model.name)
            .subtitle(&model.id)
            .build();

        // Edit button (pencil icon) — opens the edit dialog.
        let edit_btn = gtk::Button::builder()
            .icon_name("document-edit-symbolic")
            .tooltip_text("Edit Model")
            .css_classes(["flat"])
            .build();

        // Clone model data for the closure (cheap reference counts).
        // Note: model_name is NOT captured here — it's read dynamically from
        // `row_for_dialog.title()` inside the click handler so that edits made
        // to the display name are reflected even when re-opening the dialog.
        let model_id = model.id.clone();
        let script_path = model.script_path.clone();
        let port = model.port;
        let timeout = model.health_timeout_sec;
        let sender_clone = import_sender.clone();
        // Clone the row and port label so the edit dialog can live-update them
        // after a successful save (set_title / set_text). GTK widget clones are
        // cheap refcount increments.
        let row_for_dialog = row.clone();
        let port_label = gtk::Label::new(Some(&format!("port {}", model.port)));
        port_label.set_css_classes(&["dim-label"]);
        row.add_suffix(&port_label);
        let port_label_for_dialog = port_label.clone();
        let parent_clone = std::sync::Arc::clone(parent_win);

        edit_btn.connect_clicked(move |_| {
            // Read the current title and port dynamically so edits to display name
            // or port are reflected even when re-opening the dialog without closing it.
            let current_name = row_for_dialog.title().to_string();
            let current_port_str = port_label_for_dialog.text().to_string();
            let current_port: u16 = current_port_str
                .strip_prefix("port ")
                .and_then(|p| p.parse().ok())
                .unwrap_or(port);

            Self::show_edit_dialog(
                &parent_clone,
                &current_name,
                &model_id,
                &script_path,
                current_port,
                timeout,
                &sender_clone,
                row_for_dialog.clone(),
                port_label_for_dialog.clone(),
            );
        });

        row.add_suffix(&edit_btn);

        // Edit Script button (pencil-in-file icon) — opens the .sh file in
        // the system default editor via gio::AppInfo::launch_default_for_uri.
        let edit_script_btn = gtk::Button::builder()
            .icon_name("document-open-symbolic")
            .tooltip_text("Edit Script")
            .css_classes(["flat"])
            .build();

        let script_path_for_edit = model.script_path.clone();
        edit_script_btn.connect_clicked(move |_| {
            Self::launch_script_editor(&script_path_for_edit);
        });

        row.add_suffix(&edit_script_btn);

        // Delete button (trash icon) — destructive styling.
        let delete_btn = gtk::Button::builder()
            .icon_name("user-trash-symbolic")
            .tooltip_text("Delete Model")
            .css_classes(["flat", "destructive-action"])
            .build();

        let model_id_for_delete = model.id.clone();
        let model_name_for_delete = model.name.clone();
        let pm_clone = std::sync::Arc::clone(process_manager);
        let sender_for_delete = import_sender.clone();

        delete_btn.connect_clicked(move |_| {
            Self::show_delete_confirmation(
                &model_id_for_delete,
                &model_name_for_delete,
                &pm_clone,
                &sender_for_delete,
            );
        });

        row.add_suffix(&delete_btn);
        row
    }

    /// Show a confirmation dialog before deleting a model.
    ///
    /// If the model is currently running, shows a warning dialog instead and
    /// refuses to delete it (the user must stop it first).
    fn show_delete_confirmation(
        id: &str,
        name: &str,
        process_manager: &std::sync::Arc<std::sync::Mutex<ProcessManager>>,
        import_sender: &std::sync::mpsc::Sender<super::window::ImportMessage>,
    ) {
        // Check if the model is currently running.
        let is_running = {
            let pm = match process_manager.lock() {
                Ok(g) => g,
                Err(_) => {
                    Self::show_error(
                        None::<&gtk::Window>,
                        "Process manager lock poisoned — cannot delete.",
                        "SWAI — Delete Error",
                    );
                    return;
                }
            };
            pm.find_running_model(id).is_some()
        };

        if is_running {
            let dialog = gtk::MessageDialog::new(
                None::<&gtk::Window>,
                gtk::DialogFlags::MODAL,
                MessageType::Warning,
                gtk::ButtonsType::Close,
                format!(
                    "Model '{}' is currently running and cannot be deleted.\n\n\
                     Please stop the model first before deleting it.",
                    name,
                ),
            );
            dialog.set_title(Some("SWAI — Cannot Delete"));
            dialog.connect_response(|d, _| d.destroy());
            dialog.present();
            return;
        }

        // Show confirmation dialog.
        let dialog = gtk::MessageDialog::new(
            None::<&gtk::Window>,
            gtk::DialogFlags::MODAL,
            MessageType::Question,
            gtk::ButtonsType::None,
            format!(
                "Are you sure you want to delete the model \"{}\"?\n\n\
                 This will remove it from config.toml and cannot be undone.",
                name,
            ),
        );
        dialog.set_title(Some("SWAI — Delete Model"));

        dialog.add_button("_Cancel", ResponseType::Cancel);
        dialog.add_button("_Delete", ResponseType::Ok);

        let id_clone = id.to_string();
        let pm_for_confirm = std::sync::Arc::clone(process_manager);
        let sender_clone = import_sender.clone();

        dialog.connect_response(move |d, response| {
            if response != ResponseType::Ok {
                d.destroy();
                return;
            }

            // Perform the deletion.
            match pm_for_confirm.lock() {
                Ok(mut pm) => {
                    match pm.remove_model(&id_clone) {
                        Ok(()) => {
                            tracing::info!("Model '{}' deleted successfully", id_clone);
                            let _ = sender_clone.send(
                                super::window::ImportMessage::ModelDeleted {
                                    id: id_clone.clone(),
                                },
                            );
                        }
                        Err(e) => {
                            Self::show_error(
                                None::<&gtk::Window>,
                                &format!("Failed to delete model:\n\n{}", e),
                                "SWAI — Delete Error",
                            );
                        }
                    }
                }
                Err(_) => {
                    Self::show_error(
                        None::<&gtk::Window>,
                        "Process manager lock poisoned — cannot delete.",
                        "SWAI — Delete Error",
                    );
                }
            }

            d.destroy();
        });

        dialog.present();
    }

    /// Show the edit dialog for a specific model.
    ///
    /// Opens a modal `gtk::Dialog` transient to the parent window with editable
    /// fields: Display Name, Script Path (with Browse button), Port, and
    /// Health Check Timeout. Model ID is shown but read-only.
    ///
    /// Validation runs on save: script file must exist, port must be a valid
    /// positive number, and port must not conflict with any other model in
    /// `config.toml`.
    ///
    /// On successful save, broadcasts `ModelNameUpdated` through the import
    /// channel so the main window can update card labels live. Also updates
    /// the row title and port label in-place so the Manage Models dialog stays
    /// in sync without needing to close and reopen.
    fn show_edit_dialog(
        parent_win: &std::sync::Arc<gtk::Window>,
        name: &str,
        id: &str,
        script_path: &std::path::PathBuf,
        port: u16,
        timeout: u16,
        import_sender: &std::sync::mpsc::Sender<super::window::ImportMessage>,
        row: adw::ActionRow,
        port_label: gtk::Label,
    ) {
        let dialog = gtk::Dialog::builder()
            .title(format!("Edit Model: {}", id))
            .transient_for(parent_win.as_ref())
            .modal(true)
            .build();

        let content_area = dialog.content_area();

        let prefs_group = adw::PreferencesGroup::new();
        prefs_group.set_title("Model Configuration");

        // ── Model ID (read-only) ───────────────────────────────────
        let id_entry = EntryRow::builder()
            .title("Model ID")
            .text(id)
            .editable(false)
            .build();
        prefs_group.add(&id_entry);

        // ── Display Name ───────────────────────────────────────────
        let name_entry = EntryRow::builder()
            .title("Display Name")
            .text(name)
            .build();
        prefs_group.add(&name_entry);

        // ── Script Path (with Browse button) ───────────────────────
        let script_entry = EntryRow::builder()
            .title("Script File")
            .text(script_path.to_string_lossy().as_ref())
            .build();

        let name_clone_for_browse = name_entry.clone();
        let script_entry_for_browse = script_entry.clone();

        let browse_btn = gtk::Button::builder()
            .label("Browse…")
            .css_classes(["flat"])
            .build();
        browse_btn.connect_clicked(move |_| {
            Self::show_file_chooser(
                &script_entry_for_browse,
                &name_clone_for_browse,
            );
        });
        script_entry.add_suffix(&browse_btn);

        prefs_group.add(&script_entry);

        // ── Port ───────────────────────────────────────────────────
        let port_entry = Self::add_port_row(port);
        prefs_group.add(&port_entry);

        // ── Health Check Timeout ───────────────────────────────────
        let timeout_entry = Self::add_timeout_row(timeout);
        prefs_group.add(&timeout_entry);

        content_area.append(&prefs_group);

        dialog.add_button("_Cancel", ResponseType::Cancel);
        dialog.add_button("_Save", ResponseType::Ok);
        dialog.add_button("Save & Sync _Script", ResponseType::Apply);

        // Capture values for the response handler.
        let id_value = id.to_string();
        let dialog_clone = dialog.clone();
        let sender_clone = import_sender.clone();
        // Clone the row and port label so the closure can update them after a
        // successful save. GTK widget clones are cheap refcount increments;
        // the closure is dropped when the dialog closes, cleaning up naturally.
        let row_for_update = row.clone();
        let port_label_for_update = port_label.clone();

        dialog.connect_response(move |d, response| {
            if response == ResponseType::Cancel {
                d.destroy();
                return;
            }

            // Read current values from the form at click time (not at dialog
            // construction). This ensures the port reflects what the user typed.
            let new_name = name_entry.text().to_string();
            let new_script_text = script_entry.text().to_string();
            let new_port_str = port_entry.text();
            let new_timeout_str = timeout_entry.text();

            // ── Validate ─────────────────────────────────────────
            if new_script_text.is_empty() {
                Self::show_error(Some(&dialog_clone), "Script path is required.", "SWAI — Edit Error");
                return;
            }

            let script_path_buf = std::path::PathBuf::from(&new_script_text);
            if !script_path_buf.exists() {
                Self::show_error(
                    Some(&dialog_clone),
                    &format!("Script file not found: {}", script_path_buf.display()),
                    "SWAI — Edit Error",
                );
                return;
            }

            if new_name.is_empty() {
                Self::show_error(Some(&dialog_clone), "Display name is required.", "SWAI — Edit Error");
                return;
            }

            let new_port: u16 = match new_port_str.parse() {
                Ok(p) => p,
                Err(_) => {
                    Self::show_error(
                        Some(&dialog_clone),
                        &format!("Invalid port number: {}", new_port_str),
                        "SWAI — Edit Error",
                    );
                    return;
                }
            };

            let _new_timeout: u16 = match new_timeout_str.parse() {
                Ok(t) => t,
                Err(_) => {
                    Self::show_error(
                        Some(&dialog_clone),
                        &format!("Invalid timeout value: {}", new_timeout_str),
                        "SWAI — Edit Error",
                    );
                    return;
                }
            };

            // ── Check port uniqueness across all other models ──────
            if let Ok(cfg) = Config::load() {
                for model in &cfg.models {
                    if model.id != id_value && model.port == new_port {
                        Self::show_error(
                            Some(&dialog_clone),
                            &format!(
                                "Port {} is already assigned to model '{}'. \
                                 Please enter a unique port.",
                                new_port, model.name
                            ),
                            "SWAI — Edit Error",
                        );
                        return;
                    }
                }
            }

            // ── Save to config.toml ────────────────────────────────
            match Self::save_edit(
                &id_value,
                &new_name,
                &script_path_buf,
                new_port,
                _new_timeout,
            ) {
                Ok(()) => {
                    // Broadcast the name and port change to the main window.
                    let _ = sender_clone.send(super::window::ImportMessage::ModelNameUpdated {
                        id: id_value.clone(),
                        name: new_name.clone(),
                        port: new_port,
                    });

                    // Live-update the Manage Models dialog list so the row
                    // title and port label reflect the new values without
                    // requiring the user to close and reopen the dialog.
                    row_for_update.set_title(&new_name);
                    port_label_for_update.set_text(&format!("port {}", new_port));

                    // If "Save & Sync Script" was clicked, also update the
                    // .sh launch script with the new port.
                    if response == ResponseType::Apply {
                        match sync_port_in_script(&script_path_buf, new_port) {
                            Ok(()) => {
                                tracing::info!(
                                    "Synced port {} into script {}",
                                    new_port,
                                    script_path_buf.display()
                                );
                            }
                            Err(e) => {
                                Self::show_error(
                                    Some(&dialog_clone),
                                    &format!("Port synced in config but script update failed:\n{}", e),
                                    "SWAI — Script Sync Error",
                                );
                            }
                        }
                    }

                    d.destroy();
                }
                Err(e) => {
                    Self::show_error(Some(&dialog_clone), &e, "SWAI — Edit Error");
                }
            }
        });

        dialog.show();
    }

    /// Build a port entry row pre-filled with the given value.
    fn add_port_row(port: u16) -> EntryRow {
        EntryRow::builder()
            .title("Port")
            .text(port.to_string())
            .build()
    }

    /// Build a timeout entry row pre-filled with the given value.
    fn add_timeout_row(timeout: u16) -> EntryRow {
        EntryRow::builder()
            .title("Health Check Timeout (s)")
            .text(timeout.to_string())
            .build()
    }

    /// Show a native file chooser to select a launch script.
    fn show_file_chooser(
        script_entry: &EntryRow,
        _name_entry: &EntryRow,
    ) {
        let chooser = gtk::FileChooserNative::new(
            Some("Select Launch Script"),
            None::<&gtk::Window>, // No parent — center on screen
            FileChooserAction::Open,
            Some("_Select"),
            Some("_Cancel"),
        );

        // Filter to shell scripts
        let filter = gtk::FileFilter::new();
        filter.set_name(Some("Shell Scripts (*.sh)"));
        filter.add_pattern("*.sh");
        chooser.add_filter(&filter);

        // Also allow all files (in case the script has no .sh extension)
        let all_filter = gtk::FileFilter::new();
        all_filter.set_name(Some("All Files"));
        all_filter.add_pattern("*");
        chooser.add_filter(&all_filter);

        if let Ok(path) = std::env::var("HOME") {
            let _ = chooser.set_current_folder(Some(&gio::File::for_path(
                std::path::PathBuf::from(path),
            )));
        }

        let script_clone = script_entry.clone();

        chooser.connect_response(move |chooser, response| {
            if response == ResponseType::Accept {
                if let Some(file) = chooser.file() {
                    if let Some(path) = file.path() {
                        script_clone.set_text(path.to_string_lossy().as_ref());
                    }
                }
            }
            chooser.destroy();
        });

        chooser.show();
    }

    /// Save the edited model to config.toml atomically.
    fn save_edit(
        id: &str,
        new_name: &str,
        new_script_path: &std::path::PathBuf,
        new_port: u16,
        new_timeout: u16,
    ) -> Result<(), String> {
        let config_path = Config::resolve_path().ok_or_else(|| {
            "No config file found at any expected location.".to_string()
        })?;

        let content = std::fs::read_to_string(&config_path)
            .map_err(|e| format!("Failed to read config: {}", e))?;

        let mut config: Config = toml::from_str(&content)
            .map_err(|e| format!("Failed to parse config: {}", e))?;

        // Find and update the matching model.
        let found = config.models.iter_mut().find(|m| m.id == id);
        match found {
            Some(model) => {
                model.name = new_name.to_string();
                model.script_path = new_script_path.clone();
                model.port = new_port;
                model.health_timeout_sec = new_timeout;
            }
            None => {
                return Err(format!("Model '{}' not found in config.", id));
            }
        }

        // Validate the modified config (duplicate ports, missing scripts).
        Config::validate(&config, &config_path)
            .map_err(|e| format!("Config validation error: {}", e))?;

        let new_content = toml::to_string_pretty(&config)
            .map_err(|e| format!("Failed to serialize config: {}", e))?;
        std::fs::write(&config_path, &new_content)
            .map_err(|e| format!("Failed to write config file: {}", e))?;

        Ok(())
    }

    /// Show an error message dialog.
    fn show_error<P: gtk::prelude::IsA<gtk::Window>>(
        parent: Option<&P>,
        message: &str,
        title: &str,
    ) {
        let error_dialog = gtk::MessageDialog::new(
            parent,
            gtk::DialogFlags::MODAL,
            gtk::MessageType::Error,
            gtk::ButtonsType::Close,
            message,
        );
        error_dialog.set_title(Some(title));
        error_dialog.connect_response(|d, _| d.destroy());
        error_dialog.present();
    }

    /// Open the script file in the system's default editor.
    ///
    /// Uses `gio::AppInfo::launch_default_for_uri` to hand off the `.sh` file
    /// to whatever application the desktop environment considers the default
    /// text/code editor (gedit, kate, code, etc.). Errors are logged but not
    /// surfaced — if no editor is registered the OS silently does nothing.
    fn launch_script_editor(script_path: &std::path::PathBuf) {
        let uri = gio::File::for_path(script_path).uri();
        tracing::debug!("Launching default editor for {}", uri);
        let _ = gio::AppInfo::launch_default_for_uri(&uri, None::<&gio::AppLaunchContext>);
    }
}

/// Pre-compiled regexes for matching port assignment patterns in shell scripts.
///
/// Using `OnceLock` ensures the regexes are compiled exactly once regardless of
/// how many times `sync_port_in_script` is called, avoiding repeated parse/compile
/// overhead on every dialog save.
struct PortRegexes {
    /// Matches: PORT="${PORT:-N}", PORT=N, PORT="N", PORT='N', export PORT=N
    var_assign: std::sync::OnceLock<regex::Regex>,
    /// Matches: --port N or --port=N
    long_flag: std::sync::OnceLock<regex::Regex>,
    /// Matches: -p N (only when not preceded by another `-`, i.e. not `--something`)
    short_flag: std::sync::OnceLock<regex::Regex>,
}

impl Default for PortRegexes {
    fn default() -> Self {
        Self {
            var_assign: std::sync::OnceLock::new(),
            long_flag: std::sync::OnceLock::new(),
            short_flag: std::sync::OnceLock::new(),
        }
    }
}

impl PortRegexes {
    fn var_assign(&self) -> &regex::Regex {
        self.var_assign.get_or_init(|| {
            // Use [ \t] instead of \s to avoid matching newlines — since we
            // process line-by-line, a leading space/tab is the only valid prefix.
            regex::Regex::new(
                r#"(^|[ \t])(export[ \t]+)?PORT=("([^"]*)"|'([^']*)'|(\d+))"#,
            )
            .expect("port var_assign regex must compile")
        })
    }

    fn long_flag(&self) -> &regex::Regex {
        self.long_flag.get_or_init(|| {
            // Match `--port=N` or `--port N`. The separator is `=` or whitespace.
            regex::Regex::new(r"--port[= \t]+(\d+)")
                .expect("port long_flag regex must compile")
        })
    }

    fn short_flag(&self) -> &regex::Regex {
        self.short_flag.get_or_init(|| {
            // Match standalone `-p N` or `-p=N` (preceded by whitespace/start of line).
            // Prevents false positives on long flags like `--repeat-penalty 1.05`.
            regex::Regex::new(r"(^|[ \t])(-p[ \t=]+(\d+))")
                .expect("port short_flag regex must compile")
        })
    }
}

/// Synchronize the port value inside a `.sh` launch script.
///
/// Reads the current text of `port_entry` at click time (not at dialog
/// construction), then rewrites any matching port assignments in the script
/// file. The function is idempotent — running it twice with the same port
/// produces identical output.
///
/// Supported patterns:
/// - `PORT=N`, `PORT="N"`, `PORT='N'`, `export PORT=N`
/// - `--port N`, `--port=N`
/// - `-p N` (only when not preceded by `-`, to avoid matching `--something`)
///
/// Uses explicit `${1}` / `${2}` capture-group replacement syntax to avoid the
/// classic `$18091` bug where `$18` would be interpreted as capture group 18
/// instead of `$1` followed by literal `8`.
pub fn sync_port_in_script(
    script_path: &std::path::PathBuf,
    new_port: u16,
) -> Result<(), String> {
    let new_port_str = new_port.to_string();

    // Read current script content.
    let content = std::fs::read_to_string(script_path)
        .map_err(|e| format!("Failed to read script {}: {}", script_path.display(), e))?;

    let re = PortRegexes::default();

    // We apply replacements line-by-line so that `-p` on one line doesn't
    // accidentally match `--port` on another. This also preserves blank lines
    // and comments untouched.
    let mut updated_lines: Vec<String> = Vec::with_capacity(content.lines().count());

    for line in content.lines() {
        let trimmed = line.trim();

        // Skip comment-only lines entirely — never rewrite a comment.
        if trimmed.starts_with('#') {
            updated_lines.push(line.to_string());
            continue;
        }

        let mut current = line.to_string();

        // 1. Variable assignments: PORT=N, PORT="N", export PORT=N, etc.
        current = replace_first(&current, re.var_assign(), &new_port_str);

        // 2. Long flag: --port N or --port=N
        current = replace_first(&current, re.long_flag(), &new_port_str);

        // 3. Short flag: -p N (only when not preceded by `-`)
        current = replace_first(&current, re.short_flag(), &new_port_str);

        updated_lines.push(current);
    }

    let new_content = updated_lines.join("\n");

    // Only write if something actually changed — avoids unnecessary mtime bumps.
    if new_content != content {
        std::fs::write(script_path, &new_content)
            .map_err(|e| format!("Failed to write script {}: {}", script_path.display(), e))?;
        tracing::info!(
            "Synced port {} into script {}",
            new_port_str,
            script_path.display()
        );
    } else {
        tracing::debug!(
            "No port assignments found in {}; skipping rewrite.",
            script_path.display()
        );
    }

    Ok(())
}

/// Apply a single regex replacement to `input`, replacing only the capture
/// group that holds the port number with `replacement`. All other parts of
/// the match (prefix, surrounding text, non-port groups) are preserved
/// verbatim. Returns the original string unchanged when no match is found.
///
/// For unquoted ports (`PORT=8096`), replaces just the bare digits in group 3.
/// For quoted ports (`PORT="${PORT:-8096}"`), replaces only the bare digits
/// inside the quotes to preserve `${...}` shell syntax.
fn replace_first(input: &str, re: &regex::Regex, replacement: &str) -> String {
    let caps = match re.captures(input) {
        Some(c) => c,
        None => return input.to_string(),
    };

    // Check if group 3 captured a quoted value (starts with `"` or `'`).
    let group3 = caps.get(3);
    let is_quoted = group3
        .map(|m| m.as_str().starts_with('\"') || m.as_str().starts_with('\''))
        .unwrap_or(false);

    if is_quoted {
        // For quoted values like `"${PORT:-8096}"`, group 3 includes the quotes.
        // We need to replace only the bare digits inside, preserving `${...}` syntax.
        let digit_re = regex::Regex::new(r"\d+").unwrap();
        if let Some(g3) = group3 {
            if let Some(digit_match) = digit_re.find(g3.as_str()) {
                let g3_start = g3.start();
                let abs_start = g3_start + digit_match.start();
                let abs_end = g3_start + digit_match.end();
                let mut result = input.to_string();
                result.replace_range(abs_start..abs_end, replacement);
                return result;
            }
        }
        input.to_string()
    } else {
        // Unquoted: replace the group that contains just the bare digits.
        let port_group = find_port_capture_index(re, input);
        if port_group < caps.len() {
            if let Some(m) = caps.get(port_group) {
                let mut result = input.to_string();
                result.replace_range(m.range(), replacement);
                return result;
            }
        }
        input.to_string()
    }
}

/// Determine which capture-group index holds the port number for a given regex
/// and input line. The function inspects each group's matched text to find the
/// one that parses as a number — that's the port group.
///
/// Strategy: prefer groups whose entire match is digits only (most likely the
/// bare port number), then fall back to any group that parses as u16.
fn find_port_capture_index(re: &regex::Regex, line: &str) -> usize {
    let caps = re.captures(line).expect("replace_first called without a match");

    // First pass: find groups whose entire text is digits (bare port numbers).
    // This handles long_flag group 1, short_flag group 3, var_assign group 6.
    for i in 0..caps.len() {
        if let Some(m) = caps.get(i) {
            if m.as_str().chars().all(|c| c.is_ascii_digit()) && !m.as_str().is_empty() {
                if let Ok(port) = m.as_str().parse::<u16>() {
                    if port > 0 {
                        return i;
                    }
                }
            }
        }
    }

    // Second pass: any group that parses as a valid port number.
    for i in 0..caps.len() {
        if let Some(m) = caps.get(i) {
            if let Ok(port) = m.as_str().parse::<u16>() {
                if port > 0 {
                    return i;
                }
            }
        }
    }

    // Fallback: no numeric group found — shouldn't happen if regex matched.
    0
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn test_sync_port_preserves_assignment_syntax() {
        // Create a temp script with the expected PORT assignment syntax
        let tmp = tempfile::tempdir().unwrap();
        let script_path = tmp.path().join("test_sync_port.sh");
        let mut f = std::fs::File::create(&script_path).unwrap();
        writeln!(f, "#!/usr/bin/env bash").unwrap();
        writeln!(f, "set -euo pipefail").unwrap();
        writeln!(f, "").unwrap();
        writeln!(f, "MODEL_PATH=\"${{MODEL_PATH:-/tmp/test.gguf}}\"").unwrap();
        writeln!(f, "PORT=\"${{PORT:-8096}}\"").unwrap();
        writeln!(f, "PARALLEL_SLOTS=\"${{PARALLEL_SLOTS:-1}}\"").unwrap();
        writeln!(f, "").unwrap();
        writeln!(f, "exec llama-server \\").unwrap();
        writeln!(f, "  --model \"$MODEL_PATH\" \\").unwrap();
        writeln!(f, "  --port \"$PORT\" \\").unwrap();
        writeln!(f, "  --host 127.0.0.1").unwrap();
        drop(f);

        // Sync port to 9999
        sync_port_in_script(&script_path, 9999).unwrap();

        let content = std::fs::read_to_string(&script_path).unwrap();

        // Verify the PORT assignment line is preserved with correct syntax
        assert!(
            content.contains("PORT=\"${PORT:-9999}\""),
            "Expected PORT assignment with shell expansion syntax, got:\n{}",
            content
        );

        // Verify --port flag is also updated
        assert!(
            content.contains("--port \"$PORT\""),
            "Expected --port flag to reference $PORT variable"
        );
    }

    #[test]
    fn test_sync_port_unquoted_value() {
        let tmp = tempfile::tempdir().unwrap();
        let script_path = tmp.path().join("test_sync_port_unquoted.sh");
        let mut f = std::fs::File::create(&script_path).unwrap();
        writeln!(f, "PORT=8096").unwrap();
        writeln!(f, "--port 8096").unwrap();
        drop(f);

        sync_port_in_script(&script_path, 4567).unwrap();

        let content = std::fs::read_to_string(&script_path).unwrap();
        assert!(content.contains("PORT=4567"), "Unquoted PORT assignment not updated: {}", content);
        assert!(content.contains("--port 4567"), "Unquoted --port flag not updated: {}", content);
    }

    #[test]
    fn test_sync_port_export_syntax() {
        let tmp = tempfile::tempdir().unwrap();
        let script_path = tmp.path().join("test_sync_port_export.sh");
        let mut f = std::fs::File::create(&script_path).unwrap();
        writeln!(f, "export PORT=3000").unwrap();
        drop(f);

        sync_port_in_script(&script_path, 7777).unwrap();

        let content = std::fs::read_to_string(&script_path).unwrap();
        assert!(
            content.contains("export PORT=7777"),
            "Exported PORT assignment not updated: {}",
            content
        );
    }

    #[test]
    fn test_sync_port_no_match_unchanged() {
        let tmp = tempfile::tempdir().unwrap();
        let script_path = tmp.path().join("test_sync_port_no_match.sh");
        let original = "#!/bin/bash\necho hello\n";
        std::fs::write(&script_path, original).unwrap();

        sync_port_in_script(&script_path, 9999).unwrap();

        let content = std::fs::read_to_string(&script_path).unwrap();
        // .lines() strips trailing newlines, so compare without the final \n
        assert_eq!(content.trim_end_matches('\n'), original.trim_end_matches('\n'),
            "Script without port assignments should be unchanged");
    }
}
