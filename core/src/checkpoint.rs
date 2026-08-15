//! In-memory session checkpointing for message compaction.
//!
//! When Anthropic Messages API conversations grow beyond the model's context
//! window, older messages are evicted (compacted). This module captures those
//! evicted message slices, summarizes them into concise text lines, and
//! formats them into a `[Session checkpoint]` block that gets injected after
//! the system prompt on subsequent requests — so the model retains awareness
//! of earlier work without re-sending the full history.
//!
//! ## Data flow
//!
//! 1. `compact_messages_anthropic()` drops a slice of evicted messages.
//! 2. `extract_action_lines()` converts each message into plain-text action lines
//!    (e.g., "Read src/lib.rs", "Edited main.rs", "Ran command: cargo build").
//! 3. The summary is stored in a `SessionCheckpoint` keyed by `session_id`.
//! 4. Before sending the next request, `format_for_injection()` builds the
//!    checkpoint block that gets inserted into the messages array.

use serde::{Deserialize, Serialize};

/// A single summarized checkpoint entry representing one compaction event.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CheckpointEntry {
    /// Monotonically increasing index (1-based).
    pub index: usize,
    /// RFC 3339 timestamp of when this compaction occurred.
    pub timestamp: String,
    /// Plain-text summary lines produced from the dropped message slice.
    pub summary_lines: Vec<String>,
}

/// In-memory state of checkpoints for an ongoing session.
///
/// A session is identified by a `session_id` string (typically derived from
/// the Anthropic Messages API request's prompt hash or a user-provided ID).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionCheckpoint {
    pub session_id: String,
    pub initial_objective: Option<String>,
    pub entries: Vec<CheckpointEntry>,
}

impl SessionCheckpoint {
    /// Create a new checkpoint registry for the given session.
    pub fn new(session_id: String) -> Self {
        Self {
            session_id,
            initial_objective: None,
            entries: Vec::new(),
        }
    }

    /// Set the initial user objective for this session.
    pub fn set_initial_objective(&mut self, objective: String) {
        self.initial_objective = Some(objective);
    }

    /// Append a new summary entry with auto-assigned index and timestamp.
    pub fn add_entry(&mut self, summary_lines: Vec<String>) {
        let next_index = self.entries.len() + 1;
        self.entries.push(CheckpointEntry {
            index: next_index,
            timestamp: chrono::Utc::now().to_rfc3339(),
            summary_lines,
        });
    }

    /// Format all checkpoint entries into a clean markdown block for prompt injection.
    pub fn format_for_injection(&self) -> Option<String> {
        if self.entries.is_empty() && self.initial_objective.is_none() {
            return None;
        }

        let mut lines = Vec::new();
        lines.push("[Session checkpoint — earlier work in this conversation, condensed]".to_string());
        if let Some(ref obj) = self.initial_objective {
            lines.push(format!("Initial Objective: {}", obj));
        }
        lines.push("Note: this is a condensed action log, not literal file content. If you need exact field names, types, function signatures, or other precise code details from a file listed below, re-read that file — do not reconstruct it from memory.".to_string());

        let mut line_num = 1;
        for entry in &self.entries {
            for line in &entry.summary_lines {
                lines.push(format!("{}. {}", line_num, line));
                line_num += 1;
            }
        }

        lines.push("[End checkpoint — continuing below]".to_string());
        Some(lines.join("\n"))
    }

    /// Serialize all entries into the disk-persistence markdown format.
    ///
    /// Produces a multi-section document where each `CheckpointEntry` becomes
    /// its own `## Checkpoint #N (M messages compacted)` heading followed by
    /// the numbered summary lines from that entry.
    pub fn to_disk_format(&self) -> String {
        let mut out = String::new();
        out.push_str(&format!(
            "# SWAI Session Checkpoint Log\n\
             **Session ID:** `{}`\n",
            self.session_id,
        ));

        if let Some(ref obj) = self.initial_objective {
            out.push_str(&format!("**Initial Objective:** `{}`\n", obj));
        }

        out.push_str(&format!(
            "**Last Updated:** `{}Z`\n",
            chrono::Utc::now().to_rfc3339()
        ));

        let mut global_line = 1;
        for (idx, entry) in self.entries.iter().enumerate() {
            out.push_str(&format!(
                "\n## Checkpoint #{} ({} messages compacted)\n",
                idx + 1,
                entry.summary_lines.len()
            ));
            for line in &entry.summary_lines {
                out.push_str(&format!("{}. {}\n", global_line, line));
                global_line += 1;
            }
        }

        out
    }

    /// Return the number of checkpoint entries.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Check if there are no checkpoint entries.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

/// Global registry of active session checkpoints.
///
/// Thread-safe via `Arc<Mutex<>>` so multiple threads (e.g., proxy request
/// handlers, compaction workers) can read/write concurrently.
#[derive(Debug, Default, Clone)]
pub struct CheckpointRegistry {
    sessions: std::sync::Arc<std::sync::Mutex<
        std::collections::HashMap<String, SessionCheckpoint>,
    >>,
}

impl CheckpointRegistry {
    /// Create a new empty registry.
    pub fn new() -> Self {
        Self {
            sessions: std::sync::Arc::new(std::sync::Mutex::new(
                std::collections::HashMap::new(),
            )),
        }
    }

    /// Look up or create a session checkpoint for the given session ID.
    /// Returns a cloned copy of the session checkpoint.
    pub fn get_or_create(&self, session_id: &str) -> SessionCheckpoint {
        let mut sessions = self.sessions.lock().unwrap_or_else(|e| e.into_inner());
        sessions
            .entry(session_id.to_string())
            .or_insert_with(|| SessionCheckpoint::new(session_id.to_string()))
            .clone()
    }

    /// Look up a session checkpoint by ID, returning a cloned copy if found.
    pub fn get(&self, session_id: &str) -> Option<SessionCheckpoint> {
        let sessions = self.sessions.lock().unwrap_or_else(|e| e.into_inner());
        sessions.get(session_id).cloned()
    }

    /// Remove a session checkpoint by ID.
    pub fn remove(&self, session_id: &str) {
        let mut sessions = self.sessions.lock().unwrap_or_else(|e| e.into_inner());
        sessions.remove(session_id);
    }

    /// Iterate over all active sessions and their checkpoint injection text.
    pub fn format_all(&self) -> Vec<(String, String)> {
        let sessions = self.sessions.lock().unwrap_or_else(|e| e.into_inner());
        sessions
            .iter()
            .filter_map(|(id, sc)| {
                sc.format_for_injection().map(|text| (id.clone(), text))
            })
            .collect()
    }
}

/// Persistent disk writer for session checkpoint logs.
///
/// Writes a markdown-formatted checkpoint file to
/// `~/.local/share/swai/checkpoints/<session_id>.md`. The first write
/// creates the file with a header; subsequent writes append new checkpoint
/// sections. Thread-safe via an internal mutex.
#[derive(Debug, Clone)]
pub struct CheckpointWriter {
    /// The absolute path to the checkpoint file.
    file_path: std::path::PathBuf,
    /// Mutex-guarded state to serialize writes across threads.
    state: std::sync::Arc<std::sync::Mutex<CheckpointWriterState>>,
}

struct CheckpointWriterState {
    /// Whether the header has been written yet.
    header_written: bool,
}

impl std::fmt::Debug for CheckpointWriterState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CheckpointWriterState")
            .field("header_written", &self.header_written)
            .finish()
    }
}

impl CheckpointWriter {
    /// Create a new writer for the given session ID.
    ///
    /// The checkpoint file path is `~/.local/share/swai/checkpoints/<session_id>.md`.
    /// The parent directory is created automatically if it does not exist.
    pub fn new(session_id: &str) -> std::io::Result<Self> {
        Self::new_in_dir(Self::default_base_dir(), session_id)
    }

    /// Create a new writer for the given session ID in a specific directory.
    pub fn new_in_dir(base: std::path::PathBuf, session_id: &str) -> std::io::Result<Self> {
        std::fs::create_dir_all(&base)?;
        let file_path = base.join(format!("{}.md", session_id));
        Ok(Self {
            file_path,
            state: std::sync::Arc::new(std::sync::Mutex::new(CheckpointWriterState {
                header_written: false,
            })),
        })
    }

    /// Return the default base directory for checkpoint files.
    ///
    /// `~/.local/share/swai/checkpoints/` (or `$XDG_DATA_HOME/swai/checkpoints/`).
    pub fn default_base_dir() -> std::path::PathBuf {
        if let Ok(xdg) = std::env::var("XDG_DATA_HOME") {
            std::path::PathBuf::from(&xdg).join("swai").join("checkpoints")
        } else if let Ok(home) = std::env::var("HOME") {
            std::path::PathBuf::from(home).join(".local").join("share").join("swai").join("checkpoints")
        } else {
            std::path::PathBuf::from("checkpoints")
        }
    }

    /// Return the path to this writer's checkpoint file.
    pub fn file_path(&self) -> &std::path::Path {
        &self.file_path
    }

    /// Count how many checkpoint sections are already present in the on-disk file.
    pub fn existing_checkpoint_count(&self) -> usize {
        if let Ok(content) = std::fs::read_to_string(&self.file_path) {
            content
                .lines()
                .filter(|l| l.trim_start().starts_with("## Checkpoint #"))
                .count()
        } else {
            0
        }
    }

    /// Determine the next checkpoint index (1-based) based on on-disk history.
    pub fn next_checkpoint_index(&self) -> usize {
        self.existing_checkpoint_count() + 1
    }

    /// Write (or append) a new checkpoint entry to disk.
    pub fn write_entry(&self, entry: &CheckpointEntry) -> std::io::Result<()> {
        self.write_entry_with_objective(entry, None)
    }

    /// Write (or append) a new checkpoint entry to disk with optional initial objective.
    pub fn write_entry_with_objective(
        &self,
        entry: &CheckpointEntry,
        objective: Option<&str>,
    ) -> std::io::Result<()> {
        let _lock = self.state.lock().map_err(|e| {
            std::io::Error::new(
                std::io::ErrorKind::Other,
                format!("checkpoint writer lock poisoned: {}", e),
            )
        })?;

        // Derive session_id from the file name (strip .md).
        let session_id = self.file_path
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default();

        if !self.file_path.exists() {
            // First write: create the file with the header.
            let mut header = format!(
                "# SWAI Session Checkpoint Log\n\
                 **Session ID:** `{}`\n",
                session_id,
            );
            if let Some(obj) = objective {
                header.push_str(&format!("**Initial Objective:** `{}`\n", obj));
            }
            header.push_str(&format!(
                "**Last Updated:** `{}Z`\n",
                chrono::Utc::now().to_rfc3339()
            ));
            std::fs::write(&self.file_path, &header)?;
        }

        // Compute actual 1-based checkpoint index
        let actual_index = if entry.index > 0 {
            entry.index
        } else {
            self.next_checkpoint_index()
        };

        // Append a new checkpoint section.
        let section = format!(
            "\n## Checkpoint #{} ({} messages compacted)\n",
            actual_index,
            entry.summary_lines.len()
        );
        let mut lines = String::new();
        for (i, line) in entry.summary_lines.iter().enumerate() {
            lines.push_str(&format!("{}. {}\n", i + 1, line));
        }

        // Append to existing file without destroying prior sections
        use std::io::Write;
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.file_path)?;
        file.write_all(format!("{}{}", section, lines).as_bytes())?;

        Ok(())
    }

    /// Write a complete checkpoint log from a `SessionCheckpoint` (all entries).
    ///
    /// This overwrites the file entirely with all entries formatted as sections.
    pub fn write_snapshot(&self, session: &SessionCheckpoint) -> std::io::Result<()> {
        let content = session.to_disk_format();
        std::fs::write(&self.file_path, content)?;
        Ok(())
    }

    /// Read the checkpoint file contents, returning an empty string if it
    /// does not exist.
    pub fn read_contents(&self) -> String {
        std::fs::read_to_string(&self.file_path).unwrap_or_default()
    }
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_session_checkpoint_new_is_empty() {
        let sc = SessionCheckpoint::new("test-session".to_string());
        assert!(sc.is_empty());
        assert_eq!(sc.len(), 0);
        assert_eq!(sc.format_for_injection(), None);
    }

    #[test]
    fn test_session_checkpoint_add_entry() {
        let mut sc = SessionCheckpoint::new("test-session".to_string());
        sc.add_entry(vec!["Read src/lib.rs".to_string()]);
        assert_eq!(sc.len(), 1);
        assert!(!sc.is_empty());

        // Check the entry index and timestamp format.
        let entry = &sc.entries[0];
        assert_eq!(entry.index, 1);
        assert!(!entry.timestamp.is_empty());
        assert_eq!(entry.summary_lines, vec!["Read src/lib.rs"]);
    }

    #[test]
    fn test_session_checkpoint_format_for_injection_single_entry() {
        let mut sc = SessionCheckpoint::new("test".to_string());
        sc.add_entry(vec![
            "Read src/lib.rs, core/src/config.rs".to_string(),
            "Added pub mod update_checker".to_string(),
        ]);

        let formatted = sc.format_for_injection().unwrap();
        assert!(formatted.starts_with("[Session checkpoint"));
        assert!(formatted.contains("Note: this is a condensed action log, not literal file content. If you need exact field names, types, function signatures, or other precise code details from a file listed below, re-read that file — do not reconstruct it from memory."));
        assert!(formatted.contains("1. Read src/lib.rs, core/src/config.rs"));
        assert!(formatted.contains("2. Added pub mod update_checker"));
        assert!(formatted.contains("[End checkpoint"));
    }

    #[test]
    fn test_session_checkpoint_format_for_injection_multiple_entries() {
        let mut sc = SessionCheckpoint::new("test".to_string());
        sc.add_entry(vec!["Read src/lib.rs".to_string()]);
        sc.add_entry(vec![
            "Edited main.rs".to_string(),
            "Ran command: cargo build".to_string(),
        ]);

        let formatted = sc.format_for_injection().unwrap();
        // First entry lines should appear before second entry lines.
        let pos_first = formatted.find("Read src/lib.rs").unwrap();
        let pos_second_start = formatted.find("Edited main.rs").unwrap();
        assert!(pos_first < pos_second_start);

        // Should contain numbered lines from both entries.
        assert!(formatted.contains("1. Read src/lib.rs"));
        assert!(formatted.contains("2. Edited main.rs"));
        assert!(formatted.contains("3. Ran command: cargo build"));
    }

    #[test]
    fn test_session_checkpoint_format_for_injection_empty_returns_none() {
        let sc = SessionCheckpoint::new("test".to_string());
        assert_eq!(sc.format_for_injection(), None);
    }

    #[test]
    fn test_session_checkpoint_sequential_compactions_append() {
        let mut sc = SessionCheckpoint::new("session-1".to_string());

        // First compaction.
        sc.add_entry(vec!["Read src/lib.rs".to_string()]);
        assert_eq!(sc.entries[0].index, 1);

        // Second compaction — should not overwrite.
        sc.add_entry(vec!["Edited main.rs".to_string()]);
        assert_eq!(sc.entries.len(), 2);
        assert_eq!(sc.entries[1].index, 2);

        // Third compaction.
        sc.add_entry(vec!["Ran command: cargo test".to_string()]);
        assert_eq!(sc.entries.len(), 3);
        assert_eq!(sc.entries[2].index, 3);

        // All entries preserved.
        let formatted = sc.format_for_injection().unwrap();
        assert!(formatted.contains("1. Read src/lib.rs"));
        assert!(formatted.contains("2. Edited main.rs"));
        assert!(formatted.contains("3. Ran command: cargo test"));
    }

    #[test]
    fn test_checkpoint_registry_get_or_create() {
        let registry = CheckpointRegistry::new();

        // First access creates a new session.
        let sc1 = registry.get_or_create("session-a");
        assert_eq!(sc1.session_id, "session-a");
        assert!(sc1.is_empty());

        // Second access returns the same session (same ID).
        let sc2 = registry.get_or_create("session-a");
        assert_eq!(sc2.session_id, "session-a");
    }

    #[test]
    fn test_checkpoint_registry_multiple_sessions() {
        let registry = CheckpointRegistry::new();

        // Use get_or_create to get a mutable reference through the lock.
        {
            let mut sessions = registry.sessions.lock().unwrap_or_else(|e| e.into_inner());
            sessions.entry("session-a".to_string())
                .or_insert_with(|| SessionCheckpoint::new("session-a".to_string()))
                .add_entry(vec!["Action A".to_string()]);
        }

        {
            let mut sessions = registry.sessions.lock().unwrap_or_else(|e| e.into_inner());
            sessions.entry("session-b".to_string())
                .or_insert_with(|| SessionCheckpoint::new("session-b".to_string()))
                .add_entry(vec!["Action B1".to_string(), "Action B2".to_string()]);
        }

        // Verify both sessions exist.
        let format_all = registry.format_all();
        assert_eq!(format_all.len(), 2);
    }

    #[test]
    fn test_checkpoint_registry_remove() {
        let registry = CheckpointRegistry::new();

        {
            let mut sessions = registry.sessions.lock().unwrap_or_else(|e| e.into_inner());
            sessions.entry("session-x".to_string())
                .or_insert_with(|| SessionCheckpoint::new("session-x".to_string()))
                .add_entry(vec!["Something".to_string()]);
        }

        assert_eq!(registry.format_all().len(), 1);

        registry.remove("session-x");
        assert_eq!(registry.format_all().len(), 0);
    }

    #[test]
    fn test_checkpoint_entry_serde_roundtrip() {
        let entry = CheckpointEntry {
            index: 42,
            timestamp: "2026-08-14T12:00:00+00:00".to_string(),
            summary_lines: vec![
                "Read src/main.rs".to_string(),
                "Edited config.toml".to_string(),
            ],
        };

        let json = serde_json::to_string(&entry).unwrap();
        let deserialized: CheckpointEntry = serde_json::from_str(&json).unwrap();

        assert_eq!(deserialized.index, 42);
        assert_eq!(deserialized.timestamp, entry.timestamp);
        assert_eq!(deserialized.summary_lines, entry.summary_lines);
    }

    #[test]
    fn test_checkpoint_registry_format_all_skips_empty() {
        let registry = CheckpointRegistry::new();

        // Create an empty session (no entries) — modify via direct lock access.
        {
            let mut sessions = registry.sessions.lock().unwrap_or_else(|e| e.into_inner());
            sessions.entry("empty-session".to_string())
                .or_insert_with(|| SessionCheckpoint::new("empty-session".to_string()));
        }

        // Create a session with entries.
        {
            let mut sessions = registry.sessions.lock().unwrap_or_else(|e| e.into_inner());
            sessions.entry("active-session".to_string())
                .or_insert_with(|| SessionCheckpoint::new("active-session".to_string()))
                .add_entry(vec!["Something happened".to_string()]);
        }

        let formatted = registry.format_all();
        assert_eq!(formatted.len(), 1);
        assert_eq!(formatted[0].0, "active-session");
    }

    // ─── CheckpointWriter disk persistence tests ──────────────────────

    #[test]
    fn test_checkpoint_writer_creates_file() {
        let tmp = tempfile::tempdir().unwrap();
        let writer = CheckpointWriter::new_in_dir(tmp.path().to_path_buf(), "test-session").unwrap();

        let entry = CheckpointEntry {
            index: 1,
            timestamp: chrono::Utc::now().to_rfc3339(),
            summary_lines: vec![
                "Read src/lib.rs".to_string(),
                "Added pub mod config;".to_string(),
            ],
        };
        writer.write_entry(&entry).unwrap();

        let content = writer.read_contents();
        assert!(content.contains("SWAI Session Checkpoint Log"));
        assert!(content.contains("test-session"));
        assert!(content.contains("Read src/lib.rs"));
    }

    #[test]
    fn test_checkpoint_writer_incremental_append() {
        let tmp = tempfile::tempdir().unwrap();
        let writer = CheckpointWriter::new_in_dir(tmp.path().to_path_buf(), "append-session").unwrap();

        // First entry creates the file with header.
        let entry1 = CheckpointEntry {
            index: 1,
            timestamp: chrono::Utc::now().to_rfc3339(),
            summary_lines: vec![
                "Read src/lib.rs".to_string(),
                "Edited main.rs".to_string(),
            ],
        };
        writer.write_entry(&entry1).unwrap();

        // Second entry appends.
        let entry2 = CheckpointEntry {
            index: 2,
            timestamp: chrono::Utc::now().to_rfc3339(),
            summary_lines: vec!["Ran command: cargo build".to_string()],
        };
        writer.write_entry(&entry2).unwrap();

        let content = writer.read_contents();
        // Both entries should be present.
        assert!(content.contains("Read src/lib.rs"));
        assert!(content.contains("Edited main.rs"));
        assert!(content.contains("Ran command: cargo build"));
        // Should have two checkpoint sections.
        assert!(content.contains("## Checkpoint #1"));
        assert!(content.contains("## Checkpoint #2"));
    }

    #[test]
    fn test_checkpoint_writer_snapshot_overwrites() {
        let tmp = tempfile::tempdir().unwrap();
        let writer = CheckpointWriter::new_in_dir(tmp.path().to_path_buf(), "snapshot-session").unwrap();

        // Write some initial entries via write_entry.
        let entry1 = CheckpointEntry {
            index: 1,
            timestamp: chrono::Utc::now().to_rfc3339(),
            summary_lines: vec!["Old action".to_string()],
        };
        writer.write_entry(&entry1).unwrap();

        // Now write a full snapshot (overwrites).
        let mut session = SessionCheckpoint::new("snapshot-session".to_string());
        session.add_entry(vec!["New action A".to_string(), "New action B".to_string()]);
        session.add_entry(vec!["Another action".to_string()]);
        writer.write_snapshot(&session).unwrap();

        let content = writer.read_contents();
        assert!(content.contains("New action A"));
        assert!(content.contains("New action B"));
        assert!(content.contains("Another action"));
        assert!(!content.contains("Old action"));
        // Should have two checkpoint sections from the snapshot.
        assert!(content.contains("## Checkpoint #1"));
        assert!(content.contains("## Checkpoint #2"));
    }

    #[test]
    fn test_checkpoint_writer_read_nonexistent_returns_empty() {
        let tmp = tempfile::tempdir().unwrap();
        let writer = CheckpointWriter::new_in_dir(tmp.path().to_path_buf(), "nonexistent-session").unwrap();
        assert_eq!(writer.read_contents(), "");
    }

    #[test]
    fn test_checkpoint_writer_to_disk_format() {
        let mut sc = SessionCheckpoint::new("format-test".to_string());
        sc.add_entry(vec![
            "Read src/lib.rs, core/src/config.rs".to_string(),
            "Added pub mod update_checker;".to_string(),
        ]);
        sc.add_entry(vec!["Ran cargo check --workspace: passed".to_string()]);

        let formatted = sc.to_disk_format();
        assert!(formatted.starts_with("# SWAI Session Checkpoint Log"));
        assert!(formatted.contains("`format-test`"));
        assert!(formatted.contains("## Checkpoint #1 (2 messages compacted)"));
        assert!(formatted.contains("## Checkpoint #2 (1 messages compacted)"));
        assert!(formatted.contains("1. Read src/lib.rs, core/src/config.rs"));
        assert!(formatted.contains("2. Added pub mod update_checker;"));
        assert!(formatted.contains("3. Ran cargo check --workspace: passed"));
    }

    #[test]
    fn test_checkpoint_writer_default_base_dir() {
        let base = CheckpointWriter::default_base_dir();
        // Should end with checkpoints/
        assert!(base.to_string_lossy().ends_with("checkpoints")
            || base.to_string_lossy().ends_with("checkpoints\\"));
    }

    #[test]
    fn test_checkpoint_writer_multiple_instances_append_without_overwriting() {
        let tmp = tempfile::tempdir().unwrap();

        // First request / compaction event (creates file)
        let writer1 = CheckpointWriter::new_in_dir(tmp.path().to_path_buf(), "multi-compaction").unwrap();
        let idx1 = writer1.next_checkpoint_index();
        assert_eq!(idx1, 1);
        let entry1 = CheckpointEntry {
            index: idx1,
            timestamp: chrono::Utc::now().to_rfc3339(),
            summary_lines: vec!["Read core/src/lib.rs".to_string()],
        };
        writer1.write_entry_with_objective(&entry1, Some("Build feature X")).unwrap();

        // Second request / compaction event (fresh writer instance, must NOT overwrite)
        let writer2 = CheckpointWriter::new_in_dir(tmp.path().to_path_buf(), "multi-compaction").unwrap();
        let idx2 = writer2.next_checkpoint_index();
        assert_eq!(idx2, 2);
        let entry2 = CheckpointEntry {
            index: idx2,
            timestamp: chrono::Utc::now().to_rfc3339(),
            summary_lines: vec!["Read core/src/config.rs".to_string()],
        };
        writer2.write_entry_with_objective(&entry2, Some("Build feature X")).unwrap();

        // Third request / compaction event (fresh writer instance)
        let writer3 = CheckpointWriter::new_in_dir(tmp.path().to_path_buf(), "multi-compaction").unwrap();
        let idx3 = writer3.next_checkpoint_index();
        assert_eq!(idx3, 3);
        let entry3 = CheckpointEntry {
            index: idx3,
            timestamp: chrono::Utc::now().to_rfc3339(),
            summary_lines: vec!["Wrote core/src/feature.rs".to_string()],
        };
        writer3.write_entry_with_objective(&entry3, Some("Build feature X")).unwrap();

        let content = writer3.read_contents();
        assert!(content.contains("**Initial Objective:** `Build feature X`"));
        assert!(content.contains("## Checkpoint #1"));
        assert!(content.contains("Read core/src/lib.rs"));
        assert!(content.contains("## Checkpoint #2"));
        assert!(content.contains("Read core/src/config.rs"));
        assert!(content.contains("## Checkpoint #3"));
        assert!(content.contains("Wrote core/src/feature.rs"));
        assert_eq!(writer3.existing_checkpoint_count(), 3);
    }
}
