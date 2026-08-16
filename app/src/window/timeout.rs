use adw::ApplicationWindow;
use gtk::prelude::*;
use gtk4 as gtk;
use ksni::blocking::Handle;
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Arc, Mutex};

use swai_core::config::Config;
use swai_core::process_manager::{ModelState, ProcessManager};
use swai_core::proxy::ProxyState;

use crate::logs_panel::LogViewerWindow;
use crate::model_card::{CardState, ModelCard};
use crate::tray::{TrayAction, WindowAction};

use super::card_wiring::wire_card_handlers;
use super::footer::reorder_card_container;
use super::poller::notify;
use super::types::{ChannelMessage, ImportMessage, SlotUpdate};

pub struct TimeoutContext {
    pub cards: Rc<RefCell<Vec<ModelCard>>>,
    pub card_box: Rc<RefCell<gtk::Box>>,
    pub pm: Arc<Mutex<ProcessManager>>,
    pub widget: ApplicationWindow,
    pub tray_handle: Rc<RefCell<Option<Handle<crate::tray::SwaiTray>>>>,
    pub current_keep_alive: Rc<RefCell<Option<Arc<AtomicBool>>>>,
    pub proxy_state: Option<Arc<Mutex<ProxyState>>>,
    pub log_viewer: Rc<RefCell<Option<LogViewerWindow>>>,
    pub config: Config,
    pub footer_model_label: gtk::Label,
    pub sender: Sender<ChannelMessage>,
    pub receiver: Receiver<ChannelMessage>,
    pub slot_receiver: Receiver<SlotUpdate>,
    pub window_receiver: Receiver<WindowAction>,
    pub tray_receiver: Receiver<TrayAction>,
    pub quit_receiver: Receiver<()>,
    pub import_receiver: Receiver<ImportMessage>,
}

pub fn attach_timeout_handler(ctx: TimeoutContext) {
    let cards_clone = Rc::clone(&ctx.cards);
    let card_box = Rc::clone(&ctx.card_box);
    let pm_timeout = Arc::clone(&ctx.pm);
    let widget_timeout = ctx.widget.clone();
    let tray_handle_timeout = Rc::clone(&ctx.tray_handle);
    let current_keep_alive = Rc::clone(&ctx.current_keep_alive);
    let proxy_state = ctx.proxy_state.clone();
    let log_viewer = Rc::clone(&ctx.log_viewer);
    let config = ctx.config.clone();
    let footer_model_label = ctx.footer_model_label.clone();
    let sender = ctx.sender.clone();
    let receiver = ctx.receiver;
    let slot_receiver = ctx.slot_receiver;
    let window_receiver = ctx.window_receiver;
    let tray_receiver = ctx.tray_receiver;
    let quit_receiver = ctx.quit_receiver;
    let import_receiver = ctx.import_receiver;

    let pm_for_import = Arc::clone(&pm_timeout);
    let proxy_state_for_import = Rc::new(proxy_state.clone());
    let config_for_import = config.clone();
    let current_keep_alive_for_handlers = Rc::clone(&current_keep_alive);
    let sender_for_handlers = sender.clone();
    let log_viewer_for_closure = Rc::clone(&log_viewer);

    glib::timeout_add_local(std::time::Duration::from_millis(50), move || {
        let mut cards_borrow = cards_clone.borrow_mut();

        while let Ok(msg) = receiver.try_recv() {
            match msg {
                ChannelMessage::SwitchCompleted { target_id, result } => {
                    for c in cards_borrow.iter_mut() {
                        let cid = c.config().id.clone();
                        if cid == target_id {
                            match &result {
                                Ok(()) => c.set_state(CardState::Ready),
                                Err(e) => {
                                    c.set_state(CardState::Error(format!(
                                        "Failed to start: {}",
                                        e
                                    )));
                                    if config.enable_notifications() {
                                        notify(
                                            "SWAI - Model Error",
                                            "Failed to start model - process exited with error",
                                        );
                                    }
                                }
                            }
                        } else {
                            let is_running = pm_timeout
                                .lock()
                                .ok()
                                .map(|pm| pm.find_running_model(&cid).is_some())
                                .unwrap_or(false);
                            if !is_running {
                                c.set_state(CardState::Stopped);
                            }
                        }
                        c.enable_toggle();
                        c.enable_restart();
                    }
                    if let Some(ref handle) = *tray_handle_timeout.borrow() {
                        handle.update(|_| {});
                    }
                    reorder_card_container(&cards_borrow);

                    if let Some(active_card) = cards_borrow.iter().find(|c| {
                        matches!(
                            c.state(),
                            CardState::Ready | CardState::Starting | CardState::Loading
                        )
                    }) {
                        footer_model_label
                            .set_text(&format!("{} active", active_card.config().name));
                        footer_model_label.set_css_classes(&["accent-label"]);
                    } else {
                        footer_model_label
                            .set_text(&format!("SWAI v{}", env!("CARGO_PKG_VERSION")));
                        footer_model_label.set_css_classes(&["dim-label"]);
                    }

                    if result.is_ok() && config.auto_follow_logs() {
                        if let Some(ref log_viewer) = *log_viewer.borrow() {
                            log_viewer.select_model_by_id(&target_id);
                        }
                    }

                    if result.is_ok() && config.enable_notifications() && config.notify_on_switch()
                    {
                        let model_name = cards_borrow
                            .iter()
                            .find(|c| c.config().id == target_id)
                            .map(|c| c.config().name.clone())
                            .unwrap_or_else(|| target_id.clone());
                        notify("SWAI", &format!("Switched to {} (Ready)", model_name));
                    }
                }
                ChannelMessage::StopCompleted { running_id, result } => {
                    for c in cards_borrow.iter_mut() {
                        if c.config().id == running_id {
                            match &result {
                                Ok(()) => c.set_state(CardState::Stopped),
                                Err(e) => {
                                    c.set_state(CardState::Error(format!("Failed to stop: {}", e)))
                                }
                            }
                            c.enable_toggle();
                            c.enable_restart();
                        }
                    }
                    if let Some(ref handle) = *tray_handle_timeout.borrow() {
                        handle.update(|_| {});
                    }
                    reorder_card_container(&cards_borrow);

                    if let Some(active_card) = cards_borrow.iter().find(|c| {
                        matches!(
                            c.state(),
                            CardState::Ready | CardState::Starting | CardState::Loading
                        )
                    }) {
                        footer_model_label
                            .set_text(&format!("{} active", active_card.config().name));
                        footer_model_label.set_css_classes(&["accent-label"]);
                    } else {
                        footer_model_label
                            .set_text(&format!("SWAI v{}", env!("CARGO_PKG_VERSION")));
                        footer_model_label.set_css_classes(&["dim-label"]);
                    }
                }
                ChannelMessage::RestartRequested { model_id } => {
                    for c in cards_borrow.iter_mut() {
                        if c.config().id == model_id {
                            c.disable_restart();
                        } else {
                            c.set_state(CardState::Stopped);
                        }
                        c.enable_toggle();
                    }
                }
                ChannelMessage::StateUpdate { model_id, state } => {
                    let mut needs_toggle_enable = false;
                    let mut ready_model_name: Option<String> = None;
                    let mut error_info: Option<(String, String)> = None;

                    for c in cards_borrow.iter_mut() {
                        if c.config().id == model_id {
                            match &state {
                                ModelState::Starting => c.set_state(CardState::Starting),
                                ModelState::Loading => c.set_state(CardState::Loading),
                                ModelState::Ready => {
                                    c.set_state(CardState::Ready);
                                    needs_toggle_enable = true;
                                    ready_model_name = Some(c.config().name.clone());
                                }
                                ModelState::Error(msg) => {
                                    c.set_state(CardState::Error(format!(
                                        "Failed to load: {}",
                                        msg
                                    )));
                                    needs_toggle_enable = true;
                                    error_info = Some((c.config().name.clone(), msg.clone()));
                                }
                                _ => {}
                            }
                        }
                    }
                    if needs_toggle_enable {
                        for card in cards_borrow.iter_mut() {
                            card.enable_toggle();
                            card.enable_restart();
                        }
                    }

                    if let Some(ref handle) = *tray_handle_timeout.borrow() {
                        handle.update(|_| {});
                    }

                    if let Some(active_card) = cards_borrow.iter().find(|c| {
                        matches!(
                            c.state(),
                            CardState::Ready | CardState::Starting | CardState::Loading
                        )
                    }) {
                        footer_model_label
                            .set_text(&format!("{} active", active_card.config().name));
                        footer_model_label.set_css_classes(&["accent-label"]);
                    } else {
                        footer_model_label
                            .set_text(&format!("SWAI v{}", env!("CARGO_PKG_VERSION")));
                        footer_model_label.set_css_classes(&["dim-label"]);
                    }

                    if let Some(name) = ready_model_name {
                        if config.enable_notifications() && config.notify_on_switch() {
                            notify("SWAI", &format!("{} is now Ready", name));
                        }
                    } else if let Some((name, err)) = error_info {
                        if config.enable_notifications() {
                            notify(
                                "SWAI - Model Error",
                                &format!("Failed to load {}: {}", name, err),
                            );
                        }
                    }
                }
            }
        }

        while let Ok(update) = slot_receiver.try_recv() {
            for c in cards_borrow.iter_mut() {
                if c.config().id == update.model_id {
                    c.set_context(update.tokens_used, update.n_ctx);
                    if matches!(c.state(), CardState::Ready) {
                        c.set_speed(update.predicted_per_second);
                    }
                }
            }
        }

        while let Ok(action) = window_receiver.try_recv() {
            match action {
                WindowAction::Hide => widget_timeout.hide(),
                WindowAction::Show => {
                    widget_timeout.show();
                    widget_timeout.present();
                }
            }
        }

        while let Ok(action) = tray_receiver.try_recv() {
            match action {
                TrayAction::Switch(target_id) => {
                    if let Some(ref old_ka) = *current_keep_alive.borrow() {
                        old_ka.store(false, Ordering::SeqCst);
                    }

                    let new_ka = Arc::new(AtomicBool::new(true));
                    *current_keep_alive.borrow_mut() = Some(Arc::clone(&new_ka));

                    let bg_pm = Arc::clone(&pm_timeout);
                    let bg_sender = sender.clone();
                    let bg_target_id = target_id.clone();
                    let bg_ka = Arc::clone(&new_ka);

                    std::thread::spawn(move || {
                        let result = {
                            let mut pm_lock = match bg_pm.lock() {
                                Ok(g) => g,
                                Err(_) => return,
                            };

                            if let Some(running_id) = pm_lock.get_primary_model_id() {
                                let running_id = running_id.to_string();
                                if running_id == bg_target_id {
                                    return;
                                }
                                pm_lock.switch_model(&running_id, &bg_target_id)
                            } else {
                                pm_lock.start_model(&bg_target_id)
                            }
                        };

                        let is_ok = result.is_ok();
                        if !is_ok {
                            let _ = bg_sender.send(ChannelMessage::SwitchCompleted {
                                target_id: bg_target_id,
                                result,
                            });
                        }

                        if is_ok {
                            while bg_ka.load(Ordering::SeqCst) {
                                std::thread::sleep(std::time::Duration::from_millis(100));
                            }
                        }
                    });
                }
            }
        }

        if quit_receiver.try_recv().is_ok() {
            tracing::info!("quit signal received from tray - prompting confirmation");
            widget_timeout.show();
            widget_timeout.present();
            widget_timeout.close();
        }

        while let Ok(msg) = import_receiver.try_recv() {
            match msg {
                ImportMessage::ModelImported { model } => {
                    pm_for_import
                        .lock()
                        .unwrap_or_else(|e| {
                            tracing::error!(
                                "import: process manager lock poisoned, skipping model add"
                            );
                            e.into_inner()
                        })
                        .add_model(model.clone());

                    let mut card = ModelCard::new(&model);
                    card_box.borrow_mut().append(&card.widget);

                    wire_card_handlers(
                        &mut card,
                        &cards_clone,
                        &pm_for_import,
                        &proxy_state_for_import,
                        &sender_for_handlers,
                        &current_keep_alive_for_handlers,
                        &log_viewer_for_closure,
                        &config_for_import,
                    );

                    cards_borrow.push(card);
                    tracing::info!(
                        "Appended card for newly imported model '{}' (port {})",
                        model.id,
                        model.port
                    );
                }
                ImportMessage::ModelNameUpdated {
                    id: updated_id,
                    name: new_name,
                    port: new_port,
                } => {
                    for c in cards_borrow.iter_mut() {
                        if c.config().id == updated_id {
                            c.update_model(&new_name, new_port);
                        }
                    }
                }
                ImportMessage::ModelDeleted { id: deleted_id } => {
                    let len_before = cards_borrow.len();
                    cards_borrow.retain(|c| c.config().id != deleted_id);
                    if cards_borrow.len() < len_before {
                        tracing::info!("Removed card for deleted model '{}'", deleted_id);
                        reorder_card_container(&cards_borrow);
                    }
                }
            }
        }

        glib::ControlFlow::Continue
    });
}
