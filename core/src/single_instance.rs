//! Single-instance enforcement using the `single-instance` crate.
//!
//! On second instance, prints a message and exits immediately — no silent
//! failure.

use single_instance::SingleInstance;
use tracing::{info, warn};

/// Guard that ensures only one instance of SWAI runs at a time.
pub struct SingleInstanceGuard {
    instance: Option<SingleInstance>,
}

impl SingleInstanceGuard {
    /// Try to acquire the single-instance lock.
    ///
    /// Returns `Ok(Self)` if this is the first instance, or `Err(AlreadyRunning)`
    /// if another instance is already running.
    ///
    /// Bypasses the lock entirely when `SWAI_NO_SINGLE_INSTANCE=1` is set
    /// (used by integration tests that run alongside a live SWAI instance).
    pub fn try_acquire() -> Result<Self, AlreadyRunning> {
        // Test bypass: skip single-instance check when env var is set.
        if std::env::var("SWAI_NO_SINGLE_INSTANCE").ok().as_deref() == Some("1") {
            info!("single-instance guard bypassed (SWAI_NO_SINGLE_INSTANCE=1)");
            return Ok(Self { instance: None });
        }

        let instance = SingleInstance::new("swai").map_err(|e| {
            warn!("another instance of SWAI is already running: {}", e);
            AlreadyRunning
        })?;

        if !instance.is_single() {
            warn!("another instance of SWAI is already running");
            return Err(AlreadyRunning);
        }

        info!("single-instance guard acquired");
        Ok(Self { instance: Some(instance) })
    }

    /// Release the single-instance lock.
    pub fn release(&mut self) {
        if let Some(instance) = self.instance.take() {
            // The single-instance crate drops the lock automatically when dropped,
            // but we want to log it. Just drop the instance.
            drop(instance);
            info!("single-instance guard released");
        }
        // If instance is None, the guard was bypassed (test mode) — nothing to release.
    }
}

impl Drop for SingleInstanceGuard {
    fn drop(&mut self) {
        self.release();
    }
}

/// Error type for already-running instances.
#[derive(Debug, Clone)]
pub struct AlreadyRunning;

impl std::fmt::Display for AlreadyRunning {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "another instance of SWAI is already running")
    }
}

impl std::error::Error for AlreadyRunning {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_already_running_error_display() {
        let err = AlreadyRunning;
        assert_eq!(format!("{}", err), "another instance of SWAI is already running");
    }
}
