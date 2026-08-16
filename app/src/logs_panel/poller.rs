use gtk::prelude::*;
use gtk4 as gtk;
use std::cell::Cell;
use std::fs;
use std::path::{Path, PathBuf};
use std::rc::Rc;

pub fn resolve_log_file(script_path: &Path, log_dir: &Path) -> PathBuf {
    let script_stem = script_path
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();

    if let Ok(entries) = fs::read_dir(log_dir) {
        let mut matches: Vec<PathBuf> = entries
            .filter_map(|e| e.ok())
            .filter_map(|e| {
                let name = e.file_name().to_string_lossy().to_string();
                if !name.ends_with(".log") {
                    return None;
                }
                // Match: {script_stem}_{YYYYMMDD_HHMMSS}.log
                let prefix = format!("{}_", script_stem);
                if !name.starts_with(&prefix) {
                    return None;
                }
                Some(e.path())
            })
            .collect();

        // Sort descending — most recent first (timestamps are zero-padded).
        matches.sort_by(|a, b| b.file_name().cmp(&a.file_name()));

        if let Some(most_recent) = matches.first() {
            return most_recent.clone();
        }
    }

    // Fallback: create a new log file path with the current timestamp.
    let timestamp = chrono::Local::now().format("%Y%m%d_%H%M%S");
    log_dir.join(format!("{}_{}.log", script_stem, timestamp))
}

/// Start the auto-tail polling loop.
///
/// Reads newly appended bytes from the log file every 500ms and appends
/// them to the text buffer. The poller stops when the source ID is removed
/// (which happens in the window's `connect_destroy` handler).
pub fn start_tail_poller(
    log_file: PathBuf,
    text_view: gtk::TextView,
    last_offset: Rc<Cell<usize>>,
    _timeout_id: Rc<Cell<Option<glib::SourceId>>>,
) -> glib::SourceId {
    glib::timeout_add_local(std::time::Duration::from_millis(500), move || {
        // Read the entire file content.
        let content = match fs::read_to_string(&log_file) {
            Ok(c) => c,
            Err(_) => return glib::ControlFlow::Continue, // File doesn't exist yet or unreadable
        };

        let current_offset = last_offset.get();
        let new_bytes_len = content.len().saturating_sub(current_offset);

        if new_bytes_len > 0 {
            let text_buffer = text_view.buffer();
            let mut end_iter = text_buffer.end_iter();
            text_buffer.insert(&mut end_iter, &content[current_offset..]);

            // Auto-scroll to the bottom.
            let bot = text_buffer.end_iter();
            let mut bot_mut = bot;
            text_view.scroll_to_iter(&mut bot_mut, 0.0, true, 0.0, 1.0);

            last_offset.set(current_offset + new_bytes_len);
        } else if content.is_empty() && current_offset > 0 {
            // File was truncated (e.g., by Clear button) — reset offset.
            last_offset.set(0);
        }

        glib::ControlFlow::Continue
    })
}

/// Resolve the checkpoint file path for a given session ID.
///
/// Checkpoint files are stored at `~/.local/share/swai/checkpoints/<session_id>.md`
/// (or `$XDG_DATA_HOME/swai/checkpoints/<session_id>.md`).
pub fn resolve_checkpoint_path(session_id: &str) -> PathBuf {
    // Try XDG_DATA_HOME first, then fallback to ~/.local/share.
    let base = if let Ok(xdg) = std::env::var("XDG_DATA_HOME") {
        PathBuf::from(&xdg).join("swai").join("checkpoints")
    } else if let Ok(home) = std::env::var("HOME") {
        PathBuf::from(home)
            .join(".local")
            .join("share")
            .join("swai")
            .join("checkpoints")
    } else {
        PathBuf::from("checkpoints")
    };

    base.join(format!("{}.md", session_id))
}
