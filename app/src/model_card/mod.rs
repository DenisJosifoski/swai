//! SWAI — Model card widget subsystem.

pub mod types;
pub mod view;
#[cfg(test)]
mod tests;

pub use types::CardState;
pub use view::ModelCard;
