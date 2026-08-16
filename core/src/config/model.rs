use serde::Deserialize;
use std::path::PathBuf;

/// A single model configuration entry.
#[derive(Debug, Clone, Deserialize, serde::Serialize)]
pub struct ModelConfig {
    pub id: String,
    pub name: String,
    #[serde(deserialize_with = "deserialize_path")]
    pub script_path: PathBuf,
    pub port: u16,
    #[serde(default = "default_health_timeout")]
    pub health_timeout_sec: u16,
    /// Context window size in tokens (e.g., 65536 for 64k, 131072 for 128k).
    /// Used for dynamic context budgeting. Defaults to 65536 if not specified.
    #[serde(default = "default_ctx_size")]
    pub ctx_size: usize,
}

fn default_health_timeout() -> u16 {
    30
}

fn default_ctx_size() -> usize {
    65_536
}

fn deserialize_path<'de, D>(deserializer: D) -> Result<PathBuf, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let s = String::deserialize(deserializer)?;
    Ok(PathBuf::from(s))
}

/// Deserialize an optional PathBuf where an empty string ("") is treated as
/// None, allowing the accessor's default to kick in.
pub fn deserialize_optional_pathbuf<'de, D>(deserializer: D) -> Result<Option<PathBuf>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let opt = Option::<String>::deserialize(deserializer)?;
    match opt {
        Some(s) if s.is_empty() => Ok(None),
        Some(s) => Ok(Some(PathBuf::from(s))),
        None => Ok(None),
    }
}
