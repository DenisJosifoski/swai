use gtk4 as gtk;
use gtk::prelude::*;
use gtk::{FileChooserAction, ResponseType, Window};

use adw::prelude::*;
use adw::{EntryRow, PreferencesGroup};

use swai_core::config::Config;

use std::path::PathBuf;

use super::inference::{detect_port_from_script, infer_id_from_path, infer_name_from_path};
use super::types::ImportedModel;

/// A modal dialog for importing a new model from a launch script.
#[derive(Clone)]
pub struct ImportWizard {
    pub widget: gtk::Dialog,
    id_entry: EntryRow,
    name_entry: EntryRow,
    script_path_entry: EntryRow,
    port_entry: EntryRow,
    timeout_entry: EntryRow,
}

impl ImportWizard {
    /// Create a new import wizard dialog transient to the given parent window.
    pub fn new<T: IsA<Window>>(parent: &T, pre_filled_port: Option<u16>) -> Self {
        let widget = gtk::Dialog::builder()
            .title("Add Model")
            .transient_for(parent)
            .modal(true)
            .build();

        let content_area = widget.content_area();

        let prefs_group = PreferencesGroup::new();
        prefs_group.set_title("Model Configuration");

        let id_entry = Self::add_id_row();
        prefs_group.add(&id_entry);

        let name_entry = Self::add_name_row();
        prefs_group.add(&name_entry);

        let port_entry = Self::add_port_row(pre_filled_port);
        prefs_group.add(&port_entry);

        let script_path_entry = Self::add_script_path_row(parent, &id_entry, &name_entry, &port_entry);
        prefs_group.add(&script_path_entry);

        let timeout_entry = Self::add_timeout_row();
        prefs_group.add(&timeout_entry);

        content_area.append(&prefs_group);

        widget.add_button("_Cancel", ResponseType::Cancel);
        widget.add_button("_Add Model", ResponseType::Ok);

        Self {
            widget,
            id_entry,
            name_entry,
            script_path_entry,
            port_entry,
            timeout_entry,
        }
    }

    #[allow(dead_code)]
    pub fn destroy(&self) {
        self.widget.destroy();
    }

    pub fn set_display_name(&self, name: &str) {
        self.name_entry.set_text(name);
    }

    fn add_id_row() -> EntryRow {
        let row = EntryRow::new();
        row.set_title("Model ID (e.g. qwen-2.5-coder)");
        row
    }

    fn add_name_row() -> EntryRow {
        let row = EntryRow::new();
        row.set_title("Display Name");
        row
    }

    fn add_script_path_row<T: IsA<Window>>(
        parent: &T,
        id_entry: &EntryRow,
        name_entry: &EntryRow,
        port_entry: &EntryRow,
    ) -> EntryRow {
        let row = EntryRow::new();
        row.set_title("Script Path (.sh)");

        let browse_button = gtk::Button::builder()
            .label("Browse…")
            .valign(gtk::Align::Center)
            .build();

        let parent_clone = parent.clone();
        let row_clone = row.clone();
        let id_clone = id_entry.clone();
        let name_clone = name_entry.clone();
        let port_clone = port_entry.clone();

        browse_button.connect_clicked(move |_| {
            Self::show_file_chooser(&parent_clone, &row_clone, &id_clone, &name_clone, &port_clone);
        });

        row.add_suffix(&browse_button);
        row
    }

    fn add_port_row(pre_filled_port: Option<u16>) -> EntryRow {
        let row = EntryRow::new();
        row.set_title("Port (e.g. 8080)");
        if let Some(port) = pre_filled_port {
            row.set_text(&port.to_string());
        }
        row
    }

    fn add_timeout_row() -> EntryRow {
        let row = EntryRow::new();
        row.set_title("Health Timeout (seconds)");
        row.set_text("30");
        row
    }

    fn show_file_chooser<T: IsA<Window>>(
        parent: &T,
        script_row: &EntryRow,
        id_row: &EntryRow,
        name_row: &EntryRow,
        port_row: &EntryRow,
    ) {
        let chooser = gtk::FileChooserNative::new(
            Some("Select Launch Script"),
            Some(parent),
            FileChooserAction::Open,
            Some("_Open"),
            Some("_Cancel"),
        );

        let filter = gtk::FileFilter::new();
        filter.set_name(Some("Shell Scripts (*.sh)"));
        filter.add_pattern("*.sh");
        chooser.add_filter(&filter);

        let filter_all = gtk::FileFilter::new();
        filter_all.set_name(Some("All Files"));
        filter_all.add_pattern("*");
        chooser.add_filter(&filter_all);

        let script_row = script_row.clone();
        let id_row = id_row.clone();
        let name_row = name_row.clone();
        let port_row = port_row.clone();

        chooser.connect_response(move |dialog, response| {
            if response == ResponseType::Accept {
                if let Some(file) = dialog.file() {
                    if let Some(path) = file.path() {
                        script_row.set_text(&path.to_string_lossy());

                        if let Ok(content) = std::fs::read_to_string(&path) {
                            if let Some(detected_port) = detect_port_from_script(&content) {
                                port_row.set_text(&detected_port.to_string());
                            }
                        }

                        let inferred_name = infer_name_from_path(&path);
                        if name_row.text().is_empty() {
                            name_row.set_text(&inferred_name);
                        }

                        let inferred_id = infer_id_from_path(&path);
                        if id_row.text().is_empty() {
                            id_row.set_text(&inferred_id);
                        }
                    }
                }
            }
        });

        chooser.show();
    }

    fn validate_and_import(
        &self,
        existing_models: &[Config],
    ) -> Result<ImportedModel, String> {
        let script_text = self.script_path_entry.text();
        if script_text.is_empty() {
            return Err("Script path is required.".into());
        }

        let script_path = PathBuf::from(script_text.as_str());
        if !script_path.exists() {
            return Err(format!(
                "Script file not found: {}",
                script_path.display()
            ));
        }

        let id = self.id_entry.text();
        if id.is_empty() {
            return Err("Model ID is required.".into());
        }

        let name = self.name_entry.text();
        if name.is_empty() {
            return Err("Display name is required.".into());
        }

        let port_str = self.port_entry.text();
        let port: u16 = port_str.parse().map_err(|_| {
            format!("Invalid port number: {}", port_str)
        })?;

        let timeout_str = self.timeout_entry.text();
        let health_timeout_sec: u16 = timeout_str.parse().map_err(|_| {
            format!("Invalid timeout value: {}", timeout_str)
        })?;

        for cfg in existing_models {
            for model in &cfg.models {
                if model.port == port {
                    return Err(format!(
                        "Port {} is already assigned to model '{}'. Please enter a unique port (e.g. {}) for this model.",
                        port,
                        model.name,
                        port + 1
                    ));
                }
            }
        }

        for cfg in existing_models {
            for model in &cfg.models {
                if model.id == id {
                    return Err(format!(
                        "Model ID '{}' already exists.",
                        id
                    ));
                }
            }
        }

        Ok(ImportedModel {
            id: id.to_string(),
            name: name.to_string(),
            script_path,
            port,
            health_timeout_sec,
        })
    }

    /// Read the current form values and attempt to import a new model.
    pub fn try_import(&self) -> Result<ImportedModel, String> {
        let existing_configs: Vec<Config> = match Config::load() {
            Ok(cfg) => vec![cfg],
            Err(_) => vec![],
        };

        self.validate_and_import(&existing_configs)
    }

    /// Run auto-detection on a script file path and pre-fill form fields.
    #[allow(dead_code)]
    pub fn auto_detect_from_file<T: IsA<Window>>(&self, parent: &T) {
        let script_path = self.script_path_entry.text();
        if script_path.is_empty() {
            return;
        }

        let path = PathBuf::from(script_path.as_str());
        if !path.exists() {
            return;
        }

        let content = match std::fs::read_to_string(&path) {
            Ok(c) => c,
            Err(e) => {
                let dialog = gtk::MessageDialog::new(
                    Some(parent),
                    gtk::DialogFlags::MODAL,
                    gtk::MessageType::Error,
                    gtk::ButtonsType::Close,
                    format!("Failed to read script file:\n\n{}", e),
                );
                dialog.set_title(Some("SWAI — Import Error"));
                dialog.connect_response(|d, _| d.destroy());
                dialog.show();
                return;
            }
        };

        if let Some(port) = detect_port_from_script(&content) {
            self.port_entry.set_text(&port.to_string());
        }

        let inferred_name = infer_name_from_path(&path);
        if self.name_entry.text().is_empty() {
            self.name_entry.set_text(&inferred_name);
        }

        let inferred_id = infer_id_from_path(&path);
        if self.id_entry.text().is_empty() {
            self.id_entry.set_text(&inferred_id);
        }
    }
}
