//! Main application window for SWAI.
//!
//! Renders the AdwHeaderBar, boxed-list model cards, proxy status, and footer.

use gtk4 as gtk;
use gtk::prelude::*;
use gtk::{
    Application, Box as GtkBox, Button, Label,
    MessageDialog, MessageType, Orientation, ResponseType, ScrolledWindow,
};
use adw::prelude::*;
use adw::{AboutDialog, HeaderBar};

use gio::{Menu, Notification};
use std::rc::Rc;
use std::cell::RefCell;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use ksni::blocking::Handle;

use crate::import_wizard::{ImportedModel, ImportWizard};
use crate::logs_panel::LogViewerWindow;
use crate::manage_dialog::ManageModelsDialog;
use crate::menu;
use crate::model_card::{CardState, ModelCard};
use crate::preferences::PreferencesDialog;
use crate::tray::{TrayAction, WindowAction};
use swai_core::config::Config;
use swai_core::process_manager::{ModelState, ProcessError, PortState, ProcessManager, Pid};
use swai_core::proxy::ProxyState;
use swai_core::reconciler::Reconciler;

/// Messages sent from background threads to the main GUI thread.
enum ChannelMessage {
    /// A switch (start or switch_model) completed.
    SwitchCompleted { target_id: String, result: Result<(), ProcessError> },
    /// A stop completed.
    StopCompleted { running_id: String, result: Result<(), ProcessError> },
    /// A restart was manually triggered by the user via the Restart button.
    RestartRequested { model_id: String },
    /// Intermediate state update from health monitor polling.
    /// Used to drive Starting → Loading → Ready transitions in the UI.
    StateUpdate { model_id: String, state: ModelState },
}

/// Messages sent from UI dialogs (import wizard) to the main GUI thread.
#[allow(dead_code)]
pub enum ImportMessage {
    /// A new model was imported and its card should be appended.
    ModelImported { model: swai_core::config::ModelConfig },
    /// A model's details (name, port) were updated - refresh the card label live.
    ModelNameUpdated { id: String, name: String, port: u16 },
    /// A model was deleted - remove its card from the UI.
    ModelDeleted { id: String },
}

/// Context update sent from the polling thread to the main loop.
struct SlotUpdate {
    model_id: String,
    tokens_used: usize,
    n_ctx: usize,
    predicted_per_second: f64,
    #[allow(dead_code)]
    prompt_per_second: f64,
}

/// A polled /slots response for a single model.
struct SlotInfo {
    tokens_used: usize,
    n_ctx: usize,
    predicted_per_second: f64,
    prompt_per_second: f64,
}

/// The main application window.
#[allow(dead_code)]
pub struct MainWindow {
    widget: adw::ApplicationWindow,
    cards: Rc<RefCell<Vec<ModelCard>>>,
    /// Tracks the keep-alive signal for the active background thread.
    current_keep_alive: Rc<RefCell<Option<Arc<AtomicBool>>>>,
    /// Shared config - needed by the context polling thread and preferences.
    config: Config,
    /// Path to the config file on disk (for saving preferences).
    config_path: std::path::PathBuf,
    /// Proxy state - updated when a model starts/stops so the reverse proxy
    /// knows where to forward incoming requests.
    proxy_state: Option<Arc<Mutex<ProxyState>>>,
    /// The currently open log viewer window (if any).
    log_viewer: Rc<RefCell<Option<LogViewerWindow>>>,
    /// Flag set when the user clicks the window close (X) button.
    close_requested: Rc<RefCell<bool>>,
    /// Clone of the ProcessManager Arc, needed for clean shutdown in quit().
    process_manager: Arc<Mutex<ProcessManager>>,
    /// ksni tray handle for refreshing the tray menu/tooltip after state changes.
    tray_handle: Option<Handle<crate::tray::SwaiTray>>,
    /// Whether a system-tray host (StatusNotifierWatcher) is available on this desktop.
    tray_host_available: bool,
    /// Sender for import messages from the wizard dialog.
    import_sender: std::sync::mpsc::Sender<ImportMessage>,
    /// Footer proxy label (updated when config changes).
    footer_proxy_label: gtk::Label,
    /// Footer model/active label (updated when active model changes).
    footer_model_label: gtk::Label,
    /// Banner row shown at the top of the content area when an unmanaged
    /// local LLM server is detected. Contains an "Adopt" action button that
    /// opens the Add Model dialog pre-filled with the discovered port.
    unmanaged_banner: Option<adw::Banner>,
}

impl MainWindow {
    pub fn new(app: &Application, config: Config, proxy_state: Option<Arc<Mutex<ProxyState>>>) -> Self {
        // Set default icon name for all windows in GTK
        gtk::Window::set_default_icon_name("swai");

        // ── Custom CSS provider with brand design tokens ───────────
        let css = r#"
            /* ── Kill every possible headerbar underline source ────────── */
            headerbar, headerbar:backdrop {
                box-shadow: none;
                border: none;
                border-bottom: none;
                border-bottom-width: 0;
                border-bottom-style: none;
                border-bottom-color: transparent;
            }
            .titlebar, .titlebar:backdrop {
                box-shadow: none;
                border-bottom: none;
                border-bottom-width: 0;
            }
            .titlebar separator,
            headerbar separator {
                min-height: 0;
                opacity: 0;
                background: transparent;
            }
            window.background > decoration {
                box-shadow: none;
            }

            /* ── Model cards: rounded with subtle native border ────────── */
            .card, .card-active {
                background-color: alpha(@theme_fg_color, 0.06);
                border: 1px solid alpha(@theme_fg_color, 0.12);
                border-radius: 12px;
                padding: 12px;
            }
            .card-active {
                background-color: alpha(@theme_fg_color, 0.08);
                border-left: 3px solid #2dd4f0;
            }

            /* ── Switch: pill toggle matching button.png design ────────── */
            switch,
            switch:hover,
            switch:active,
            switch:backdrop {
                border-radius: 10px; /* 20px height / 2 = 10px */
                min-width: 40px;
                min-height: 20px;
                padding: 0;
                border: none;
                background-image: none;
                background-color: alpha(@theme_fg_color, 0.25);
                box-shadow: none;
                -gtk-icon-source: none;
                outline: none;
            }
            switch:checked,
            switch:checked:hover,
            switch:checked:active {
                background-image: none;
                background-color: #2dd4f0;
                box-shadow: none;
            }
            switch slider,
            switch:hover slider,
            switch:active slider,
            switch:checked slider,
            switch:checked:hover slider {
                border-radius: 50%;
                min-width: 16px;
                min-height: 16px;
                padding: 0;
                margin: 2px; /* GTK allocates 20x20 for slider; 2px margin on all sides forces slider to 16x16 */
                border: none;
                background-image: none;
                background-color: @theme_bg_color;
                box-shadow: none;
                -gtk-icon-source: none;
                -gtk-icon-transform: none;
                outline: none;
                opacity: 1;
            }
            switch:checked slider,
            switch:checked:hover slider {
                background-image: none;
                background-color: #ffffff;
            }

            /* ── Progress bar: 4-tier context coloring ─────────────────── */
            progressbar trough {
                min-height: 4px;
                border-radius: 2px;
            }
            progressbar progress {
                min-height: 4px;
                border-radius: 2px;
            }
            progressbar.ctx-green progress  { background-color: #4ade80; }
            progressbar.ctx-cyan progress   { background-color: #2dd4f0; }
            progressbar.ctx-orange progress { background-color: #f59e0b; }
            progressbar.ctx-red progress    { background-color: #ef4444; }

            /* Context label color tiers */
            .ctx-text-green  { color: #4ade80; }
            .ctx-text-cyan   { color: #2dd4f0; }
            .ctx-text-orange { color: #f59e0b; }
            .ctx-text-red    { color: #ef4444; }

            /* ── Accent label (footer active model name) ───────────────── */
            .accent-label {
                color: #2dd4f0;
            }
        "#;

        let provider = gtk::CssProvider::new();
        provider.load_from_data(css);

        if let Some(display) = adw::gdk::Display::default() {
            gtk::style_context_add_provider_for_display(
                &display,
                &provider,
                gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
            );
        }

        // ── AdwApplicationWindow with native menu bar support ───────
        let widget = adw::ApplicationWindow::builder()
            .application(app)
            .title("SWAI")
            .icon_name("swai")
            .default_width(640)
            .default_height(520)
            .build();

        // ── AdwHeaderBar with menubar (left) + action buttons (right) ──
        let header_bar = Self::build_header_bar(app);

        // ── Main content area (header bar + scrollable cards + footer bar) ──
        let main_vbox = GtkBox::new(Orientation::Vertical, 0);
        main_vbox.append(&header_bar);

        // Build the card container.
        let card_box = Rc::new(RefCell::new(
            GtkBox::new(Orientation::Vertical, 12)
        ));
        {
            let bx = card_box.borrow_mut();
            bx.set_margin_start(16);
            bx.set_margin_end(16);
            bx.set_margin_top(16);
            bx.set_margin_bottom(16);
        }

        let footer_card_box = Rc::clone(&card_box);

        let cards = Rc::new(RefCell::new(
            config.models.iter().map(|m| {
                let card = ModelCard::new(m);
                card_box.borrow_mut().append(&card.widget);
                card
            }).collect::<Vec<_>>()
        ));

        Self::reorder_card_container(&cards.borrow());

        // Build the scrollable container.
        let cards_scroll = Self::build_cards_container(&card_box.borrow());

        // Clone for the timeout closure (moved into it below).
        let _cards_scroll_for_timeout = cards_scroll.clone();

        main_vbox.append(&cards_scroll);

        // ── Footer bar (dynamic proxy port + active model name) ─────
        let (footer_bar, footer_proxy_label, footer_model_label) =
            Self::build_footer_bar(config.proxy_port());

        // Clone footer labels before moving into the timeout closure.
        let footer_model_label_clone = footer_model_label.clone();

        main_vbox.append(&footer_bar);

        widget.set_content(Some(&main_vbox));

        // Resolve config file path for preferences saving.
        let config_path = Config::resolve_path().unwrap_or_else(|| {
            std::path::PathBuf::from("/nonexistent/config.toml")
        });

        // Shared process manager.
        let pm = Arc::new(Mutex::new(
            ProcessManager::new(config.clone()),
        ));

        // Phase 16.2: Probe for unmanaged local LLM servers and build an
        // adoption banner if any are found. The banner sits above the card
        // container and offers a one-click "Adopt" workflow.
        let reconciler = Reconciler::new(config.clone());
        let unmanaged_servers = reconciler.probe_unmanaged_servers();
        let (unmanaged_banner, adopt_port, adopt_model_name) = if !unmanaged_servers.is_empty() {
            let first = &unmanaged_servers[0];
            let banner = Self::build_adoption_banner(
                &format!(
                    "Unmanaged local model detected on port {} ({})",
                    first.port, first.model_name
                ),
                first.port,
                first.model_name.clone(),
                &widget,
            );
            (Some(banner), Some(first.port), Some(first.model_name.clone()))
        } else {
            (None, None, None)
        };

        // Insert the adoption banner into the main box if one was created.
        if let Some(ref banner) = unmanaged_banner {
            main_vbox.insert_before(banner, Some(&cards_scroll));
        }

        // Phase 20: Check for SWAI updates in the background. If a newer
        // version is available, show an `adw::Banner` at the top of the
        // content area with a "Download & Install" action button.
        let current_version = env!("CARGO_PKG_VERSION").to_string();
        Self::check_for_update_background("verdioso/swai", &current_version);

        let current_keep_alive = Rc::new(RefCell::new(None::<Arc<AtomicBool>>));

       // Reconcile: detect any models already running from a previous session.
        let mut restored_model_id = None;
        {
            let mut pm_guard = pm.lock().unwrap_or_else(|e| e.into_inner());
            let mut running_model_found = None;
            for model in pm_guard.config().models.iter() {
                if matches!(ProcessManager::check_port(model.port), PortState::OccupiedByModel) {
                    let pid = ProcessManager::get_port_pid(model.port).ok();
                    running_model_found = Some((model.clone(), pid));
                    break;
                }
            }
            if let Some((model, pid)) = running_model_found {
                restored_model_id = Some(model.id.clone());
                let guard = swai_core::process_manager::LinuxProcessGuard {
                    pid: pid.map(|p| Pid::from_raw(p as i32)),
                    port: model.port,
                    shutdown_timeout_sec: 10,
                };
                pm_guard.set_running_model(
                    swai_core::process_manager::RunningModel {
                        id: model.id.clone(),
                        guard: Box::new(guard),
                        state: swai_core::process_manager::ModelState::Ready,
                    },
                );
            }
        }

        if let Some(restored_id) = &restored_model_id {
            for c in cards.borrow_mut().iter_mut() {
                if c.config().id == *restored_id {
                    c.set_state(CardState::Ready);
                }
            }
        }

        // Phase 7: Track close-request state.
        let close_requested = Rc::new(RefCell::new(false));

        // Phase 7.1: Detect whether a tray host is available.
        let tray_host_available = crate::tray::tray_host_available();
        tracing::info!(
            "tray host available: {}",
            if tray_host_available { "yes" } else { "no" }
        );

        // Clone PM for MainWindow struct (needed for quit()).
        let pm_for_struct = Arc::clone(&pm);

        // Clone PM and app for the close dialog.
        let pm_close = Arc::clone(&pm);
        let app_close = app.clone();

        // Clone PM for refresh action (needs to be after timeout closure borrows pm).
        let pm_refresh = Arc::clone(&pm);

        // Phase 7: Window close (X) behavior.
        let close_requested_clone = Rc::clone(&close_requested);
        let widget_hide = widget.clone();
        let widget_close_req = widget.clone();
        widget_close_req.connect_close_request(move |win| {
            if *close_requested_clone.borrow() {
                return glib::Propagation::Stop;
            }

            // Close any open child modal dialogs (like Preferences) so the quit/minimize dialog can present.
            if let Some(app) = win.application() {
                let win_ptr = win.upcast_ref::<gtk::Window>().as_ptr();
                for w in app.windows() {
                    if w.as_ptr() != win_ptr {
                        w.destroy();
                    }
                }
            }

            *close_requested_clone.borrow_mut() = true;

            let tray_available = tray_host_available;

            let (dialog_msg, has_minimize) = if tray_available {
                ("Quit SWAI entirely, or minimize to tray?", true)
            } else {
                (
                    "Quit SWAI entirely?\n\n\
                     Minimize to tray isn't available - no system tray was \
                     detected on this desktop.",
                    false,
                )
            };

            let dialog = MessageDialog::new(
                Some(win),
                gtk::DialogFlags::MODAL,
                MessageType::Question,
                gtk::ButtonsType::None,
                dialog_msg,
            );
            dialog.set_title(Some("SWAI"));

            dialog.add_button("Quit", ResponseType::Close);
            if has_minimize {
                dialog.add_button("Minimize to Tray", ResponseType::Apply);
            }

            let close_clone = Rc::clone(&close_requested_clone);
            let pm_quit = Arc::clone(&pm_close);
            let app_quit = app_close.clone();
            let widget_hide_ref = widget_hide.clone();
            dialog.connect_response(move |d, response| {
                match response {
                    ResponseType::Close => {
                        let _ = pm_quit.lock().unwrap_or_else(|e| {
                            tracing::error!("close dialog: process manager lock poisoned, continuing with shutdown");
                            e.into_inner()
                        }).stop_all(true);
                        tracing::info!("user chose to quit from close dialog");
                        for w in app_quit.windows() {
                            w.destroy();
                        }
                        app_quit.quit();
                    }
                    ResponseType::Apply => {
                        tracing::info!("user chose to minimize to tray");
                        widget_hide_ref.hide();
                    }
                    _ => {}
                }
                *close_clone.borrow_mut() = false;
                d.destroy();
            });

            dialog.present();
            glib::Propagation::Stop
        });

        // Wire up application-level actions.
        {
            let pm_wa = Arc::clone(&pm);
            let app_wa = app.clone();
            let on_quit: Arc<dyn Fn()> = Arc::new(move || {
                let _ = pm_wa.lock().unwrap_or_else(|e| {
                    tracing::error!("quit: process manager lock poisoned, continuing with shutdown");
                    e.into_inner()
                }).stop_all(true);
                for w in app_wa.windows() {
                    w.destroy();
                }
                app_wa.quit();
            });
            Self::wire_actions(&widget, app, on_quit, Arc::clone(&pm));
        }

        // Create channels for process management messages.
        let (sender, receiver) = std::sync::mpsc::channel::<ChannelMessage>();
        let sender_poll = sender.clone();

        // Channel for context slot updates.
        let (slot_sender, slot_receiver) = std::sync::mpsc::channel::<SlotUpdate>();

        // Phase 7: Channels for tray menu actions.
        let (window_sender, window_receiver) = std::sync::mpsc::channel::<WindowAction>();
        let (tray_sender, tray_receiver) = std::sync::mpsc::channel::<TrayAction>();
        let (quit_sender, quit_receiver) = std::sync::mpsc::channel::<()>();

        // Phase 8: Channel for import wizard messages.
        let (import_sender, import_receiver) = std::sync::mpsc::channel::<ImportMessage>();

        // Manage Models action.
        {
            let import_sender_for_manage = import_sender.clone();
            let pm_for_manage = Arc::clone(&pm);
            let manage_models_action = gio::SimpleAction::new("manage_models", None);
            manage_models_action.connect_activate(glib::clone!(
                #[weak]
                widget,
                move |_, _| {
                    Self::show_manage_models_dialog(&widget, &import_sender_for_manage, &pm_for_manage);
                }
            ));
            widget.add_action(&manage_models_action);
        }

        // Phase 8: Add Model action.
        {
            let import_sender_for_action = import_sender.clone();
            let add_model_action = gio::SimpleAction::new("add_model", None);
            add_model_action.connect_activate(glib::clone!(
                #[weak]
                widget,
                move |_, _| {
                    Self::show_add_model_dialog(&widget, &import_sender_for_action);
                }
            ));
            widget.add_action(&add_model_action);
        }

        // Poll channel messages on the main context loop.
        let cards_clone = Rc::clone(&cards);
        let pm_timeout = Arc::clone(&pm);
        let widget_timeout = widget.clone();
        let _app_timeout = app.clone();
        let quit_receiver = quit_receiver;
        let tray_receiver = tray_receiver;
        let import_receiver = import_receiver;
        let tray_handle_timeout: Rc<RefCell<Option<Handle<crate::tray::SwaiTray>>>> =
            Rc::new(RefCell::new(None));
        let tray_handle_for_struct = Rc::clone(&tray_handle_timeout);
        let current_keep_alive_for_struct = Rc::clone(&current_keep_alive);
        let current_keep_alive_for_handlers = Rc::clone(&current_keep_alive);
        let current_keep_alive_post_closure = Rc::clone(&current_keep_alive);
        let sender_for_handlers = sender.clone();
        let sender_for_post_closure = sender.clone();

        // Clone shared refs for the import handler.
        let pm_for_import = Arc::clone(&pm);
        let proxy_state_for_import = Rc::new(proxy_state.clone());
        let config_for_import = config.clone();


        // Create log_viewer before the timeout closure.
        let log_viewer = Rc::new(RefCell::new(None::<LogViewerWindow>));
        let log_viewer_for_closure = Rc::clone(&log_viewer);

        // Clone config for auto-follow preference check in timeout closure.
        let config_for_timeout = config.clone();

        let _timeout_card_box = Rc::clone(&footer_card_box);

        glib::timeout_add_local(std::time::Duration::from_millis(50), move || {
            let mut cards_borrow = cards_clone.borrow_mut();

            // ── Process process-management messages first. ─────────────
            while let Ok(msg) = receiver.try_recv() {
                match msg {
                    ChannelMessage::SwitchCompleted { target_id, result } => {
                        for c in cards_borrow.iter_mut() {
                            let cid = c.config().id.clone();
                            if cid == target_id {
                                match &result {
                                    Ok(()) => {
                                        c.set_state(CardState::Ready);
                                    }
                                    Err(e) => {
                                        c.set_state(CardState::Error(format!(
                                            "Failed to start: {}", e
                                        )));
                                        if config_for_timeout.enable_notifications() {
                                            Self::notify(
                                                "SWAI - Model Error",
                                                "Failed to start model - process exited with error",
                                            );
                                        }
                                    }
                                }
                            } else {
                                let is_running = pm_timeout.lock().ok().map(|pm| pm.find_running_model(&cid).is_some()).unwrap_or(false);
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
                        Self::reorder_card_container(&cards_borrow);

                        // Update footer model label.
                        if let Some(active_card) = cards_borrow.iter().find(|c| {
                            matches!(c.state(), CardState::Ready | CardState::Starting | CardState::Loading)
                        }) {
                            footer_model_label_clone.set_text(&format!("{} active", active_card.config().name));
                            footer_model_label_clone.set_css_classes(&["accent-label"]);
                        } else {
                            footer_model_label_clone.set_text(&format!("SWAI v{}", env!("CARGO_PKG_VERSION")));
                            footer_model_label_clone.set_css_classes(&["dim-label"]);
                        }

                        // Phase 14.2: Auto-follow active model in LogViewerWindow.
                        if result.is_ok() && config_for_timeout.auto_follow_logs() {
                            if let Some(ref log_viewer) = *log_viewer_for_closure.borrow() {
                                log_viewer.select_model_by_id(&target_id);
                            }
                        }

                        // Phase 15.2: Notify on switch if notifications & switch alert are enabled.
                        if result.is_ok() && config_for_timeout.enable_notifications() && config_for_timeout.notify_on_switch() {
                            let model_name = cards_borrow.iter()
                                .find(|c| c.config().id == target_id)
                                .map(|c| c.config().name.clone())
                                .unwrap_or_else(|| target_id.clone());
                            Self::notify(
                                "SWAI",
                                &format!("Switched to {} (Ready)", model_name),
                            );
                        }
                    }
                    ChannelMessage::StopCompleted { running_id, result } => {
                        for c in cards_borrow.iter_mut() {
                            if c.config().id == running_id {
                                match &result {
                                    Ok(()) => c.set_state(CardState::Stopped),
                                    Err(e) => c.set_state(CardState::Error(format!(
                                        "Failed to stop: {}", e
                                    ))),
                                }
                                c.enable_toggle();
                                c.enable_restart();
                            }
                        }
                        if let Some(ref handle) = *tray_handle_timeout.borrow() {
                            handle.update(|_| {});
                        }
                        Self::reorder_card_container(&cards_borrow);

                        // Update footer model label.
                        if let Some(active_card) = cards_borrow.iter().find(|c| {
                            matches!(c.state(), CardState::Ready | CardState::Starting | CardState::Loading)
                        }) {
                            footer_model_label_clone.set_text(&format!("{} active", active_card.config().name));
                            footer_model_label_clone.set_css_classes(&["accent-label"]);
                        } else {
                            footer_model_label_clone.set_text(&format!("SWAI v{}", env!("CARGO_PKG_VERSION")));
                            footer_model_label_clone.set_css_classes(&["dim-label"]);
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
                    // P0-1/P2-4: Handle intermediate state updates from health monitor.
                    ChannelMessage::StateUpdate { model_id, state } => {
                        let mut needs_toggle_enable = false;
                        for c in cards_borrow.iter_mut() {
                            if c.config().id == model_id {
                                match &state {
                                    ModelState::Starting => {
                                        c.set_state(CardState::Starting);
                                    }
                                    ModelState::Loading => {
                                        c.set_state(CardState::Loading);
                                    }
                                    ModelState::Ready => {
                                        c.set_state(CardState::Ready);
                                        needs_toggle_enable = true;
                                    }
                                    ModelState::Error(msg) => {
                                        c.set_state(CardState::Error(format!(
                                            "Failed to load: {}", msg
                                        )));
                                        needs_toggle_enable = true;
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
                    }
                }
            }

            // ── Process context slot updates. ──────────────────────────
            while let Ok(update) = slot_receiver.try_recv() {
                for c in cards_borrow.iter_mut() {
                    if c.config().id == update.model_id {
                        c.set_context(update.tokens_used, update.n_ctx);

                        // Update live speed label if model is Ready.
                        let current_state = c.state();
                        if matches!(current_state, CardState::Ready) {
                            c.set_speed(update.predicted_per_second);
                        }
                    }
                }
            }

            // ── Phase 7: Process tray window actions. ──────────────────
            while let Ok(action) = window_receiver.try_recv() {
                match action {
                    WindowAction::Hide => {
                        widget_timeout.hide();
                    }
                    WindowAction::Show => {
                        widget_timeout.show();
                        widget_timeout.present();
                    }
                }
            }

            // ── Phase 7: Process tray quick-switch actions. ────────────
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

                            // The health monitor drives the final state.
                            // Only send SwitchCompleted on failure.
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

            // ── Phase 7: Process tray quit signals. ────────────────────
            if quit_receiver.try_recv().is_ok() {
                tracing::info!("quit signal received from tray - prompting confirmation");
                widget_timeout.show();
                widget_timeout.present();
                widget_timeout.close();
            }

            // ── Phase 8: Process import wizard messages. ───────────────
            while let Ok(msg) = import_receiver.try_recv() {
                match msg {
                    ImportMessage::ModelImported { model } => {
                        pm_for_import.lock().unwrap_or_else(|e| {
                            tracing::error!("import: process manager lock poisoned, skipping model add");
                            e.into_inner()
                        }).add_model(model.clone());

                        let mut card = ModelCard::new(&model);
                        let model_id = model.id.clone();
                        let model_id_restart = model_id.clone();

                        let pm_import = Arc::clone(&pm_for_import);
                        let keep_alive_import = Rc::clone(&current_keep_alive_for_handlers);
                        let sender_import = sender_for_handlers.clone();
                        let proxy_import = Rc::clone(&proxy_state_for_import);
                        let cards_import = Rc::clone(&cards_clone);

                        // ── Toggle handler ───────────────────────────────
                        {
                            let ka_ref = Rc::clone(&keep_alive_import);
                            let cards_inner = Rc::clone(&cards_import);
                            let sender_inner = sender_import.clone();
                            let pm_ref = Arc::clone(&pm_import);
                            let proxy_for_toggle = Rc::clone(&proxy_import);
                            // Clone for health monitor use inside the closure.
                            let sender_health_import = sender_import.clone();
                            card.set_toggle_handler(move |on| {
                                let proxy_for_handler = proxy_for_toggle.as_ref()
                                    .as_ref()
                                    .map(Arc::clone);
                                let cards_inner = cards_inner.borrow();
                                if on {
                                    let target_card = match cards_inner.iter().find(|c| c.config().id == model_id) {
                                        Some(c) => c,
                                        None => return,
                                    };
                                    if target_card.state().is_transitioning() {
                                        return;
                                    }
                                    for c in cards_inner.iter() {
                                        if !c.state().is_transitioning() {
                                            c.disable_toggle();
                                        }
                                    }
                                    target_card.set_starting();

                                    let is_switching = {
                                        if let Ok(pm_check) = pm_ref.lock() {
                                            pm_check.running_count() >= pm_check.max_concurrent_models()
                                        } else {
                                            true
                                        }
                                    };
                                    if is_switching {
                                        if let Some(ref old_ka) = *ka_ref.borrow() {
                                            old_ka.store(false, Ordering::SeqCst);
                                        }
                                    }
                                    let new_ka = Arc::new(AtomicBool::new(true));
                                    *ka_ref.borrow_mut() = Some(Arc::clone(&new_ka));

                                    let bg_model_id = model_id.clone();
                                    let pm_thread = Arc::clone(&pm_ref);
                                    let sender_thread = sender_inner.clone();
                                    let ka_thread = Arc::clone(&new_ka);
                                    let sender_health_for_thread = sender_health_import.clone();

                                    std::thread::spawn(move || {
                                        let result = {
                                            let mut pm_lock = match pm_thread.lock() {
                                                Ok(g) => g,
                                                Err(_) => return,
                                            };
                                            if pm_lock.find_running_model(&bg_model_id).is_some() {
                                                return;
                                            }
                                            let running_count = pm_lock.running_count();
                                            let max_concurrent = pm_lock.max_concurrent_models();

                                            if running_count < max_concurrent {
                                                pm_lock.start_model(&bg_model_id)
                                            } else {
                                                let primary_id = pm_lock.get_primary_model_id().unwrap_or("").to_string();
                                                if !primary_id.is_empty() {
                                                    pm_lock.switch_model(&primary_id, &bg_model_id)
                                                } else {
                                                    pm_lock.start_model(&bg_model_id)
                                                }
                                            }
                                        };

                                        let is_ok = result.is_ok();
                                        let _port_for_proxy = if is_ok {
                                            pm_thread.lock()
                                                .ok()
                                                .and_then(|pm| pm.config().models.iter()
                                                    .find(|m| m.id == bg_model_id)
                                                    .map(|m| m.port))
                                        } else {
                                            None
                                        };

                                        // P0-1/P2-4: Spawn health monitor polling thread.
                                        // The health monitor drives the final Ready/Error
                                        // state — we don't send SwitchCompleted on success
                                        // because start_model only spawns the process.
                                        if is_ok {
                                            let pm_health = Arc::clone(&pm_thread);
                                            let sender_health = sender_health_for_thread.clone();
                                            let model_id_health = bg_model_id.clone();
                                            Self::spawn_health_monitor(pm_health, sender_health, model_id_health);
                                        } else {
                                            let _ = sender_thread.send(ChannelMessage::SwitchCompleted {
                                                target_id: bg_model_id,
                                                result,
                                            });
                                        }

                                        if is_ok {
                                            if let Some(ref proxy) = proxy_for_handler {
                                                let running = pm_thread.lock()
                                                    .ok()
                                                    .map(|pm| pm.running_model_ports())
                                                    .unwrap_or_default();
                                                proxy.lock().unwrap_or_else(|e| {
                                                    tracing::error!("proxy state lock poisoned");
                                                    e.into_inner()
                                                }).sync_models(running);
                                            }
                                        }

                                        if is_ok {
                                            while ka_thread.load(Ordering::SeqCst) {
                                                std::thread::sleep(std::time::Duration::from_millis(100));
                                            }
                                        }
                                    });
                                } else {
                                    if let Some(ref old_ka) = *ka_ref.borrow() {
                                        old_ka.store(false, Ordering::SeqCst);
                                    }
                                    *ka_ref.borrow_mut() = None;

                                    let pm_thread = Arc::clone(&pm_ref);
                                    let sender_thread = sender_inner.clone();
                                    let proxy_thread = proxy_for_toggle.as_ref().as_ref().map(Arc::clone);
                                    let bg_model_id = model_id.clone();

                                    std::thread::spawn(move || {
                                        let mut pm_lock = match pm_thread.lock() {
                                            Ok(g) => g,
                                            Err(_) => return,
                                        };
                                        let result = pm_lock.stop_model(&bg_model_id, false);
                                        let is_ok = result.is_ok();
                                        let _ = sender_thread.send(ChannelMessage::StopCompleted {
                                            running_id: bg_model_id,
                                            result,
                                        });

                                        if is_ok {
                                            if let Some(ref proxy) = proxy_thread {
                                                let running = pm_lock.running_model_ports();
                                                proxy.lock().unwrap_or_else(|e| {
                                                    tracing::error!("proxy state lock poisoned");
                                                    e.into_inner()
                                                }).sync_models(running);
                                            }
                                        }
                                    });
                                }
                            });
                        }

                        // ── Restart button handler ───────────────────────
                        {
                            let cards_restart = Rc::clone(&cards_import);
                            let sender_restart = sender_import.clone();
                            let pm_restart = Arc::clone(&pm_import);
                            let ka_ref_restart = Rc::clone(&keep_alive_import);
                            let proxy_for_restart = Rc::clone(&proxy_import);

                            card.restart_button.connect_clicked(move |_| {
                                let proxy_thread = proxy_for_restart.as_ref()
                                    .as_ref()
                                    .map(Arc::clone);
                                let cards_inner = cards_restart.borrow();
                                let target = match cards_inner.iter().find(|c| c.config().id == model_id_restart) {
                                    Some(c) => c,
                                    None => return,
                                };

                                if target.state().is_transitioning() || target.restart_requested() {
                                    return;
                                }

                                target.disable_restart();

                                if let Some(ref old_ka) = *ka_ref_restart.borrow() {
                                    old_ka.store(false, Ordering::SeqCst);
                                }
                                let new_ka = Arc::new(AtomicBool::new(true));
                                *ka_ref_restart.borrow_mut() = Some(Arc::clone(&new_ka));

                                let bg_model_id = model_id_restart.clone();
                                let pm_thread = Arc::clone(&pm_restart);
                                let sender_thread = sender_restart.clone();
                                let ka_thread = new_ka;
                                let proxy_restart = proxy_thread.as_ref().map(Arc::clone);

                                std::thread::spawn(move || {
                                    let _ = sender_thread.send(ChannelMessage::RestartRequested {
                                        model_id: bg_model_id.clone(),
                                    });

                                    let result = {
                                        let mut pm_lock = match pm_thread.lock() {
                                            Ok(g) => g,
                                            Err(_) => return,
                                        };

                                        if pm_lock.get_primary_model_id() == Some(bg_model_id.as_str()) {
                                            let _ = pm_lock.stop_model(&bg_model_id, false);
                                            std::thread::sleep(std::time::Duration::from_millis(500));
                                        }

                                        pm_lock.start_model(&bg_model_id)
                                    };

                                    let is_ok = result.is_ok();
                                    let _port_for_proxy = if is_ok {
                                        pm_thread.lock()
                                            .ok()
                                            .and_then(|pm| pm.config().models.iter()
                                                .find(|m| m.id == bg_model_id)
                                                .map(|m| m.port))
                                    } else {
                                        None
                                    };

                                    // P0-1/P2-4: Spawn health monitor polling thread.
                                    // The health monitor drives the final Ready/Error
                                    // state — we don't send SwitchCompleted on success
                                    // because start_model only spawns the process.
                                    if is_ok {
                                        let pm_health = Arc::clone(&pm_thread);
                                        let sender_health = sender_thread.clone();
                                        let model_id_health = bg_model_id.clone();
                                        Self::spawn_health_monitor(pm_health, sender_health, model_id_health);
                                    } else {
                                        let _ = sender_thread.send(ChannelMessage::SwitchCompleted {
                                            target_id: bg_model_id,
                                            result,
                                        });
                                    }

                                    if is_ok {
                                        if let Some(ref proxy) = proxy_restart {
                                            let running = pm_thread.lock()
                                                .ok()
                                                .map(|pm| pm.running_model_ports())
                                                .unwrap_or_default();
                                            proxy.lock().unwrap_or_else(|e| {
                                                tracing::error!("proxy state lock poisoned");
                                                e.into_inner()
                                            }).sync_models(running);
                                        }
                                    }

                                    if is_ok {
                                        while ka_thread.load(Ordering::SeqCst) {
                                            std::thread::sleep(std::time::Duration::from_millis(100));
                                        }
                                    }
                                });
                            });
                        }

                        // ── Logs button handler ────────────────────────
                        {
                            let card_config = model.clone();
                            let log_viewer_ref = Rc::clone(&log_viewer_for_closure);
                            let log_dir = config_for_import.log_dir();
                            let all_models = config_for_import.models.clone();

                            card.set_logs_handler(move || {
                                let viewer = LogViewerWindow::new(
                                    &card_config.name,
                                    &card_config.script_path,
                                    &log_dir,
                                    &card_config.id,
                                    &all_models,
                                );
                                viewer.present();
                                *log_viewer_ref.borrow_mut() = Some(viewer);
                            });
                        }

                        cards_borrow.push(card);
                        tracing::info!(
                            "Appended card for newly imported model '{}' (port {})",
                            model.id,
                            model.port
                        );
                    }
                    // Phase 11.2: Live-update a card label and port when settings
                    // change via the Edit dialog.
                    ImportMessage::ModelNameUpdated { id: updated_id, name: new_name, port: new_port } => {
                        for c in cards_borrow.iter_mut() {
                            if c.config().id == updated_id {
                                c.update_model(&new_name, new_port);
                            }
                        }
                    }
                    // Phase 11.3: Remove a model's card when it is deleted.
                    ImportMessage::ModelDeleted { id: deleted_id } => {
                        let len_before = cards_borrow.len();
                        cards_borrow.retain(|c| c.config().id != deleted_id);
                        if cards_borrow.len() < len_before {
                            tracing::info!("Removed card for deleted model '{}'", deleted_id);
                            Self::reorder_card_container(&cards_borrow);
                        }
                    }
                }
            }

            glib::ControlFlow::Continue
        });

        // Start the context polling thread.
        let pm_poll = Arc::clone(&pm);
        let auto_restart_enabled = config.auto_restart_on_context_full();
        let enable_notifications = config.enable_notifications();
        Self::spawn_context_poller(
            pm_poll,
            sender_poll,
            slot_sender,
            auto_restart_enabled,
            proxy_state.clone(),
            enable_notifications,
        );

        // Wire toggle and restart handlers.
        {
            let pm_clone = Arc::clone(&pm);
            let keep_alive_ref = Rc::clone(&current_keep_alive_post_closure);
            let sender_ref = sender_for_post_closure.clone();
            let proxy_state_for_bg = Rc::new(proxy_state.clone());
            let cards_for_toggle = Rc::clone(&cards);
            let sender_for_toggle = sender_ref.clone();
            let pm_for_toggle = Arc::clone(&pm_clone);

            let mut cards_borrow = cards.borrow_mut();
            for card in cards_borrow.iter_mut() {
                let model_id = card.config().id.clone();
                let model_id_toggle = model_id.clone();
                let model_id_restart = model_id.clone();
                let proxy_for_toggle = Rc::clone(&proxy_state_for_bg);
                let proxy_for_restart = Rc::clone(&proxy_state_for_bg);

                // ── Toggle handler ───────────────────────────────────
                {
                    let ka_ref = Rc::clone(&keep_alive_ref);
                    let cards_inner = cards_for_toggle.clone();
                    let sender_inner = sender_for_toggle.clone();
                    let pm_ref = Arc::clone(&pm_for_toggle);
                    // Clone for health monitor use inside the closure.
                    let sender_health_for_toggle = sender_for_toggle.clone();
                    card.set_toggle_handler(move |on| {
                        let proxy_for_handler = proxy_for_toggle.as_ref()
                            .as_ref()
                            .map(Arc::clone);
                        let cards_inner = cards_inner.borrow();
                        if on {
                            let target_card = match cards_inner.iter().find(|c| c.config().id == model_id_toggle) {
                                Some(c) => c,
                                None => return,
                            };
                            if target_card.state().is_transitioning() {
                                return;
                            }
                            for c in cards_inner.iter() {
                                if !c.state().is_transitioning() {
                                    c.disable_toggle();
                                }
                            }
                            target_card.set_starting();

                            if let Some(ref old_ka) = *ka_ref.borrow() {
                                old_ka.store(false, Ordering::SeqCst);
                            }

                            let new_ka = Arc::new(AtomicBool::new(true));
                            *ka_ref.borrow_mut() = Some(Arc::clone(&new_ka));

                            let bg_model_id = model_id_toggle.clone();
                            let pm_thread = Arc::clone(&pm_ref);
                            let sender_thread = sender_inner.clone();
                            let ka_thread = Arc::clone(&new_ka);
                            // Clone for health monitor inside closure body (Fn can't move captured values).
                            let sender_health_for_thread = sender_health_for_toggle.clone();

                            std::thread::spawn(move || {
                                let result = {
                                    let mut pm_lock = match pm_thread.lock() {
                                        Ok(g) => g,
                                        Err(_) => return,
                                    };
                                    if pm_lock.get_primary_model_id() == Some(bg_model_id.as_str()) {
                                        return;
                                    }
                                    let running_count = pm_lock.running_count();
                                    let max_concurrent = pm_lock.max_concurrent_models();

                                    if running_count < max_concurrent {
                                        // Room under the concurrent limit — add this model
                                        // as a new concurrent instance (does not stop others).
                                        pm_lock.start_model(&bg_model_id)
                                    } else {
                                        // At the limit — replace the primary model.
                                        let primary_id = pm_lock.get_primary_model_id().unwrap_or("").to_string();
                                        if !primary_id.is_empty() {
                                            pm_lock.switch_model(&primary_id, &bg_model_id)
                                        } else {
                                            pm_lock.start_model(&bg_model_id)
                                        }
                                    }
                                };

                                let is_ok = result.is_ok();
                                let _port_for_proxy = if is_ok {
                                    pm_thread.lock()
                                        .ok()
                                        .and_then(|pm| pm.config().models.iter()
                                            .find(|m| m.id == bg_model_id)
                                            .map(|m| m.port))
                                } else {
                                    None
                                };

                                // P0-1/P2-4: Spawn health monitor polling thread.
                                // The health monitor will send StateUpdate messages
                                // that drive the card to Ready/Loading/Error.
                                // We do NOT send SwitchCompleted here — start_model
                                // only spawns the process, it doesn't confirm the
                                // model is healthy. The health monitor owns the
                                // final state transition.
                                if is_ok {
                                    let pm_health = Arc::clone(&pm_thread);
                                    let sender_health = sender_health_for_thread.clone();
                                    let model_id_health = bg_model_id.clone();
                                    Self::spawn_health_monitor(pm_health, sender_health, model_id_health);
                                } else {
                                    // start_model failed — send error to UI.
                                    let _ = sender_thread.send(ChannelMessage::SwitchCompleted {
                                        target_id: bg_model_id,
                                        result,
                                    });
                                }

                                if is_ok {
                                    if let Some(ref proxy) = proxy_for_handler {
                                        let running = pm_thread.lock()
                                            .ok()
                                            .map(|pm| pm.running_model_ports())
                                            .unwrap_or_default();
                                        proxy.lock().unwrap_or_else(|e| {
                                            tracing::error!("proxy state lock poisoned");
                                            e.into_inner()
                                        }).sync_models(running);
                                    }
                                }

                                if is_ok {
                                    while ka_thread.load(Ordering::SeqCst) {
                                        std::thread::sleep(std::time::Duration::from_millis(100));
                                    }
                                }
                            });
                        } else {
                            if let Some(ref old_ka) = *ka_ref.borrow() {
                                old_ka.store(false, Ordering::SeqCst);
                            }
                            *ka_ref.borrow_mut() = None;

                            let pm_thread = Arc::clone(&pm_ref);
                            let sender_thread = sender_inner.clone();
                            let proxy_thread = proxy_for_toggle.as_ref().as_ref().map(Arc::clone);

                            let bg_model_id = model_id_toggle.clone();

                            std::thread::spawn(move || {
                                let mut pm_lock = match pm_thread.lock() {
                                    Ok(g) => g,
                                    Err(_) => return,
                                };
                                let result = pm_lock.stop_model(&bg_model_id, false);
                                let is_ok = result.is_ok();
                                let _ = sender_thread.send(ChannelMessage::StopCompleted {
                                    running_id: bg_model_id,
                                    result,
                                });

                                if is_ok {
                                    if let Some(ref proxy) = proxy_thread {
                                        let running = pm_lock.running_model_ports();
                                        proxy.lock().unwrap_or_else(|e| {
                                            tracing::error!("proxy state lock poisoned");
                                            e.into_inner()
                                        }).sync_models(running);
                                    }
                                }
                            });
                        }
                    });
                }

                // ── Restart button handler ───────────────────────────
                {
                    let cards_restart = cards_for_toggle.clone();
                    let sender_restart = sender_ref.clone();
                    let pm_restart = Arc::clone(&pm_clone);
                    let ka_ref_restart = Rc::clone(&keep_alive_ref);
                    // Clone for health monitor use inside the closure.
                    let sender_health_for_restart = sender_ref.clone();

                    card.restart_button.connect_clicked(move |_| {
                        let proxy_thread = proxy_for_restart.as_ref()
                            .as_ref()
                            .map(Arc::clone);
                        let cards_inner = cards_restart.borrow();
                        let target = match cards_inner.iter().find(|c| c.config().id == model_id_restart) {
                            Some(c) => c,
                            None => return,
                        };

                        if target.state().is_transitioning() || target.restart_requested() {
                            return;
                        }

                        target.disable_restart();

                        if let Some(ref old_ka) = *ka_ref_restart.borrow() {
                            old_ka.store(false, Ordering::SeqCst);
                        }

                        let new_ka = Arc::new(AtomicBool::new(true));
                        *ka_ref_restart.borrow_mut() = Some(Arc::clone(&new_ka));

                        let bg_model_id = model_id_restart.clone();
                        let pm_thread = Arc::clone(&pm_restart);
                        let sender_thread = sender_restart.clone();
                        let ka_thread = new_ka;
                        let proxy_restart = proxy_thread.as_ref().map(Arc::clone);
                        // Clone for health monitor inside closure body (Fn can't move captured values).
                        let _sender_health_for_thread = sender_health_for_restart.clone();

                        std::thread::spawn(move || {
                            let _ = sender_thread.send(ChannelMessage::RestartRequested {
                                model_id: bg_model_id.clone(),
                            });

                            let result = {
                                let mut pm_lock = match pm_thread.lock() {
                                    Ok(g) => g,
                                    Err(_) => return,
                                };

                                if pm_lock.get_primary_model_id() == Some(bg_model_id.as_str()) {
                                    let _ = pm_lock.stop_model(&bg_model_id, false);
                                    std::thread::sleep(std::time::Duration::from_millis(500));
                                }

                                pm_lock.start_model(&bg_model_id)
                            };

                            let is_ok = result.is_ok();
                            let _port_for_proxy = if is_ok {
                                pm_thread.lock()
                                    .ok()
                                    .and_then(|pm| pm.config().models.iter()
                                        .find(|m| m.id == bg_model_id)
                                        .map(|m| m.port))
                            } else {
                                None
                            };

                            // P0-1/P2-4: Spawn health monitor polling thread.
                            // The health monitor drives the final Ready/Error
                            // state — we don't send SwitchCompleted on success
                            // because start_model only spawns the process.
                            if is_ok {
                                let pm_health = Arc::clone(&pm_thread);
                                let sender_health = sender_thread.clone();
                                let model_id_health = bg_model_id.clone();
                                Self::spawn_health_monitor(pm_health, sender_health, model_id_health);
                            } else {
                                let _ = sender_thread.send(ChannelMessage::SwitchCompleted {
                                    target_id: bg_model_id,
                                    result,
                                });
                            }

                            if is_ok {
                                if let Some(ref proxy) = proxy_restart {
                                    let running = pm_thread.lock()
                                        .ok()
                                        .map(|pm| pm.running_model_ports())
                                        .unwrap_or_default();
                                    proxy.lock().unwrap_or_else(|e| {
                                        tracing::error!("proxy state lock poisoned");
                                        e.into_inner()
                                    }).sync_models(running);
                                }
                            }

                            if is_ok {
                                while ka_thread.load(Ordering::SeqCst) {
                                    std::thread::sleep(std::time::Duration::from_millis(100));
                                }
                            }
                        });
                    });
                }

                // ── Logs button handler ────────────────────────────────
                {
                    let card_config = card.config().clone();
                    let log_viewer_ref = Rc::clone(&log_viewer);
                    let log_dir = config.log_dir();
                    let all_models = config.models.clone();

                    card.set_logs_handler(move || {
                        let viewer = LogViewerWindow::new(
                            &card_config.name,
                            &card_config.script_path,
                            &log_dir,
                            &card_config.id,
                            &all_models,
                        );
                        viewer.present();
                        *log_viewer_ref.borrow_mut() = Some(viewer);
                    });
                }
            }

            Self::reorder_card_container(&cards_borrow);
        }

        // Phase 7: Create the system tray icon using ksni.
        let tray_handle = crate::tray::create_tray(
            config.clone(),
            Arc::clone(&pm),
            window_sender.clone(),
            tray_sender,
            quit_sender.clone(),
        );
        if tray_handle.is_some() {
            tracing::info!("system tray icon created");
        }

        *tray_handle_for_struct.borrow_mut() = tray_handle;

        // Clone cards before moving into struct (needed for toggle_logs action).
        let cards_for_toggle = Rc::clone(&cards);

        let instance = Self {
            widget: widget.clone(),
            cards,
            current_keep_alive: current_keep_alive_for_struct,
            config,
            config_path,
            proxy_state,
            log_viewer: Rc::clone(&log_viewer),
            close_requested,
            process_manager: pm_for_struct,
            tray_handle: None,
            tray_host_available,
            import_sender: import_sender.clone(),
            footer_proxy_label,
            footer_model_label,
            unmanaged_banner: unmanaged_banner.clone(),
        };

        // Wire the toggle logs panel action.
        // Opens LogViewerWindow for the currently active/Ready model,
        // or falls back to the first model if none are active.
        {
            let lv_ref = Rc::clone(&log_viewer);
            let config_for_logs = instance.config.clone();
            let toggle_action = gio::SimpleAction::new("toggle_logs", None);
            toggle_action.connect_activate(glib::clone!(
                #[strong]
                lv_ref,
                move |_, _| {
                    // Find the active (Ready) model card.
                    let cards = cards_for_toggle.borrow();
                    let target = cards
                        .iter()
                        .find(|c| matches!(c.state(), CardState::Ready))
                        .or_else(|| cards.first());

                    if let Some(card) = target {
                        let cfg = card.config().clone();
                        let viewer = LogViewerWindow::new(
                            &cfg.name,
                            &cfg.script_path,
                            &config_for_logs.log_dir(),
                            &cfg.id,
                            &config_for_logs.models,
                        );
                        viewer.present();
                        *lv_ref.borrow_mut() = Some(viewer);
                    }
                }
            ));
            instance.widget.add_action(&toggle_action);
        }

        // Wire the refresh action - triggers instant health check & port
        // reconciliation across all models.
        {
            let pm_refresh = Arc::clone(&pm_refresh);
            let tray_handle_for_refresh = Rc::clone(&tray_handle_for_struct);
            let refresh_action = gio::SimpleAction::new("refresh", None);
            refresh_action.connect_activate(move |_, _| {
                // Reconcile ports: check each model's port and update state.
                for model in &pm_refresh.lock().unwrap_or_else(|e| {
                    tracing::error!("refresh: process manager lock poisoned");
                    e.into_inner()
                }).config().models {
                    match ProcessManager::check_port(model.port) {
                        PortState::OccupiedByModel => {
                            tracing::info!(
                                "Refresh: port {} occupied by model {}",
                                model.port,
                                model.id
                            );
                        }
                        _ => {
                            tracing::info!(
                                "Refresh: port {} free or occupied by other process",
                                model.port
                            );
                        }
                    }
                }
                // Update tray menu if available.
                if let Some(ref handle) = *tray_handle_for_refresh.borrow() {
                    handle.update(|_| {});
                }
            });
            instance.widget.add_action(&refresh_action);
        }

        // Phase 16.2: Wire adoption action for unmanaged-server banner.
        if let (Some(ref banner), Some(adopt_port), Some(adopt_model_name)) =
            (&unmanaged_banner, adopt_port, adopt_model_name)
        {
            let import_sender_for_adopt = import_sender.clone();
            let adopt_action = gio::SimpleAction::new("adopt_model", None);
            adopt_action.connect_activate(glib::clone!(
                #[weak]
                widget,
                move |_, _| {
                    Self::show_adopt_model_dialog(
                        &widget,
                        &import_sender_for_adopt,
                        adopt_port,
                        adopt_model_name.clone(),
                    );
                }
            ));
            widget.add_action(&adopt_action);

            // Set the action on the banner's primary action button.
            banner.set_action_name(Some("win.adopt_model"));
            banner.set_button_label(Some("Adopt"));
        }

        instance
    }

    /// Build an `adw::Banner` row prompting the user to adopt an unmanaged
    /// local LLM server discovered on the given port. The banner carries a
    /// "win.adopt_model" action that opens the Add Model dialog pre-filled
    /// with the discovered port and model name.
    fn build_adoption_banner(
        message: &str,
        _port: u16,
        _model_name: String,
        _parent: &adw::ApplicationWindow,
    ) -> adw::Banner {
        let banner = adw::Banner::new(message);
        banner.set_button_label(Some("Adopt"));

        // Mark the banner with a CSS class so it stands out visually.
        banner.set_css_classes(&["adoption-banner"]);

        banner
    }

    /// Show the Add Model dialog pre-filled with the discovered unmanaged
    /// server's port and model name. The user can then provide a launch
    /// script path and register the model into `config.toml`.
    fn show_adopt_model_dialog(
        parent: &adw::ApplicationWindow,
        import_sender: &std::sync::mpsc::Sender<ImportMessage>,
        port: u16,
        model_name: String,
    ) {
        let wizard = ImportWizard::new(parent, Some(port));

        // Pre-fill the display name with the discovered model name so the
        // user has a sensible default they can edit.
        wizard.set_display_name(&model_name);

        let parent_clone = parent.clone();
        let sender_clone = import_sender.clone();
        let wizard_clone = wizard.clone();

        wizard.widget.connect_response(move |d, response| {
            if response == ResponseType::Ok {
                match wizard_clone.try_import() {
                    Ok(imported) => {
                        match Self::append_model_to_config(&imported) {
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

    pub fn show(&self) {
        self.widget.show();
    }

    /// Phase 7: Clean quit.
    #[allow(dead_code)]
    pub fn quit(&self) {
        tracing::info!("quitting SWAI");
        let _ = self.process_manager.lock().unwrap_or_else(|e| {
            tracing::error!("quit: process manager lock poisoned, continuing with shutdown");
            e.into_inner()
        }).stop_all(true);
        self.widget.close();
    }

    /// Wire up application-level actions for menu items.
    fn wire_actions(
        window: &adw::ApplicationWindow,
        _app: &Application,
        on_quit: Arc<dyn Fn()>,
        process_manager: Arc<Mutex<ProcessManager>>,
    ) {
        let quit_action = gio::SimpleAction::new("quit", None);
        quit_action.connect_activate(move |_, _| {
            on_quit();
        });
        window.add_action(&quit_action);

        let refresh_action = gio::SimpleAction::new("refresh", None);
        refresh_action.connect_activate(|_, _| {
            tracing::info!("Refresh requested (stub)");
        });
        window.add_action(&refresh_action);

        let about_action = gio::SimpleAction::new("about", None);
        about_action.connect_activate(glib::clone!(
            #[weak]
            window,
            move |_, _| {
                Self::show_about_dialog(&window);
            }
        ));
        window.add_action(&about_action);

        let github_action = gio::SimpleAction::new("github", None);
        github_action.connect_activate(|_, _| {
            let _ = gio::AppInfo::launch_default_for_uri(
                "https://github.com/verdioso/swai",
                None::<&gio::AppLaunchContext>,
            );
        });
        window.add_action(&github_action);

        // Preferences action.
        let pm_prefs = Arc::clone(&process_manager);
        let preferences_action = gio::SimpleAction::new("preferences", None);
        preferences_action.connect_activate(glib::clone!(
            #[weak]
            window,
            move |_, _| {
                Self::show_preferences_dialog(&window, &pm_prefs);
            }
        ));
        window.add_action(&preferences_action);

        // Toggle Logs Panel action.
        let toggle_logs_action = gio::SimpleAction::new("toggle_logs", None);
        window.add_action(&toggle_logs_action);
    }

    /// Build the AdwHeaderBar with menubar on the left and action buttons (+ / refresh) on the right.
    fn build_header_bar(_app: &Application) -> HeaderBar {
        let header_bar = HeaderBar::new();

        // ── Native menubar (File | Edit | View | Help) packed on LEFT ─
        let menu_model = menu::build_context_menu();
        let menubar = gtk::PopoverMenuBar::from_model(Some(&menu_model));
        header_bar.pack_start(&menubar);

        // ── Add Model button (+) ───────────────────────────────────
        let add_btn = Button::builder()
            .icon_name("list-add-symbolic")
            .tooltip_text("Add Model")
            .action_name("win.add_model")
            .css_classes(vec!["suggested-action"])
            .build();
        header_bar.pack_end(&add_btn);

        // ── Refresh button (🔄) ────────────────────────────────────
        let refresh_btn = Button::builder()
            .icon_name("view-refresh-symbolic")
            .tooltip_text("Refresh")
            .action_name("win.refresh")
            .css_classes(vec!["flat"])
            .build();
        header_bar.pack_end(&refresh_btn);

        // ── Manage Models gear button (⚙) ───────────────────────────
        let manage_btn = Button::builder()
            .icon_name("preferences-system-symbolic")
            .tooltip_text("Manage Models")
            .action_name("win.manage_models")
            .css_classes(vec!["flat"])
            .build();
        header_bar.pack_end(&manage_btn);

        header_bar
    }

    /// Build the native application menu model (File | Edit | View | Help).
    #[allow(dead_code)]
    fn build_menu_model(_app: &Application) -> Menu {
        let menu = Menu::new();

        // ── File section ───────────────────────────────────────────
        let file_section = Menu::new();
        file_section.append(Some("Add Model"), Some("win.add_model"));
        file_section.append(Some("Quit"), Some("win.quit"));
        menu.append_section(None, &file_section);

        // ── Edit section ───────────────────────────────────────────
        let edit_section = Menu::new();
        edit_section.append(Some("Preferences"), Some("win.preferences"));
        menu.append_section(None, &edit_section);

        // ── View section ───────────────────────────────────────────
        let view_section = Menu::new();
        view_section.append(Some("Toggle Logs Panel"), Some("win.toggle_logs"));
        view_section.append(Some("Refresh"), Some("win.refresh"));
        menu.append_section(None, &view_section);

        // ── Help section ───────────────────────────────────────────
        let help_section = Menu::new();
        help_section.append(Some("About"), Some("win.about"));
        help_section.append(Some("Open GitHub Repo"), Some("win.github"));
        menu.append_section(None, &help_section);

        menu
    }

    /// Show the preferences dialog (non-blocking).
    fn show_preferences_dialog(
        parent: &adw::ApplicationWindow,
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

        let _dialog_clone = dialog.clone();

        dialog.widget.connect_response(move |d, response| {
            if response == ResponseType::Ok {
                let values = _dialog_clone.values();
                match Self::save_preferences(&values, &config_path) {
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

    /// Show the Manage Models dialog (Edit → Manage Models / gear button).
    fn show_manage_models_dialog(
        parent: &adw::ApplicationWindow,
        import_sender: &std::sync::mpsc::Sender<ImportMessage>,
        process_manager: &Arc<Mutex<ProcessManager>>,
    ) {
        let dialog = ManageModelsDialog::new(parent, import_sender.clone(), Arc::clone(process_manager));
        dialog.widget.connect_response(|d, _| {
            d.destroy();
        });
        dialog.widget.show();
    }

    /// Show the Add Model import wizard (File → Add Model).
    fn show_add_model_dialog(parent: &adw::ApplicationWindow, import_sender: &std::sync::mpsc::Sender<ImportMessage>) {
        let wizard = ImportWizard::new(parent, None);

        let parent_clone = parent.clone();
        let sender_clone = import_sender.clone();
        let wizard_clone = wizard.clone();

        wizard.widget.connect_response(move |d, response| {
            if response == ResponseType::Ok {
                match wizard_clone.try_import() {
                    Ok(imported) => {
                        match Self::append_model_to_config(&imported) {
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

    /// Append a new model to config.toml and return success.
    fn append_model_to_config(model: &ImportedModel) -> Result<(), String> {
        Self::append_model_to_config_at(&Config::resolve_path().ok_or_else(|| -> String {
            "No config file found. Please create one at ~/.config/swai/config.toml first.".to_string()
        })?, model)
    }

    /// Append a new model to a config file at the given path.
    pub(crate) fn append_model_to_config_at(
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

    /// Save the given preferences values to disk.
    fn save_preferences(
        values: &crate::preferences::PreferencesValues,
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

        // Sync autostart state with the filesystem (Phase 18).
        if values.autostart_on_login {
            swai_core::autostart::enable_autostart()
                .map_err(|e| format!("Failed to enable autostart: {}", e))?;
        } else {
            swai_core::autostart::disable_autostart()
                .map_err(|e| format!("Failed to disable autostart: {}", e))?;
        }

        Ok(())
    }

    /// Build a scrollable container holding all model cards.
    fn build_cards_container(cards_box: &GtkBox) -> ScrolledWindow {
        let scrolled = ScrolledWindow::new();
        scrolled.set_hexpand(true);
        scrolled.set_vexpand(true);
        scrolled.set_policy(gtk::PolicyType::Automatic, gtk::PolicyType::Automatic);
        scrolled.set_child(Some(cards_box));
        scrolled
    }

    fn reorder_card_container(cards: &[ModelCard]) {
        for card in cards {
            if matches!(card.state(), CardState::Ready | CardState::Starting | CardState::Loading) {
                card.widget.set_css_classes(&["card-active"]);
            } else {
                card.widget.set_css_classes(&["card"]);
            }
        }
    }

    /// Build a footer bar showing proxy state (left) and active model name or version (right).
    ///
    /// Returns the proxy label and model label so they can be updated dynamically.
    fn build_footer_bar(proxy_port: u16) -> (gtk::Box, gtk::Label, gtk::Label) {
        let footer = GtkBox::new(Orientation::Horizontal, 0);
        footer.set_css_classes(&["toolbar"]);
        footer.set_margin_start(12);
        footer.set_margin_end(12);
        footer.set_margin_bottom(6);

        // Left side: proxy address.
        let proxy_label = Label::new(Some(&format!("Proxy: 127.0.0.1:{proxy_port}")));
        proxy_label.set_css_classes(&["dim-label"]);
        proxy_label.set_halign(gtk::Align::Start);
        footer.append(&proxy_label);

        // Spacer.
        let spacer = Label::new(Some(""));
        spacer.set_hexpand(true);
        footer.append(&spacer);

        // Right side: active model name in cyan or version string.
        let model_label = Label::new(Some(&format!("SWAI v{}", env!("CARGO_PKG_VERSION"))));
        model_label.set_css_classes(&["dim-label"]);
        model_label.set_halign(gtk::Align::End);
        footer.append(&model_label);

        (footer, proxy_label, model_label)
    }

    /// Show the About dialog using AdwAboutDialog.
    fn show_about_dialog(parent: &adw::ApplicationWindow) {
        let version = env!("CARGO_PKG_VERSION");

        // Create the AdwAboutDialog.
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

        // Phase 20: Show a separate "Check for Updates" dialog instead of
        // embedding a button in the About dialog (AdwAboutDialog doesn't
        // support custom action widgets).
        let parent_about = parent.clone();
        let parent_for_response = parent_about.clone();
        let check_btn = gtk::Button::builder()
            .label("Check for Updates…")
            .css_classes(vec!["suggested-action"])
            .margin_top(12)
            .build();

        let check_dialog = gtk::Dialog::builder()
            .title("SWAI - Check for Updates")
            .transient_for(&parent_about)
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

        check_btn.connect_clicked(move |btn| {
            btn.set_sensitive(false);
            btn.set_label("Checking…");

            // Clone parent_for_response for the inner closure.
            let parent_resp = parent_for_response.clone();

            // Perform update check synchronously (quick HTTP request).
            let result = crate::update_checker::check_for_updates_blocking(
                "verdioso/swai",
                version,
            );

            match result {
                crate::update_checker::UpdateCheckResult::UpdateAvailable { version, .. } => {
                    let dlg = gtk::MessageDialog::new(
                        Some(&parent_about),
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
                        Some(&parent_about),
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
                        Some(&parent_about),
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

        // Show the check dialog transient to the about dialog.
        check_dialog.present();

        // Present the about dialog.
        about_dialog.present(Some(parent));
    }

    /// Spawn the context polling thread.
    fn spawn_context_poller(
        pm: Arc<Mutex<ProcessManager>>,
        sender: std::sync::mpsc::Sender<ChannelMessage>,
        slot_sender: std::sync::mpsc::Sender<SlotUpdate>,
        auto_restart_enabled: bool,
        proxy_state: Option<Arc<Mutex<ProxyState>>>,
        enable_notifications: bool,
    ) {
        let http_client = reqwest::blocking::Client::builder()
            .timeout(std::time::Duration::from_secs(3))
            .build()
            .expect("failed to build reqwest blocking client");

        let poll_running = Arc::new(AtomicBool::new(true));
        let poll_running_clone = Arc::clone(&poll_running);

        std::thread::spawn(move || {
            let mut last_poll_attempt = std::time::Instant::now();
            let mut last_tokens: std::collections::HashMap<String, (usize, std::time::Instant)> =
                std::collections::HashMap::new();

            while poll_running_clone.load(Ordering::SeqCst) {
                for _ in 0..20 {
                    if !poll_running_clone.load(Ordering::SeqCst) {
                        return;
                    }
                    std::thread::sleep(std::time::Duration::from_millis(100));
                }

                if last_poll_attempt.elapsed() < std::time::Duration::from_secs(1) {
                    continue;
                }

                let ready_ports: Vec<(String, u16)> = match pm.lock() {
                    Ok(pm_lock) => pm_lock
                        .get_running_models()
                        .iter()
                        .filter_map(|rm| {
                            if rm.state == ModelState::Ready {
                                let id = rm.id.clone();
                                let port = pm_lock
                                    .config()
                                    .models
                                    .iter()
                                    .find(|m| m.id == id)
                                    .map(|m| m.port)
                                    .unwrap_or(0);
                                Some((id, port))
                            } else {
                                None
                            }
                        })
                        .collect(),
                    Err(_) => continue,
                };

                for (model_id, port) in &ready_ports {
                    let url = format!("http://127.0.0.1:{}/slots", port);

                    match http_client.get(&url).send() {
                        Ok(resp) => {
                            if resp.status().is_success() {
                                if let Ok(body) = resp.text() {
                                    if let Some(slot_info) = Self::parse_slots_response(&body) {
                                        let now = std::time::Instant::now();
                                        let mut calculated_speed = slot_info.predicted_per_second;

                                        // Fallback: Delta Token Speedometer if llama-server doesn't provide predicted_per_second
                                        if calculated_speed == 0.0 {
                                            if let Some(&(prev_tokens, prev_time)) = last_tokens.get(model_id) {
                                                let dt = now.duration_since(prev_time).as_secs_f64();
                                                if dt > 0.3 && slot_info.tokens_used >= prev_tokens {
                                                    let delta_tokens = slot_info.tokens_used - prev_tokens;
                                                    if delta_tokens > 0 {
                                                        calculated_speed = delta_tokens as f64 / dt;
                                                    }
                                                }
                                            }
                                        }
                                        last_tokens.insert(model_id.clone(), (slot_info.tokens_used, now));

                                        let _ = slot_sender.send(SlotUpdate {
                                            model_id: model_id.clone(),
                                            tokens_used: slot_info.tokens_used,
                                            n_ctx: slot_info.n_ctx,
                                            predicted_per_second: calculated_speed,
                                            prompt_per_second: slot_info.prompt_per_second,
                                        });

                                        if auto_restart_enabled
                                            && slot_info.n_ctx > 0
                                            && (slot_info.tokens_used as f64 / slot_info.n_ctx as f64) >= 0.98
                                        {
                                            Self::trigger_auto_restart(
                                                &pm,
                                                &sender,
                                                &slot_sender,
                                                &proxy_state,
                                                model_id,
                                                enable_notifications,
                                                slot_info.n_ctx,
                                            );
                                        }
                                    }
                                }
                            }
                        }
                        Err(e) => {
                            tracing::debug!(
                                "slots poll failed for model '{}' on port {}: {}",
                                model_id, port, e
                            );
                        }
                    }
                }

                last_poll_attempt = std::time::Instant::now();
            }

            tracing::info!("context poller thread exited");
        });
    }

    /// Parse the /slots JSON response to extract context and speed information.
    fn parse_slots_response(body: &str) -> Option<SlotInfo> {
        let json: serde_json::Value = serde_json::from_str(body).ok()?;

        // Helper to extract speed metrics from a slot (checks top-level, timings, metrics, and stats).
        let extract_speed = |slot: &serde_json::Value| -> (f64, f64) {
            let find_num = |key: &str| -> Option<f64> {
                slot.get(key)
                    .or_else(|| slot.get("timings").and_then(|t| t.get(key)))
                    .or_else(|| slot.get("metrics").and_then(|m| m.get(key)))
                    .or_else(|| slot.get("stats").and_then(|s| s.get(key)))
                    .and_then(|v| v.as_f64())
            };

            let predicted = find_num("predicted_per_second")
                .or_else(|| find_num("predicted_ms"))
                .or_else(|| find_num("tokens_per_second"))
                .unwrap_or(0.0);

            let prompt = find_num("prompt_per_second")
                .or_else(|| find_num("prompt_ms"))
                .unwrap_or(0.0);

            (predicted, prompt)
        };

        // Case A: Top-level Array.
        if let Some(arr) = json.as_array() {
            if let Some(first_slot) = arr.first() {
                let n_ctx = first_slot.get("n_ctx").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
                let mut tokens_used: u64 = 0;
                let (mut predicted_per_second, mut prompt_per_second) = (0.0, 0.0);

                for slot in arr {
                    let prompt = slot.get("n_prompt_tokens")
                        .or_else(|| slot.get("prompt_tokens_total"))
                        .or_else(|| slot.get("n_past"))
                        .and_then(|v| v.as_u64())
                        .unwrap_or(0);

                    let gen = slot.get("next_token")
                        .and_then(|v| v.as_array())
                        .and_then(|arr| arr.first())
                        .and_then(|tok| tok.get("n_decoded"))
                        .or_else(|| slot.get("generation_tokens_total"))
                        .or_else(|| slot.get("n_decoded"))
                        .and_then(|v| v.as_u64())
                        .unwrap_or(0);

                    tokens_used += prompt + gen;

                    // Accumulate speed metrics (take the first valid values).
                    if predicted_per_second == 0.0 {
                        let (p, pr) = extract_speed(slot);
                        if p > 0.0 || pr > 0.0 {
                            predicted_per_second = p;
                            prompt_per_second = pr;
                        }
                    }
                }

                if n_ctx > 0 {
                    return Some(SlotInfo {
                        tokens_used: tokens_used as usize,
                        n_ctx,
                        predicted_per_second,
                        prompt_per_second,
                    });
                }
            }
        }

        // Case B: Top-level Object.
        let n_ctx = json.get("n_ctx").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
        let mut tokens_used: u64 = 0;
        let (mut predicted_per_second, mut prompt_per_second) = (0.0, 0.0);

        if let Some(slots) = json.get("slots").and_then(|v| v.as_array()) {
            for slot in slots {
                let prompt = slot.get("n_prompt_tokens")
                    .or_else(|| slot.get("prompt_tokens_total"))
                    .or_else(|| slot.get("n_past"))
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0);

                let gen = slot.get("next_token")
                    .and_then(|v| v.as_array())
                    .and_then(|arr| arr.first())
                    .and_then(|tok| tok.get("n_decoded"))
                    .or_else(|| slot.get("generation_tokens_total"))
                    .or_else(|| slot.get("n_decoded"))
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0);

                tokens_used += prompt + gen;

                // Accumulate speed metrics (take the first valid values).
                if predicted_per_second == 0.0 {
                    let (p, pr) = extract_speed(slot);
                    if p > 0.0 || pr > 0.0 {
                        predicted_per_second = p;
                        prompt_per_second = pr;
                    }
                }
            }
        }

        if n_ctx > 0 {
            Some(SlotInfo {
                tokens_used: tokens_used as usize,
                n_ctx,
                predicted_per_second,
                prompt_per_second,
            })
        } else {
            None
        }
    }

    /// Trigger an auto-restart when context is full (>=98% of n_ctx).
    fn trigger_auto_restart(
        pm: &Arc<Mutex<ProcessManager>>,
        sender: &std::sync::mpsc::Sender<ChannelMessage>,
        slot_sender: &std::sync::mpsc::Sender<SlotUpdate>,
        proxy_state: &Option<Arc<Mutex<ProxyState>>>,
        model_id: &str,
        enable_notifications: bool,
        n_ctx: usize,
    ) {
        static LAST_RESTART: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

        let now_sec = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);

        let last_sec = LAST_RESTART.load(Ordering::Relaxed);
        if now_sec > 0 && last_sec > 0 && now_sec < last_sec + 15 {
            tracing::info!("auto-restart for '{}' suppressed by 15s cooldown guard", model_id);
            return;
        }
        LAST_RESTART.store(now_sec, Ordering::Relaxed);

        // Immediately reset the UI progress bar tokens to 0 so stale 98% metrics don't re-trigger.
        let _ = slot_sender.send(SlotUpdate {
            model_id: model_id.to_string(),
            tokens_used: 0,
            n_ctx,
            predicted_per_second: 0.0,
            prompt_per_second: 0.0,
        });

        let bg_model_id = model_id.to_string();
        let bg_pm = Arc::clone(pm);
        let bg_sender = sender.clone();
        let bg_proxy = proxy_state.clone();
        let bg_enable_notifications = enable_notifications;

        std::thread::spawn(move || {
            let _ = bg_sender.send(ChannelMessage::RestartRequested {
                model_id: bg_model_id.clone(),
            });

            let result = {
                let mut pm_lock = match bg_pm.lock() {
                    Ok(g) => g,
                    Err(_) => return,
                };

                if pm_lock.get_primary_model_id() == Some(bg_model_id.as_str()) {
                    let _ = pm_lock.stop_model(&bg_model_id, true);
                    std::thread::sleep(std::time::Duration::from_millis(1500));
                }

                pm_lock.start_model(&bg_model_id)
            };

            let is_ok = result.is_ok();

            // The health monitor drives the final state.
            // Only send SwitchCompleted on failure.
            if !is_ok {
                let _ = bg_sender.send(ChannelMessage::SwitchCompleted {
                    target_id: bg_model_id.clone(),
                    result,
                });
            }

            if is_ok && bg_enable_notifications {
                // Schedule notification on the main (GLib) thread.
                let auto_restart_body = format!(
                    "Context full - {} restarted",
                    bg_model_id
                );
                glib::idle_add_once(move || {
                    MainWindow::notify("SWAI", &auto_restart_body);
                });

                if let Some(ref proxy) = bg_proxy {
                    let mut ps = proxy.lock().unwrap_or_else(|e| {
                        tracing::error!("auto-restart: proxy state lock poisoned");
                        e.into_inner()
                    });
                    let running = bg_pm.lock()
                        .ok()
                        .map(|pm| pm.running_model_ports())
                        .unwrap_or_default();
                    ps.sync_models(running);
                }
            }
        });
    }

    /// Send a native desktop toast notification across GNOME, KDE, and all Linux desktops.
    fn notify(title: &str, body: &str) {
        // Try notify-send first (direct DBus notification to org.freedesktop.Notifications for GNOME/KDE).
        let res = std::process::Command::new("notify-send")
            .arg("-i")
            .arg("swai")
            .arg("-a")
            .arg("SWAI")
            .arg(title)
            .arg(body)
            .status();

        if res.is_ok() && res.unwrap().success() {
            return;
        }

        // Fallback to GIO Application notification if notify-send is unavailable.
        let notification = Notification::new("");
        notification.set_title(title);
        notification.set_body(Some(body));
        notification.set_icon(&gio::ThemedIcon::new("swai"));

        let app = gio::Application::default()
            .and_then(|app| app.downcast::<adw::Application>().ok());

        if let Some(app) = app {
            app.send_notification(Some("swai-notification"), &notification);
        }
    }

    /// Spawn a health monitor thread for the given model.
    ///
    /// Bridges `ProcessManager::start_model_and_report` (which uses `Sender<ModelState>`)
    /// to the main channel (`Sender<ChannelMessage>`), converting state updates
    /// into `ChannelMessage::StateUpdate` variants.
    ///
    /// If the model is **already running** (e.g. started by the toggle handler
    /// before this function was called), we skip `start_model_and_report` and
    /// instead directly begin polling the model's port so the UI receives the
    /// correct Ready/Loading/Starting transitions instead of staying stuck on
    /// the initial Starting state.
    fn spawn_health_monitor(
        pm: Arc<Mutex<ProcessManager>>,
        sender: std::sync::mpsc::Sender<ChannelMessage>,
        model_id: String,
    ) {
        // Create a separate channel for ModelState → convert to ChannelMessage in main thread.
        let (health_tx, health_rx) = std::sync::mpsc::channel::<ModelState>();

        // Check if the model is already running. If so, the toggle handler
        // already started it — don't call start_model_and_report (which would
        // fail with AnotherModelRunning and never poll).
        let already_running = pm.lock()
            .ok()
            .map(|p| p.find_running_model(&model_id).is_some())
            .unwrap_or(false);

        if already_running {
            // Model already started — just poll its port until Ready or timeout.
            let port = pm.lock()
                .ok()
                .and_then(|p| p.get_port_for_model(&model_id));

            if let Some(port) = port {
                let monitor = swai_core::health_monitor::HealthMonitor::new(port, 30);
                std::thread::spawn(move || {
                    monitor.wait_until_ready_with_updates(health_tx);
                });
            }
        } else {
            // Model not yet running — start it and report.
            let health_pm = Arc::clone(&pm);
            let health_model_id = model_id.clone();
            std::thread::spawn(move || {
                if let Ok(mut pm) = health_pm.lock() {
                    let _ = pm.start_model_and_report(&health_model_id, health_tx);
                }
            });
        }

        // Drain the health channel and convert to ChannelMessage.
        while let Ok(state) = health_rx.recv() {
            let _ = sender.send(ChannelMessage::StateUpdate {
                model_id: model_id.clone(),
                state,
            });
        }
    }

    // ─── Phase 20: Background Update Checker ────────────────────────────

    /// Check for SWAI updates in the background. If a newer version is available,
    /// log it. The UI notification is handled by the manual check buttons.
    fn check_for_update_background(
        github_repo: &str,
        current_version: &str,
    ) {
        // Run the update check on a background thread to avoid blocking the UI.
        let bg_github_repo = github_repo.to_string();
        let bg_current_version = current_version.to_string();

        std::thread::spawn(move || {
            let result = crate::update_checker::check_for_updates_blocking(
                &bg_github_repo,
                &bg_current_version,
            );

            match result {
                crate::update_checker::UpdateCheckResult::UpdateAvailable { version, .. } => {
                    tracing::info!("Background update check: update available v{}", version);
                }
                crate::update_checker::UpdateCheckResult::NoUpdate => {
                    tracing::debug!("Background update check: no update available");
                }
                crate::update_checker::UpdateCheckResult::Error(e) => {
                    tracing::warn!("Background update check failed: {}", e);
                }
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    /// Verify that adopting an unmanaged server writes the model into config.toml.
    #[test]
    fn test_adopt_model_registers_into_config() {
        let tmp = tempfile::tempdir().unwrap();
        let script_path = tmp.path().join("adopt-test.sh");
        let mut f = std::fs::File::create(&script_path).unwrap();
        writeln!(f, "#!/bin/sh\nllama-server --port 8096").unwrap();
        drop(f);

        // Create a minimal config.toml with no models.
        let config_path = tmp.path().join("config.toml");
        std::fs::write(&config_path, "schema_version = 1\n").unwrap();

        let model = ImportedModel {
            id: "adopted-model".to_string(),
            name: "Adopted Model".to_string(),
            script_path: script_path.clone(),
            port: 8096,
            health_timeout_sec: 30,
        };

        let result = MainWindow::append_model_to_config_at(&config_path, &model);
        assert!(result.is_ok(), "Adoption should succeed: {:?}", result.err());

        // Reload and verify the model was registered.
        let content = std::fs::read_to_string(&config_path).unwrap();
        let config: Config = toml::from_str(&content).unwrap();
        assert_eq!(config.models.len(), 1);
        assert_eq!(config.models[0].id, "adopted-model");
        assert_eq!(config.models[0].name, "Adopted Model");
        assert_eq!(config.models[0].port, 8096);
        assert_eq!(config.models[0].script_path, script_path);
    }

    /// Verify that adopting a model with a duplicate port is rejected.
    #[test]
    fn test_adopt_model_duplicate_port_rejected() {
        let tmp = tempfile::tempdir().unwrap();
        let script_path = tmp.path().join("adopt-dup.sh");
        let mut f = std::fs::File::create(&script_path).unwrap();
        writeln!(f, "#!/bin/sh\nllama-server --port 9090").unwrap();
        drop(f);

        // Create a config.toml with an existing model on port 9090.
        let config_path = tmp.path().join("config.toml");
        let existing_script = tmp.path().join("existing.sh");
        std::fs::write(&existing_script, "#!/bin/sh\necho existing").unwrap();

        let config_content = format!(
            "schema_version = 1\n\n[[models]]\nid = \"existing\"\nname = \"Existing\"\nscript_path = \"{}\"\nport = 9090\nhealth_timeout_sec = 30\n",
            existing_script.display()
        );
        std::fs::write(&config_path, &config_content).unwrap();

        let model = ImportedModel {
            id: "duplicate-port".to_string(),
            name: "Duplicate Port".to_string(),
            script_path: script_path.clone(),
            port: 9090, // Same port as existing model
            health_timeout_sec: 30,
        };

        let result = MainWindow::append_model_to_config_at(&config_path, &model);
        assert!(result.is_err(), "Duplicate port should be rejected");
        let err_msg = result.unwrap_err();
        assert!(
            err_msg.contains("duplicate") || err_msg.contains("Duplicate"),
            "Error should mention duplicate port: {}",
            err_msg
        );

        // Config should be unchanged.
        let content = std::fs::read_to_string(&config_path).unwrap();
        let config: Config = toml::from_str(&content).unwrap();
        assert_eq!(config.models.len(), 1);
    }

    /// Verify that adopting a model with a missing script is rejected.
    #[test]
    fn test_adopt_model_missing_script_rejected() {
        let tmp = tempfile::tempdir().unwrap();
        let config_path = tmp.path().join("config.toml");
        std::fs::write(&config_path, "schema_version = 1\n").unwrap();

        let model = ImportedModel {
            id: "no-script".to_string(),
            name: "No Script".to_string(),
            script_path: tmp.path().join("nonexistent.sh"),
            port: 7777,
            health_timeout_sec: 30,
        };

        let result = MainWindow::append_model_to_config_at(&config_path, &model);
        assert!(result.is_err(), "Missing script should be rejected");
    }
}
