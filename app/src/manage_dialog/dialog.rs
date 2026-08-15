use gtk4 as gtk;
use gtk::prelude::*;
use gtk::{MessageType, ResponseType, Window};

use adw::prelude::*;

use swai_core::config::{Config, ModelConfig};
use swai_core::process_manager::ProcessManager;

use super::edit_dialog::show_edit_dialog;
use super::helpers::{launch_script_editor, show_error};
use crate::window::ImportMessage;

/// A non-blocking modal dialog listing all configured models.
#[derive(Clone)]
pub struct ManageModelsDialog {
    pub widget: gtk::Dialog,
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
        import_sender: std::sync::mpsc::Sender<ImportMessage>,
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
        import_sender: &std::sync::mpsc::Sender<ImportMessage>,
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

            show_edit_dialog(
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
            launch_script_editor(&script_path_for_edit);
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
        import_sender: &std::sync::mpsc::Sender<ImportMessage>,
    ) {
        // Check if the model is currently running.
        let is_running = {
            let pm = match process_manager.lock() {
                Ok(g) => g,
                Err(_) => {
                    show_error(
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
                                ImportMessage::ModelDeleted {
                                    id: id_clone.clone(),
                                },
                            );
                        }
                        Err(e) => {
                            show_error(
                                None::<&gtk::Window>,
                                &format!("Failed to delete model:\n\n{}", e),
                                "SWAI — Delete Error",
                            );
                        }
                    }
                }
                Err(_) => {
                    show_error(
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

}
