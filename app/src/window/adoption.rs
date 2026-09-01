use adw::ApplicationWindow;
use gtk::prelude::*;
use gtk::{MessageDialog, MessageType, ResponseType};
use gtk4 as gtk;

use super::dialogs::append_model_to_config;
use super::types::ImportMessage;
use crate::import_wizard::ImportWizard;

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
                Ok(imported) => match append_model_to_config(&imported) {
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
                            ctx_size: 65_536,
                        };
                        let _ = sender_clone.send(ImportMessage::ModelImported {
                            model: model_config,
                        });
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
                },
                Err(e) => {
                    tracing::error!("Adoption failed: {}", e);
                }
            }
        }
        d.destroy();
    });

    wizard.widget.show();
}

/// Detect and restore already running models into ProcessManager on boot.
pub fn restore_running_models(
    pm: &std::sync::Arc<std::sync::Mutex<swai_core::process_manager::ProcessManager>>,
    cards: &std::rc::Rc<std::cell::RefCell<Vec<crate::model_card::ModelCard>>>,
) {
    let mut restored_model_id = None;
    {
        let mut pm_guard = pm.lock().unwrap_or_else(|e| e.into_inner());
        let mut running_model_found = None;
        for model in pm_guard.config().models.iter() {
            if matches!(
                swai_core::process_manager::ProcessManager::check_port(model.port),
                swai_core::process_manager::PortState::OccupiedByModel
            ) {
                let pid = swai_core::process_manager::ProcessManager::get_port_pid(model.port).ok();
                running_model_found = Some((model.clone(), pid));
                break;
            }
        }
        if let Some((model, pid)) = running_model_found {
            restored_model_id = Some(model.id.clone());
            let guard = swai_core::process_manager::LinuxProcessGuard {
                pid: pid.map(|p| swai_core::process_manager::Pid::from_raw(p as i32)),
                port: model.port,
                shutdown_timeout_sec: 10,
            };
            pm_guard.set_running_model(swai_core::process_manager::RunningModel {
                id: model.id.clone(),
                guard: Box::new(guard),
                state: swai_core::process_manager::ModelState::Ready,
            });
        }
    }

    if let Some(restored_id) = restored_model_id {
        for c in cards.borrow_mut().iter_mut() {
            if c.config().id == restored_id {
                c.set_state(crate::model_card::CardState::Ready);
            }
        }
    }
}
