//! System tray integration for SWAI using ksni (StatusNotifierItem).
//!
//! Implements `ksni::Tray` to provide a context menu with:
//! - Active model label (shows currently running model or "No active model")
//! - Quick-switch entries for each configured model
//! - Show / Hide Window actions
//! - Quit action (stops all models and exits the application)
//!
//! Menu callbacks that need to interact with GTK send messages through channels
//! back to MainWindow's main loop. Model switching is handled by MainWindow
//! (which owns the keep-alive guards and GTK UI updates).

use std::sync::{Arc, Mutex, OnceLock};

use swai_core::config::Config;
use swai_core::process_manager::ProcessManager;
use ksni::blocking::TrayMethods;
use ksni::{blocking::Handle, Category, MenuItem, Status, ToolTip, Tray};

/// Actions sent from the tray menu back to MainWindow's main loop.
pub enum WindowAction {
    /// Hide the main window (minimize to tray).
    Hide,
    /// Show the main window.
    Show,
}

/// Actions sent from the tray quick-switch entries back to MainWindow.
/// MainWindow handles the full switch flow: keep-alive management,
/// background thread spawn, and UI card updates via ChannelMessage.
pub enum TrayAction {
    /// Switch to the model with the given id.
    Switch(String),
}

/// Struct that holds shared quit state accessible from both MainWindow and ksni's
/// background thread. When ksni sets `should_quit = true`, MainWindow detects it
/// in its idle handler and calls `app.quit()`.
#[allow(dead_code)]
#[derive(Default)]
pub struct QuitState {
    pub should_quit: bool,
}


/// The ksni tray implementation for SWAI.
///
/// This struct is owned by ksni and runs on its background thread. Menu
/// callbacks receive `&mut self` and can perform model switching directly via
/// the ProcessManager (no GTK needed) or send window/quit actions through
/// channels back to MainWindow's main loop.
pub struct SwaiTray {
    /// Configuration with model definitions.
    config: Config,
    /// Shared process manager for model lifecycle control.
    process_manager: Arc<Mutex<ProcessManager>>,
    /// Channel sender for window visibility actions (hide/show).
    /// Messages are received by MainWindow's main loop.
    window_sender: std::sync::mpsc::Sender<WindowAction>,
    /// Channel sender for tray quick-switch actions.
    /// MainWindow receives these and handles the full switch flow
    /// (keep-alive, background thread, UI card updates).
    tray_sender: std::sync::mpsc::Sender<TrayAction>,
    /// Channel sender for quit signals.
    /// When a quit message is sent, MainWindow's idle handler detects it and
    /// calls `app.quit()` on the main GTK thread.
    quit_sender: std::sync::mpsc::Sender<()>,
}

impl SwaiTray {
    /// Create a new tray instance.
    pub fn new(
        config: Config,
        process_manager: Arc<Mutex<ProcessManager>>,
        window_sender: std::sync::mpsc::Sender<WindowAction>,
        tray_sender: std::sync::mpsc::Sender<TrayAction>,
        quit_sender: std::sync::mpsc::Sender<()>,
    ) -> Self {
        Self {
            config,
            process_manager,
            window_sender,
            tray_sender,
            quit_sender,
        }
    }

    /// Get the currently running model name, if any.
    fn active_model_name(&self) -> String {
        match self.process_manager.lock() {
            Ok(pm) => pm
                .get_running_model_id()
                .and_then(|id| {
                    self.config
                        .models
                        .iter()
                        .find(|m| m.id == id)
                        .map(|m| format!("● {}", m.name))
                })
                .unwrap_or_else(|| "○ No active model".to_string()),
            Err(_) => "○ No active model".to_string(),
        }
    }

    /// Get the window visibility label based on current state.
    fn window_label(&self) -> String {
        // We don't have direct access to window visibility state here.
        // Default to "Hide Window" — the menu will show this and the user
        // can toggle it. The actual visibility is managed by MainWindow.
        "Hide Window".to_string()
    }
}

impl Tray for SwaiTray {
    /// Category: application status indicator.
    fn category(&self) -> Category {
        Category::ApplicationStatus
    }

    /// Unique identifier required by some system trays to avoid unexpected behaviors.
    fn id(&self) -> String {
        "swai".to_string()
    }

    /// Display title for the tray item.
    fn title(&self) -> String {
        "SWAI".to_string()
    }

    /// Status: active (visible in tray).
    fn status(&self) -> Status {
        Status::Active
    }

    /// Icon name from the freedesktop icon theme.
    fn icon_name(&self) -> String {
        let home = std::env::var("HOME").unwrap_or_else(|_| "/root".to_string());
        format!("{}/.local/share/icons/hicolor/512x512/apps/swai.png", home)
    }

    /// Tooltip shown on hover.
    fn tool_tip(&self) -> ToolTip {
        ToolTip {
            title: "SWAI".to_string(),
            description: format!("Active model: {}", self.active_model_name()),
            ..Default::default()
        }
    }

    /// Build the context menu.
    ///
    /// The menu structure is:
    /// - Active model label (disabled, informational)
    /// - Separator
    /// - Quick-switch entries for each configured model
    /// - Separator
    /// - Show Window / Hide Window
    /// - Quit
    fn menu(&self) -> Vec<MenuItem<Self>> {
        use ksni::menu::*;

        let mut items: Vec<MenuItem<Self>> = Vec::new();

        // ── Active model label (disabled, informational) ──────────
        items.push(
            StandardItem {
                label: self.active_model_name(),
                enabled: false,
                ..Default::default()
            }
            .into(),
        );

        // ── Separator ──────────────────────────────────────────────
        items.push(MenuItem::Separator);

        // ── Quick-switch entries for each configured model ─────────
        for model in &self.config.models {
            let model_id = model.id.clone();
            let model_name = model.name.clone();
            let tray_sender = self.tray_sender.clone();

            items.push(
                StandardItem {
                    label: model_name,
                    enabled: true,
                    activate: Box::new(move |_this: &mut Self| {
                        // Send switch request through channel to MainWindow.
                        // MainWindow handles the full switch flow:
                        // keep-alive management, background thread spawn,
                        // and GTK UI card updates via ChannelMessage.
                        let _ = tray_sender.send(TrayAction::Switch(model_id.clone()));
                    }),
                    ..Default::default()
                }
                .into(),
            );
        }

        // ── Separator ──────────────────────────────────────────────
        items.push(MenuItem::Separator);

        // ── Show Window ────────────────────────────────────────────
        let window_sender_show = self.window_sender.clone();
        items.push(
            StandardItem {
                label: "Show Window".to_string(),
                enabled: true,
                activate: Box::new(move |_this: &mut Self| {
                    let _ = window_sender_show.send(WindowAction::Show);
                    tracing::info!("tray: show window requested");
                }),
                ..Default::default()
            }
            .into(),
        );

        // ── Separator ──────────────────────────────────────────────
        items.push(MenuItem::Separator);

        // ── Hide Window ────────────────────────────────────────────
        let window_label = self.window_label();
        let window_sender_hide = self.window_sender.clone();
        items.push(
            StandardItem {
                label: window_label,
                enabled: true,
                activate: Box::new(move |_this: &mut Self| {
                    // Send hide request through channel to MainWindow's main loop.
                    // The actual show/hide happens on the GTK main thread.
                    let _ = window_sender_hide.send(WindowAction::Hide);
                    tracing::info!("tray: hide window requested");
                }),
                ..Default::default()
            }
            .into(),
        );

        // ── Separator ──────────────────────────────────────────────
        items.push(MenuItem::Separator);

        // ── Quit ───────────────────────────────────────────────────
        let quit_sender = self.quit_sender.clone();
        items.push(
            StandardItem {
                label: "Quit".to_string(),
                enabled: true,
                activate: Box::new(move |_this: &mut Self| {
                    // Send quit signal through channel. MainWindow's idle
                    // handler will call app.quit() on the main GTK thread.
                    let _ = quit_sender.send(());
                    tracing::info!("tray: quit requested");
                }),
                ..Default::default()
            }
            .into(),
        );

        items
    }
}

/// Whether a system-tray host (StatusNotifierWatcher) is registered on the
/// D-Bus session bus.
///
/// Cached once at first call via `OnceLock`.  The check is performed lazily
/// so that it does not block app startup on systems where D-Bus may be slow;
/// however the caller in `main.rs` invokes it eagerly during startup to avoid
/// any blocking inside the GTK signal handler.
static TRAY_HOST_AVAILABLE: OnceLock<bool> = OnceLock::new();

/// Resolve the D-Bus session bus address.
#[allow(dead_code)]
fn session_bus_address() -> String {
    // Prefer $DBUS_SESSION_BUS_ADDRESS (set by systemd --user, etc.)
    if let Ok(addr) = std::env::var("DBUS_SESSION_BUS_ADDRESS") {
        if !addr.is_empty() {
            return addr;
        }
    }
    // Fall back to the standard XDG runtime directory path.
    std::env::var("XDG_RUNTIME_DIR")
        .map(|d| format!("{}/bus", d))
        .unwrap_or_else(|_| "/run/user/0/bus".to_string())
}

/// Check whether a tray host (StatusNotifierWatcher) is registered on the
/// D-Bus session bus.
///
/// This is the desktop-agnostic signal for "a tray icon will actually be
/// visible somewhere."  KDE Plasma registers it natively; GNOME requires the
/// AppIndicator / KStatusNotifierItem extension; many WMs (i3, sway, etc.)
/// do not register it at all.
///
/// Uses `gio::DBusConnection::call_sync` to call
/// `org.freedesktop.DBus.NameHasOwner("org.kde.StatusNotifierWatcher")` on
/// the session bus.  Returns `false` on any error (connection failure, no
/// owner, permission denied).
///
/// **Important:** This function must be called from the GTK main thread
/// because it blocks on a D-Bus call that requires the default main context.
/// The caller in `main.rs` invokes it once during startup to cache the result.
pub fn tray_host_available() -> bool {
    *TRAY_HOST_AVAILABLE.get_or_init(|| {
        let conn = match gio::bus_get_sync(gio::BusType::Session, None::<&gio::Cancellable>) {
            Ok(c) => c,
            Err(e) => {
                tracing::debug!("failed to connect to D-Bus session bus: {e}");
                return false;
            }
        };

        let result = conn.call_sync(
            Some("org.freedesktop.DBus"),
            "/org/freedesktop/DBus",
            "org.freedesktop.DBus",
            "NameHasOwner",
            Some(&glib::Variant::tuple_from_iter(std::iter::once(
                glib::Variant::from("org.kde.StatusNotifierWatcher"),
            ))),
            None,
            gio::DBusCallFlags::NONE,
            3000,
            None::<&gio::Cancellable>,
        );

        match result {
            Ok(reply) => {
                let has_owner = reply.child_value(0).get::<bool>().unwrap_or(false);
                tracing::info!("D-Bus StatusNotifierWatcher owner detected: {}", has_owner);
                has_owner
            }
            Err(e) => {
                tracing::warn!("D-Bus NameHasOwner call failed: {e}");
                false
            }
        }
    })
}

/// Create and spawn the system tray.
///
/// Returns a `Handle<SwaiTray>` that can be used to update the tray state
/// (e.g., refresh the active model label) from other threads.
pub fn create_tray(
    config: Config,
    process_manager: Arc<Mutex<ProcessManager>>,
    window_sender: std::sync::mpsc::Sender<WindowAction>,
    tray_sender: std::sync::mpsc::Sender<TrayAction>,
    quit_sender: std::sync::mpsc::Sender<()>,
) -> Option<Handle<SwaiTray>> {
    let tray = SwaiTray::new(
        config,
        process_manager,
        window_sender,
        tray_sender,
        quit_sender,
    );
    match tray.spawn() {
        Ok(handle) => Some(handle),
        Err(e) => {
            tracing::warn!("failed to spawn ksni tray, continuing without tray icon: {}", e);
            None
        }
    }
}
