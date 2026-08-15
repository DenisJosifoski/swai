use std::path::PathBuf;

/// Metadata for a newly configured model produced by the import wizard.
#[derive(Debug, Clone)]
pub struct ImportedModel {
    pub id: String,
    pub name: String,
    pub script_path: PathBuf,
    pub port: u16,
    pub health_timeout_sec: u16,
}
