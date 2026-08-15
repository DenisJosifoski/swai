use std::collections::HashMap;

/// Shared state between the proxy server and the application.
///
/// Updated by the app whenever a model starts, stops, switches, or restarts.
/// The proxy reads this state on every incoming request to decide where to
/// forward (or whether to return 503).
///
/// In multi-model mode, `active_models` holds all concurrently running models;
/// `primary_port` is the port of the first-started model (fallback target).
#[derive(Debug, Clone, Default)]
pub struct ProxyState {
    /// The port of the primary (first-started) active model server.
    /// `None` means no model is running.
    pub primary_port: Option<u16>,

    /// All currently running models, keyed by their configured id.
    /// Each entry maps to the port its server is bound to.
    pub active_models: HashMap<String, u16>,

    /// Whether any model is currently in a transitional state (starting / restarting).
    /// When `true`, the proxy returns 503 even if ports are set, because
    /// models on those ports are not yet Ready to serve requests.
    pub is_loading: bool,
}

impl ProxyState {
    /// Create a new proxy state with no active model.
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the primary target port and mark all models as loaded (Ready).
    pub fn set_target(&mut self, port: u16) {
        self.primary_port = Some(port);
        self.is_loading = false;
    }

    /// Register a running model (id → port mapping) with the proxy state.
    pub fn add_model(&mut self, id: String, port: u16) {
        self.active_models.insert(id, port);
        // First model added becomes the primary.
        if self.primary_port.is_none() {
            self.primary_port = Some(port);
        }
        self.is_loading = false;
    }

    /// Sync the proxy state with the full set of running models from the
    /// ProcessManager. Replaces the entire `active_models` map and recomputes
    /// `primary_port` from the first entry.
    ///
    /// Call this after any model start/stop to keep the proxy state consistent
    /// for dynamic multi-model routing.
    pub fn sync_models(&mut self, models: Vec<(String, u16)>) {
        self.active_models.clear();
        self.primary_port = None;
        for (id, port) in models {
            self.active_models.insert(id.clone(), port);
            if self.primary_port.is_none() {
                self.primary_port = Some(port);
            }
        }
        self.is_loading = false;
    }

    /// Remove a running model from the proxy state by id.
    pub fn remove_model(&mut self, id: &str) -> Option<u16> {
        let port = self.active_models.remove(id);
        // If we removed the primary, shift to the next available model.
        if let Some(p) = port {
            if self.primary_port == Some(p) {
                self.primary_port = self.active_models.values().next().copied();
            }
        }
        port
    }

    /// Look up the port for a running model by id or name.
    ///
    /// Checks both `id` and `name` fields from config. Returns `None` if not found.
    pub fn find_model_port(&self, identifier: &str) -> Option<u16> {
        // Direct id match
        if let Some(&port) = self.active_models.get(identifier) {
            return Some(port);
        }
        // Name match — caller should pass the config to resolve names.
        // This is handled at a higher level by the app.
        None
    }

    /// Mark the proxy as loading (model is starting/restarting).
    pub fn set_loading(&mut self) {
        self.is_loading = true;
    }

    /// Clear all model state and mark as not loading.
    pub fn clear(&mut self) {
        self.active_models.clear();
        self.primary_port = None;
        self.is_loading = false;
    }
}
