use serde::de::Error as _;
use thiserror::Error;

/// Error types for configuration operations.
#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("I/O error reading config: {0}")]
    Io(#[from] std::io::Error),

    #[error("TOML parse error: {0}")]
    TomlParse(#[from] toml::de::Error),

    #[error("configuration error: {0}")]
    Validation(String),

    #[error("no config file found at any expected location")]
    NotFound,
}

impl From<toml::ser::Error> for ConfigError {
    fn from(e: toml::ser::Error) -> Self {
        ConfigError::TomlParse(toml::de::Error::custom(e.to_string()))
    }
}
