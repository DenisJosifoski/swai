//! SWAI — Live log viewer and session checkpoint panel subsystem.

pub mod poller;
#[cfg(test)]
mod tests;
pub mod types;
pub mod window;

pub use window::LogViewerWindow;
