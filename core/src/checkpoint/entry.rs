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
        lines.push(
            "[Session checkpoint — earlier work in this conversation, condensed]".to_string(),
        );
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
