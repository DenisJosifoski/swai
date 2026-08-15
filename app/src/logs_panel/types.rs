/// View mode for the log viewer window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViewMode {
    /// Live tail of the model's log file.
    Logs,
    /// Read-only display of the session checkpoint file.
    Checkpoints,
}
