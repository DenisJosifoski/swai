//! Import wizard modal dialog for adding new models to SWAI.
//!
//! Workflow:
//! 1. User clicks "Browse" to select a `.sh` script file.
//! 2. The wizard scans the script text for port patterns (`--port`, `PORT=`, `-p`)
//!    and pre-fills the port field; also infers a display name from the filename.
//! 3. User adjusts the form fields (ID, name, port, timeout).
//! 4. On "Add Model": validates script existence, unique ID, and no duplicate port,
//!    then appends the model to `config.toml` via `toml::to_string_pretty`.
//!
//! The dialog is shown non-blockingly via `connect_response` in the caller
//! (window.rs), so the GTK main loop is never blocked.

use gtk4 as gtk;
use gtk::prelude::*;
use gtk::{FileChooserAction, ResponseType, Window};

use adw::prelude::*;
use adw::{EntryRow, PreferencesGroup};

use swai_core::config::Config;

use std::path::PathBuf;

/// A newly imported model configuration ready to be saved.
pub struct ImportedModel {
    pub id: String,
    pub name: String,
    pub script_path: PathBuf,
    pub port: u16,
    pub health_timeout_sec: u16,
}

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
    ///
    /// `pre_filled_port` optionally pre-fills the Port field (e.g. when adopted
    /// from an unmanaged-server banner). All other fields start empty so the
    /// user can fill them in.
    pub fn new<T: IsA<Window>>(parent: &T, pre_filled_port: Option<u16>) -> Self {
        let widget = gtk::Dialog::builder()
            .title("Add Model")
            .transient_for(parent)
            .modal(true)
            .build();

        // ── Content area with preferences group ────────────────────
        let content_area = widget.content_area();

        let prefs_group = PreferencesGroup::new();
        prefs_group.set_title("Model Configuration");

        // ── Model ID ───────────────────────────────────────────────
        let id_entry = Self::add_id_row();
        prefs_group.add(&id_entry);

        // ── Display Name ───────────────────────────────────────────
        let name_entry = Self::add_name_row();
        prefs_group.add(&name_entry);

        // ── Port (defined before Script Path since script_path depends on it) ──
        let port_entry = Self::add_port_row(pre_filled_port);
        prefs_group.add(&port_entry);

        // ── Script Path (with Browse) ──────────────────────────────
        let script_path_entry = Self::add_script_path_row(parent, &id_entry, &name_entry, &port_entry);
        prefs_group.add(&script_path_entry);

        // ── Health Check Timeout ───────────────────────────────────
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

    /// Destroy the dialog.
    #[allow(dead_code)]
    pub fn destroy(&self) {
        self.widget.destroy();
    }

    /// Pre-fill the display name field. Used when adopting an unmanaged
    /// server so the user has a sensible default they can edit.
    pub fn set_display_name(&self, name: &str) {
        self.name_entry.set_text(name);
    }

    fn add_id_row() -> EntryRow {
        EntryRow::builder()
            .title("Model ID")
            .build()
    }

    fn add_name_row() -> EntryRow {
        EntryRow::builder()
            .title("Display Name")
            .build()
    }

    fn add_script_path_row<T: IsA<Window>>(
        dialog_parent: &T,
        id_entry: &EntryRow,
        name_entry: &EntryRow,
        port_entry: &EntryRow,
    ) -> EntryRow {
        let entry = EntryRow::builder()
            .title("Script File")
            .build();

        // Add Browse button to the end of the row.
        let entry_clone = entry.clone();
        let id_clone = id_entry.clone();
        let name_clone = name_entry.clone();
        let port_clone = port_entry.clone();
        let dialog_parent_clone = dialog_parent.to_owned();

        let browse_btn = gtk::Button::builder()
            .label("Browse…")
            .css_classes(["flat"])
            .build();
        browse_btn.connect_clicked(move |_| {
            Self::show_file_chooser(
                &entry_clone,
                &id_clone,
                &name_clone,
                &port_clone,
                &dialog_parent_clone,
            );
        });
        entry.add_suffix(&browse_btn);

        entry
    }

    fn add_port_row(pre_filled_port: Option<u16>) -> EntryRow {
        let text = pre_filled_port
            .map(|p| p.to_string())
            .unwrap_or_else(|| "8090".to_string());
        EntryRow::builder()
            .title("Port")
            .text(text)
            .build()
    }

    fn add_timeout_row() -> EntryRow {
        EntryRow::builder()
            .title("Health Check Timeout (s)")
            .text("30")
            .build()
    }

    /// Show a native desktop file chooser to select a launch script.
    fn show_file_chooser<T: IsA<Window>>(
        script_entry: &EntryRow,
        id_entry: &EntryRow,
        name_entry: &EntryRow,
        port_entry: &EntryRow,
        parent: &T,
    ) {
        let chooser = gtk::FileChooserNative::new(
            Some("Select Launch Script"),
            Some(parent),
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
            let _ = chooser.set_current_folder(Some(&gio::File::for_path(PathBuf::from(path))));
        }

        let script_entry_clone = script_entry.clone();
        let id_entry_clone = id_entry.clone();
        let name_entry_clone = name_entry.clone();
        let port_entry_clone = port_entry.clone();

        chooser.connect_response(move |chooser, response| {
            if response == ResponseType::Accept {
                if let Some(file) = chooser.file() {
                    if let Some(path) = file.path() {
                        let path_str = path.to_string_lossy().to_string();
                        script_entry_clone.set_text(&path_str);

                        // Perform auto-detection from selected script file!
                        if let Ok(content) = std::fs::read_to_string(&path) {
                            if let Some(detected_port) = Self::detect_port_from_script(&content) {
                                port_entry_clone.set_text(&detected_port.to_string());
                            }
                        }

                        let inferred_name = Self::infer_name_from_path(&path);
                        let inferred_id = Self::infer_id_from_path(&path);
                        if name_entry_clone.text().is_empty() {
                            name_entry_clone.set_text(&inferred_name);
                        }
                        if id_entry_clone.text().is_empty() {
                            id_entry_clone.set_text(&inferred_id);
                        }
                    }
                }
            }
            chooser.destroy();
        });

        chooser.show();
    }

    /// Extract a port number from script text by scanning for common patterns.
    fn detect_port_from_script(text: &str) -> Option<u16> {
        let mut last_detected: Option<u16> = None;

        for line in text.lines() {
            // Strip inline comments (anything after '#')
            let code_part = match line.find('#') {
                Some(idx) => &line[..idx],
                None => line,
            };

            let trimmed = code_part.trim();
            if trimmed.is_empty() {
                continue;
            }

            // Pattern 1: PORT=... or export PORT=...
            for prefix in ["PORT=", "export PORT="] {
                if let Some(rest) = trimmed.strip_prefix(prefix) {
                    if let Some(extracted) = Self::extract_port_number(rest) {
                        last_detected = Some(extracted);
                    }
                }
            }

            // Pattern 2: --port N or --port=N
            if let Some(idx) = trimmed.find("--port") {
                let after = &trimmed[idx + "--port".len()..];
                let after = after.trim_start_matches('=').trim();
                if let Some(extracted) = Self::extract_port_number(after) {
                    last_detected = Some(extracted);
                }
            }

            // Pattern 3: -p N
            let mut search_from = 0;
            while let Some(idx) = trimmed[search_from..].find(" -p ") {
                let abs_idx = search_from + idx;
                let after = &trimmed[abs_idx + 4..];
                if let Some(extracted) = Self::extract_port_number(after) {
                    if extracted >= 1024 {
                        last_detected = Some(extracted);
                    }
                }
                search_from = abs_idx + 4;
            }
        }

        last_detected
    }

    /// Helper to extract a 4+ digit port number from a raw token string.
    fn extract_port_number(s: &str) -> Option<u16> {
        let first_word = s.split_whitespace().next()?.trim_matches(|c| c == '"' || c == '\'');
        if let Ok(port) = first_word.parse::<u16>() {
            return Some(port);
        }

        if let Some(dash_idx) = s.find(":-") {
            let after_dash = &s[dash_idx + 2..];
            let digits: String = after_dash.chars().take_while(|c| c.is_ascii_digit()).collect();
            if let Ok(port) = digits.parse::<u16>() {
                if port >= 1024 {
                    return Some(port);
                }
            }
        }

        let digits: String = s.chars().filter(|c| c.is_ascii_digit()).collect();
        if let Ok(port) = digits.parse::<u16>() {
            if port >= 1024 {
                return Some(port);
            }
        }

        None
    }

    /// Infer a display name from a script filename.
    fn infer_name_from_path(path: &PathBuf) -> String {
        path.file_stem()
            .and_then(|s| s.to_str())
            .map(|s| {
                let words: Vec<String> = s
                    .replace(['-', '_'], " ")
                    .split_whitespace()
                    .filter(|w| !w.is_empty())
                    .map(|word| {
                        let mut chars = word.chars();
                        match chars.next() {
                            Some(c) => c.to_uppercase().collect::<String>() + &chars.as_str().to_lowercase(),
                            None => String::new(),
                        }
                    })
                    .collect();
                words.join(" ")
            })
            .unwrap_or_else(|| "New Model".to_string())
    }

    /// Infer a model ID slug from a script filename.
    fn infer_id_from_path(path: &PathBuf) -> String {
        path.file_stem()
            .and_then(|s| s.to_str())
            .map(|s| s.replace([' ', '_'], "-").to_lowercase())
            .unwrap_or_else(|| "new-model".to_string())
    }

    /// Validate inputs and return an `ImportedModel` if everything checks out.
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
                dialog.present();
                return;
            }
        };

        if let Some(port) = Self::detect_port_from_script(&content) {
            self.port_entry.set_text(&port.to_string());
        }

        let inferred_name = Self::infer_name_from_path(&path);
        if self.name_entry.text().is_empty() {
            self.name_entry.set_text(&inferred_name);
        }

        let inferred_id = Self::infer_id_from_path(&path);
        if self.id_entry.text().is_empty() {
            self.id_entry.set_text(&inferred_id);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detect_port_double_dash() {
        let script = "#!/bin/sh\nllama-server --port 8090 --model /models/llama.gguf\n";
        assert_eq!(ImportWizard::detect_port_from_script(script), Some(8090));
    }

    #[test]
    fn test_detect_port_equals() {
        let script = "#!/bin/sh\nexport PORT=8091\nllama-server --port $PORT\n";
        assert_eq!(ImportWizard::detect_port_from_script(script), Some(8091));
    }

    #[test]
    fn test_detect_port_short() {
        let script = "#!/bin/sh\nllama-server -p 8092 --model model.gguf\n";
        assert_eq!(ImportWizard::detect_port_from_script(script), Some(8092));
    }

    #[test]
    fn test_detect_port_comment_skipped() {
        let script = "#!/bin/sh\n# --port 12345\nllama-server --port 8090\n";
        assert_eq!(ImportWizard::detect_port_from_script(script), Some(8090));
    }

    #[test]
    fn test_detect_port_no_match() {
        let script = "#!/bin/sh\necho 'hello world'\n";
        assert_eq!(ImportWizard::detect_port_from_script(script), None);
    }

    #[test]
    fn test_detect_port_small_number_ignored() {
        let script = "#!/bin/sh\nllama-server -p 2 --model model.gguf\n";
        assert_eq!(ImportWizard::detect_port_from_script(script), None);
    }

    #[test]
    fn test_infer_name_from_path() {
        let p = PathBuf::from("/home/user/scripts/llama-7b-chat.sh");
        assert_eq!(ImportWizard::infer_name_from_path(&p), "Llama 7b Chat");
    }

    #[test]
    fn test_infer_id_from_path() {
        let p = PathBuf::from("/home/user/scripts/llama-7b-chat.sh");
        assert_eq!(ImportWizard::infer_id_from_path(&p), "llama-7b-chat");
    }

    #[test]
    fn test_infer_name_root_path() {
        let p = PathBuf::from("/");
        assert_eq!(ImportWizard::infer_name_from_path(&p), "New Model");
    }
}
