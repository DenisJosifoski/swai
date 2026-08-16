use gtk::prelude::*;
use gtk::{FileChooserAction, ResponseType};
use gtk4 as gtk;

use adw::prelude::*;
use adw::EntryRow;

use super::helpers::show_error;
use super::sync_port::sync_port_in_script;
use crate::window::ImportMessage;
use swai_core::config::Config;

pub fn show_edit_dialog(
    parent_win: &std::sync::Arc<gtk::Window>,
    name: &str,
    id: &str,
    script_path: &std::path::PathBuf,
    port: u16,
    timeout: u16,
    import_sender: &std::sync::mpsc::Sender<ImportMessage>,
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
    let name_entry = EntryRow::builder().title("Display Name").text(name).build();
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
        show_file_chooser(&script_entry_for_browse, &name_clone_for_browse);
    });
    script_entry.add_suffix(&browse_btn);

    prefs_group.add(&script_entry);

    // ── Port ───────────────────────────────────────────────────
    let port_entry = add_port_row(port);
    prefs_group.add(&port_entry);

    // ── Health Check Timeout ───────────────────────────────────
    let timeout_entry = add_timeout_row(timeout);
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
            show_error(
                Some(&dialog_clone),
                "Script path is required.",
                "SWAI — Edit Error",
            );
            return;
        }

        let script_path_buf = std::path::PathBuf::from(&new_script_text);
        if !script_path_buf.exists() {
            show_error(
                Some(&dialog_clone),
                &format!("Script file not found: {}", script_path_buf.display()),
                "SWAI — Edit Error",
            );
            return;
        }

        if new_name.is_empty() {
            show_error(
                Some(&dialog_clone),
                "Display name is required.",
                "SWAI — Edit Error",
            );
            return;
        }

        let new_port: u16 = match new_port_str.parse() {
            Ok(p) => p,
            Err(_) => {
                show_error(
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
                show_error(
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
                    show_error(
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
        match save_edit(
            &id_value,
            &new_name,
            &script_path_buf,
            new_port,
            _new_timeout,
        ) {
            Ok(()) => {
                // Broadcast the name and port change to the main window.
                let _ = sender_clone.send(ImportMessage::ModelNameUpdated {
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
                            show_error(
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
                show_error(Some(&dialog_clone), &e, "SWAI — Edit Error");
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
fn show_file_chooser(script_entry: &EntryRow, _name_entry: &EntryRow) {
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
        let _ =
            chooser.set_current_folder(Some(&gio::File::for_path(std::path::PathBuf::from(path))));
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
    let config_path = Config::resolve_path()
        .ok_or_else(|| "No config file found at any expected location.".to_string())?;

    let content = std::fs::read_to_string(&config_path)
        .map_err(|e| format!("Failed to read config: {}", e))?;

    let mut config: Config =
        toml::from_str(&content).map_err(|e| format!("Failed to parse config: {}", e))?;

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
