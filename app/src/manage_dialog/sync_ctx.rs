//! Context-size sync helper — thin wrapper around core's inference module.
//!
//! Used by the Edit Model dialog to push a new ctx-size value into the
//! `.sh` launch script on disk while preserving comments and formatting.

use std::path::PathBuf;

use swai_core::import_wizard::sync_ctx_size_in_script as core_sync_ctx;

/// Synchronize the context size inside a `.sh` launch script.
///
/// Wraps `swai_core::import_wizard::sync_ctx_size_in_script` with a
/// `PathBuf` interface for the UI layer.
pub fn sync_ctx_size_in_script(script_path: &PathBuf, new_ctx: usize) -> Result<(), String> {
    core_sync_ctx(script_path, new_ctx)
}
