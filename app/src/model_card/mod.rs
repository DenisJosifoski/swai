//! SWAI — Model card widget subsystem.

#[cfg(test)]
mod tests;
pub mod telemetry;
pub mod types;
pub mod view;

pub use types::CardState;
pub use view::ModelCard;
