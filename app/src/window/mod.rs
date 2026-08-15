//! SWAI — Main application window subsystem.

pub mod adoption;
pub mod card_wiring;
pub mod dialogs;
pub mod footer;
pub mod header;
pub mod health;
pub mod poller;
pub mod styles;
pub mod timeout;
pub mod types;
pub mod window;
#[cfg(test)]
mod tests;

pub use types::ImportMessage;
pub use window::MainWindow;
