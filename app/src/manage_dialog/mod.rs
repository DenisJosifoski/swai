//! SWAI — Manage models dialog subsystem.

pub mod dialog;
pub mod edit_dialog;
pub mod helpers;
pub mod sync_port;
#[cfg(test)]
mod tests;

pub use dialog::ManageModelsDialog;
