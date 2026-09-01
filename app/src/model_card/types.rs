/// UI-visible model state.
#[derive(Debug, Clone, PartialEq)]
pub enum CardState {
    Stopped,
    Starting,
    Loading,
    Ready,
    Error(String),
}

impl CardState {
    pub fn status_text(&self) -> &str {
        match self {
            Self::Stopped => "Stopped",
            Self::Starting => "Starting...",
            Self::Loading => "Loading...",
            Self::Ready => "Ready",
            Self::Error(msg) => msg,
        }
    }

    pub fn is_on(&self) -> bool {
        matches!(self, Self::Ready | Self::Starting | Self::Loading)
    }

    pub fn is_transitioning(&self) -> bool {
        matches!(self, Self::Starting | Self::Loading)
    }
}

/// Context polling state for a single model card.
#[derive(Debug, Clone, PartialEq)]
#[allow(dead_code)]
pub enum PollingState {
    /// No polling active (model stopped or not yet started).
    Inactive,
    /// Polling /slots is active and the last check succeeded.
    Active {
        /// Tokens used in the current slot.
        tokens_used: usize,
        /// Total context window size (n_ctx).
        n_ctx: usize,
    },
    /// Polling is active but the last /slots request failed.
    Error,
}

impl PollingState {
    #[allow(dead_code)]
    pub fn is_active(&self) -> bool {
        !matches!(self, Self::Inactive)
    }
}

/// Live speed metrics for a model card.
#[derive(Debug, Clone, Default)]
#[allow(dead_code)]
pub struct SpeedMetrics {
    /// Generation speed (predicted_per_second) in tok/s.
    pub predicted_per_second: f64,
    /// Prompt evaluation speed (prompt_per_second) in p-tok/s.
    pub prompt_per_second: f64,
    /// Live elapsed duration in seconds. `None` when no active request.
    pub elapsed_duration_sec: Option<f64>,
}
