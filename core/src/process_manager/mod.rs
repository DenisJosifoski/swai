//! SWAI — Process manager subsystem.

pub mod error;
pub mod guard;
pub mod manager;
#[cfg(test)]
mod tests;
pub mod types;

pub use error::ProcessError;
pub use guard::{LinuxProcessGuard, ProcessGuard};
pub use manager::ProcessManager;
pub use nix::unistd::Pid;
pub use types::{ModelState, PortState, RunningModel};
