use gtk4 as gtk;
use gtk::prelude::*;
use gtk::{MessageDialog, MessageType, Orientation, ResponseType};
use adw::prelude::*;
use adw::{AboutDialog, ApplicationWindow};
use std::sync::{Arc, Mutex};

use swai_core::config::Config;
use swai_core::process_manager::ProcessManager;

use crate::import_wizard::{ImportedModel, ImportWizard};
use crate::manage_dialog::ManageModelsDialog;
use crate::preferences::{PreferencesDialog, PreferencesValues};
use super::types::ImportMessage;

pub fn show_preferences_dialog(
    parent: &ApplicationWindow,
    process_manager: &Arc<Mutex<ProcessManager>>,
) {
    let config = match Config::load() {
        Ok(cfg) => cfg,
        Err(e) => {
            let dialog = MessageDialog::new(
                Some(parent),
                gtk::DialogFlags::MODAL,
                MessageType::Error,
                gtk::ButtonsType::Close,
                format!("Failed to load config:\n\n{}", e),
            );
            dialog.set_title(Some("SWAI - Config Error"));
            dialog.connect_response(|d, _| d.destroy());
            dialog.present();
            return;
        }
    };

    let dialog = PreferencesDialog::new(parent, &config);
    let config_path = Config::resolve_path().unwrap_or_else(|| {
        std::path::PathBuf::from("/nonexistent/config.toml")
    });
    let parent_clone = parent.clone();
    let pm_clone = Arc::clone(process_manager);
    let dialog_clone = dialog.clone();

    dialog.widget.connect_response(move |d, response| {
        if response == ResponseType::Ok {
            let values = dialog_clone.values();
            match save_preferences(&values, &config_path) {
                Ok(()) => {
                    tracing::info!("Preferences saved successfully");
                    if let Ok(new_cfg) = Config::load() {
                        if let Ok(mut pm) = pm_clone.lock() {
                            pm.update_config(new_cfg);
                            tracing::info!("Updated ProcessManager config in memory");
                        }
                    }
                }
                Err(e) => {
                    let error_dialog = MessageDialog::new(
                        Some(&parent_clone),
                        gtk::DialogFlags::MODAL,
                        MessageType::Error,
                        gtk::ButtonsType::Close,
                        &e,
                    );
                    error_dialog.set_title(Some("SWAI - Save Error"));
                    error_dialog.connect_response(|ed, _| ed.destroy());
                    error_dialog.present();
                }
            }
        }
        d.destroy();
    });

    dialog.widget.show();
}

pub fn show_manage_models_dialog(
    parent: &ApplicationWindow,
    import_sender: &std::sync::mpsc::Sender<ImportMessage>,
    process_manager: &Arc<Mutex<ProcessManager>>,
) {
    let dialog = ManageModelsDialog::new(parent, import_sender.clone(), Arc::clone(process_manager));
    dialog.widget.connect_response(|d, _| {
        d.destroy();
    });
    dialog.widget.show();
}

pub fn show_add_model_dialog(
    parent: &ApplicationWindow,
    import_sender: &std::sync::mpsc::Sender<ImportMessage>,
) {
    let wizard = ImportWizard::new(parent, None);
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
                                "Model '{}' added successfully (port {})",
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

pub fn append_model_to_config(model: &ImportedModel) -> Result<(), String> {
    append_model_to_config_at(&Config::resolve_path().ok_or_else(|| -> String {
        "No config file found. Please create one at ~/.config/swai/config.toml first.".to_string()
    })?, model)
}

pub fn append_model_to_config_at(
    config_path: &std::path::Path,
    model: &ImportedModel,
) -> Result<(), String> {
    let content = std::fs::read_to_string(config_path)
        .map_err(|e| format!("Failed to read config: {}", e))?;

    let mut config: Config = toml::from_str(&content)
        .map_err(|e| format!("Failed to parse config: {}", e))?;

    config.models.push(swai_core::config::ModelConfig {
        id: model.id.clone(),
        name: model.name.clone(),
        script_path: model.script_path.clone(),
        port: model.port,
        health_timeout_sec: model.health_timeout_sec,
    });

    Config::validate(&config, config_path)
        .map_err(|e| format!("Config validation error: {}", e))?;

    let new_content = toml::to_string_pretty(&config)
        .map_err(|e| format!("Failed to serialize config: {}", e))?;
    std::fs::write(config_path, &new_content)
        .map_err(|e| format!("Failed to write config file: {}", e))?;

    Ok(())
}

pub fn save_preferences(
    values: &PreferencesValues,
    config_path: &std::path::Path,
) -> Result<(), String> {
    let mut config = Config::load().map_err(|e| format!("Failed to load config: {}", e))?;
    config.global.log_dir = values.log_dir.clone();
    config.global.proxy_port = values.proxy_port;
    config.global.auto_restart_on_context_full = Some(values.auto_restart_on_context_full);
    config.global.auto_follow_logs = Some(values.auto_follow_logs);
    config.preferences.enable_notifications = values.enable_notifications;
    config.preferences.notify_on_switch = values.notify_on_switch;
    config.preferences.autostart_on_login = values.autostart_on_login;
    config.preferences.max_concurrent_models = values.max_concurrent_models;

    Config::validate(&config, config_path).map_err(|e| format!("Config validation error: {}", e))?;

    let content = toml::to_string_pretty(&config)
        .map_err(|e| format!("Failed to serialize config: {}", e))?;
    std::fs::write(config_path, &content)
        .map_err(|e| format!("Failed to write config file: {}", e))?;

    if values.autostart_on_login {
        swai_core::autostart::enable_autostart()
            .map_err(|e| format!("Failed to enable autostart: {}", e))?;
    } else {
        swai_core::autostart::disable_autostart()
            .map_err(|e| format!("Failed to disable autostart: {}", e))?;
    }

    Ok(())
}

pub fn show_about_dialog(parent: &ApplicationWindow) {
    let version = env!("CARGO_PKG_VERSION");
    let about_dialog = AboutDialog::builder()
        .application_name("SWAI")
        .version(version)
        .comments(
            "Native Linux desktop app for starting, stopping, and \
             monitoring local llama.cpp model servers.",
        )
        .license_type(gtk::License::MitX11)
        .website("https://github.com/verdioso/swai")
        .developers(vec!["SWAI contributors"])
        .build();

    about_dialog.add_link("GitHub", "https://github.com/verdioso/swai");
    about_dialog.present(Some(parent));
}

pub fn show_check_updates_dialog(parent: &ApplicationWindow) {
    let version = env!("CARGO_PKG_VERSION");
    let parent_win = parent.clone();
    let parent_for_response = parent_win.clone();
    let check_btn = gtk::Button::builder()
        .label("Check for Updates…")
        .css_classes(vec!["suggested-action"])
        .margin_top(12)
        .build();

    let check_dialog = gtk::Dialog::builder()
        .title("SWAI - Check for Updates")
        .transient_for(&parent_win)
        .modal(true)
        .build();

    let check_content = gtk::Box::new(Orientation::Vertical, 12);
    check_content.set_margin_start(24);
    check_content.set_margin_end(24);
    check_content.set_margin_top(24);
    check_content.set_margin_bottom(24);

    let info_label = gtk::Label::builder()
        .label(&format!(
            "You are currently running SWAI v{}.\n\n\
             Click the button below to check for updates.",
            version,
        ))
        .wrap(true)
        .halign(gtk::Align::Start)
        .build();
    check_content.append(&info_label);
    check_content.append(&check_btn);

    check_dialog.content_area().append(&check_content);
    check_dialog.add_button("_Cancel", ResponseType::Cancel);
    check_dialog.connect_response(|d, _| {
        d.destroy();
    });

    check_btn.connect_clicked(move |btn| {
        btn.set_sensitive(false);
        btn.set_label("Checking…");

        let parent_resp = parent_for_response.clone();

        let result = crate::update_checker::check_for_updates_blocking(
            "verdioso/swai",
            version,
        );

        match result {
            crate::update_checker::UpdateCheckResult::UpdateAvailable { version, .. } => {
                let dlg = gtk::MessageDialog::new(
                    Some(&parent_resp),
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

                dlg.connect_response(move |d, response| {
                    if response == ResponseType::Ok {
                        let install_result =
                            crate::update_installer::install_update(
                                "verdioso/swai",
                                &version,
                            );
                        match install_result {
                            crate::update_installer::UpdateInstallResult::Success {
                                new_version,
                            } => {
                                let notif = gtk::MessageDialog::new(
                                    Some(&parent_resp),
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
                                    Some(&parent_resp),
                                    gtk::DialogFlags::MODAL,
                                    gtk::MessageType::Error,
                                    gtk::ButtonsType::Close,
                                    &format!("Update failed:\n\n{}", e),
                                );
                                err.set_title(Some("SWAI - Update Error"));
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
                    Some(&parent_resp),
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
                    Some(&parent_resp),
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

    check_dialog.present();
}
