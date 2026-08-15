use gtk4 as gtk;
use gtk::prelude::*;
use gtk::{MessageDialog, MessageType, ResponseType};
use adw::ApplicationWindow;

use crate::import_wizard::ImportWizard;
use super::dialogs::append_model_to_config;
use super::types::ImportMessage;

pub fn build_adoption_banner(
    message: &str,
    _port: u16,
    _model_name: String,
    _parent: &ApplicationWindow,
) -> adw::Banner {
    let banner = adw::Banner::new(message);
    banner.set_button_label(Some("Adopt"));
    banner.set_css_classes(&["adoption-banner"]);
    banner
}

pub fn show_adopt_model_dialog(
    parent: &ApplicationWindow,
    import_sender: &std::sync::mpsc::Sender<ImportMessage>,
    port: u16,
    model_name: String,
) {
    let wizard = ImportWizard::new(parent, Some(port));
    wizard.set_display_name(&model_name);

    let parent_clone = parent.clone();
    let sender_clone = import_sender.clone();
    let wizard_clone = wizard.clone();

    wizard.widget.connect_response(move |d, response| {
        if response == ResponseType::Ok {
            match wizard_clone.try_import() {
                Ok(imported) => {
                    match append_model_to_config(&imported) {
                        Ok(()) => {
                            tracing::info!(
                                "Model '{}' adopted successfully (port {})",
                                imported.id,
                                imported.port
                            );
                            let model_config = swai_core::config::ModelConfig {
                                id: imported.id.clone(),
                                name: imported.name.clone(),
                                script_path: imported.script_path.clone(),
                                port: imported.port,
                                health_timeout_sec: imported.health_timeout_sec,
                            };
                            let _ = sender_clone.send(ImportMessage::ModelImported { model: model_config });
                        }
                        Err(e) => {
                            let error_dialog = MessageDialog::new(
                                Some(&parent_clone),
                                gtk::DialogFlags::MODAL,
                                MessageType::Error,
                                gtk::ButtonsType::Close,
                                format!("Failed to save model:\n\n{}", e),
                            );
                            error_dialog.set_title(Some("SWAI - Save Error"));
                            error_dialog.connect_response(|ed, _| ed.destroy());
                            error_dialog.present();
                        }
                    }
                }
                Err(e) => {
                    let error_dialog = MessageDialog::new(
                        Some(&parent_clone),
                        gtk::DialogFlags::MODAL,
                        MessageType::Error,
                        gtk::ButtonsType::Close,
                        format!("Validation error:\n\n{}", e),
                    );
                    error_dialog.set_title(Some("SWAI - Import Error"));
                    error_dialog.connect_response(|ed, _| ed.destroy());
                    error_dialog.present();
                }
            }
        }
        d.destroy();
    });

    wizard.widget.show();
}
