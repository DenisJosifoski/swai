use std::collections::HashMap;

/// Shared state between the proxy server and the application.
///
/// Updated by the app whenever a model starts, stops, switches, or restarts.
/// The proxy reads this state on every incoming request to decide where to
/// forward (or whether to return 503).
///
/// In multi-model mode, `active_models` holds all concurrently running models;
/// `primary_port` is the port of the first-started model (fallback target).
#[derive(Debug, Clone)]
pub struct ProxyState {
    /// The port of the primary (first-started) active model server.
    /// `None` means no model is running.
    pub primary_port: Option<u16>,

    /// All currently running models, keyed by their configured id.
    /// Each entry maps to the port its server is bound to.
    pub active_models: HashMap<String, u16>,

    /// Context window sizes (in tokens) for each running model, keyed by model id.
    /// Used by the compaction budget manager to scale thresholds dynamically.
    pub model_ctx_sizes: HashMap<String, usize>,

    /// Whether any model is currently in a transitional state (starting / restarting).
    /// When `true`, the proxy returns 503 even if ports are set, because
    /// models on those ports are not yet Ready to serve requests.
    pub is_loading: bool,

    /// When `false`, bypasses diff generation for file-write tools and loop
    /// breaker heuristic entirely. The proxy acts as a transparent router.
    pub enable_checkpointing: bool,
}

impl Default for ProxyState {
    fn default() -> Self {
        Self {
            primary_port: None,
            active_models: HashMap::new(),
            model_ctx_sizes: HashMap::new(),
            is_loading: false,
            enable_checkpointing: true,
        }
    }
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
        self.add_model_with_ctx(id, port, 65_536);
    }

    /// Register a running model with its context window size.
    pub fn add_model_with_ctx(&mut self, id: String, port: u16, ctx_size: usize) {
        self.model_ctx_sizes.insert(id.clone(), ctx_size);
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

    /// Look up the context window size (in tokens) for the model running on the given port.
    /// Falls back to 65536 (64k) if the model's context size is not known.
    pub fn ctx_size_for_port(&self, port: u16) -> usize {
        for (id, &p) in &self.active_models {
            if p == port {
                return self.model_ctx_sizes.get(id).copied().unwrap_or(65_536);
            }
        }
        65_536
    }

    /// Mark the proxy as loading (model is starting/restarting).
    pub fn set_loading(&mut self) {
        self.is_loading = true;
    }

    /// Clear all model state and mark as not loading.
    pub fn clear(&mut self) {
        self.active_models.clear();
        self.model_ctx_sizes.clear();
        self.primary_port = None;
        self.is_loading = false;
        self.enable_checkpointing = true;
    }
}
