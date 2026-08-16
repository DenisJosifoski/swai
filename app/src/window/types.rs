use swai_core::process_manager::{ModelState, ProcessError};

#[derive(Debug)]
pub enum ChannelMessage {
    /// A switch (start or switch_model) completed.
    SwitchCompleted {
        target_id: String,
        result: Result<(), ProcessError>,
    },
    /// A stop completed.
    StopCompleted {
        running_id: String,
        result: Result<(), ProcessError>,
    },
    /// A restart was manually triggered by the user via the Restart button.
    RestartRequested { model_id: String },
    /// Intermediate state update from health monitor polling.
    /// Used to drive Starting → Loading → Ready transitions in the UI.
    StateUpdate { model_id: String, state: ModelState },
}

/// Messages sent from UI dialogs (import wizard) to the main GUI thread.
#[derive(Debug, Clone)]
pub enum ImportMessage {
    /// A new model was imported and its card should be appended.
    ModelImported {
        model: swai_core::config::ModelConfig,
    },
    /// A model's details (name, port) were updated - refresh the card label live.
    ModelNameUpdated { id: String, name: String, port: u16 },
    /// A model was deleted - remove its card from the UI.
    ModelDeleted { id: String },
}

/// Context update sent from the polling thread to the main loop.
#[derive(Debug, Clone)]
pub struct SlotUpdate {
    pub model_id: String,
    pub tokens_used: usize,
    pub n_ctx: usize,
    pub predicted_per_second: f64,
    #[allow(dead_code)]
    pub prompt_per_second: f64,
}

/// A polled /slots response for a single model.
#[derive(Debug, Clone)]
pub struct SlotInfo {
    pub tokens_used: usize,
    pub n_ctx: usize,
    pub predicted_per_second: f64,
    #[allow(dead_code)]
    pub prompt_per_second: f64,
}
