use std::path::PathBuf;
use serde::Deserialize;

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
}

fn default_health_timeout() -> u16 {
    30
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
