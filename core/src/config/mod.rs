//! SWAI — Configuration subsystem.

pub mod config;
pub mod error;
pub mod model;
pub mod preferences;
#[cfg(test)]
mod tests;

pub use config::{example_config, Config};
pub use error::ConfigError;
pub use model::ModelConfig;
pub use preferences::{GlobalSettings, PreferencesConfig};
