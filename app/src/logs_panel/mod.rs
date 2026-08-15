//! SWAI — Live log viewer and session checkpoint panel subsystem.

pub mod poller;
pub mod types;
pub mod window;
#[cfg(test)]
mod tests;

pub use window::LogViewerWindow;
