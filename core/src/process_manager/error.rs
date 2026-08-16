use crate::config;
use thiserror::Error;

/// Error types for process management operations.
#[derive(Debug, Error)]
pub enum ProcessError {
    #[error("another model is already running")]
    AnotherModelRunning,

    #[error("model '{0}' is not running")]
    NotRunning(String),

    #[error("port {port} occupied by unknown process (pid: {pid})")]
    PortOccupiedByUnknownProcess { pid: u32, port: u16 },

    #[error("failed to spawn process: {0}")]
    Spawn(#[from] std::io::Error),

    #[error("process exited unexpectedly with code: {0}")]
    UnexpectedExit(i32),

    #[error("shutdown timeout exceeded for model '{0}'")]
    ShutdownTimeout(String),

    #[error("port {0} still occupied after shutdown timeout")]
    PortStillOccupied(u16),

    #[error("health check failed during startup of '{0}'")]
    HealthCheckFailed(String),

    #[error("I/O error: {0}")]
    Io(std::io::Error),

    #[error("signal error: {0}")]
    Signal(#[from] nix::Error),

    #[error("cannot signal process group — target is the current process group (pid: {pid})")]
    CannotSignalOwnProcessGroup { pid: i32 },
}

impl From<config::ConfigError> for ProcessError {
    fn from(e: config::ConfigError) -> Self {
        ProcessError::Io(std::io::Error::other(format!("config error: {}", e)))
    }
}

impl From<toml::de::Error> for ProcessError {
    fn from(e: toml::de::Error) -> Self {
        ProcessError::Io(std::io::Error::other(format!("toml parse error: {}", e)))
    }
}
