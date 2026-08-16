use serde_json::Value;
use std::collections::HashMap;

/// Tracks session-level behavior to detect loops and excessive planning.
#[derive(Debug, Default, Clone)]
pub struct SessionTracker {
    file_reads: HashMap<String, usize>,
    turns_without_write: usize,
    total_turns: usize,
    last_intervention_turn: usize,
}

impl SessionTracker {
    /// Creates a new, empty SessionTracker.
    pub fn new() -> Self {
        Self::default()
    }

    /// Resets the tracker state.
    pub fn reset(&mut self) {
        self.file_reads.clear();
        self.turns_without_write = 0;
        self.total_turns = 0;
        self.last_intervention_turn = 0;
    }

    /// Records an assistant turn by analyzing the last message in the sequence.
    pub fn record_turn(&mut self, messages: &[Value]) {
        self.total_turns += 1;

        let last_msg = match messages.last() {
            Some(msg) => msg,
            None => {
                self.turns_without_write += 1;
                return;
            }
        };

        let mut has_write_tool = false;

        if let Some(tool_calls) = last_msg.get("tool_calls").and_then(|t| t.as_array()) {
            for tool_call in tool_calls {
                let name = tool_call
                    .get("function")
                    .and_then(|f| f.get("name"))
                    .and_then(|n| n.as_str())
                    .unwrap_or("");
                let args = tool_call
                    .get("function")
                    .and_then(|f| f.get("arguments"))
                    .and_then(|a| a.as_str())
                    .and_then(|s| serde_json::from_str::<Value>(s).ok())
                    .or_else(|| {
                        tool_call
                            .get("function")
                            .and_then(|f| f.get("arguments"))
                            .cloned()
                    });

                // Check for write/command tools
                if is_write_or_command_tool(name) {
                    has_write_tool = true;
                }

                // Check for read tools
                if is_read_tool(name) {
                    if let Some(args) = &args {
                        if let Some(path) = extract_file_path(args) {
                            *self.file_reads.entry(path).or_insert(0) += 1;
                        }
                    }
                }
            }
        } else if let Some(content) = last_msg.get("content").and_then(|c| c.as_array()) {
            // Anthropic style tool use blocks
            for block in content {
                if block.get("type").and_then(|t| t.as_str()) == Some("tool_use") {
                    let name = block.get("name").and_then(|n| n.as_str()).unwrap_or("");
                    let input = block.get("input");

                    if is_write_or_command_tool(name) {
                        has_write_tool = true;
                    }

                    if is_read_tool(name) {
                        if let Some(input) = input {
                            if let Some(path) = extract_file_path(input) {
                                *self.file_reads.entry(path).or_insert(0) += 1;
                            }
                        }
                    }
                }
            }
        }

        if has_write_tool {
            self.turns_without_write = 0;
        } else {
            self.turns_without_write += 1;
        }
    }

    /// Checks if an intervention is needed based on the current state.
    pub fn check_interventions(&mut self) -> Option<String> {
        if self.total_turns > 0 && self.total_turns.saturating_sub(self.last_intervention_turn) < 2
        {
            return None;
        }

        let mut over_read_files = Vec::new();
        for (path, &count) in &self.file_reads {
            if count >= 3 {
                over_read_files.push(format!("{} ({} times)", path, count));
            }
        }
        over_read_files.sort();

        let is_looping_reads = !over_read_files.is_empty();
        let is_looping_turns = self.turns_without_write >= 5;

        if !is_looping_reads && !is_looping_turns {
            return None;
        }

        let mut intervention = String::from("<system-reminder>\n");
        let mut msg_parts = Vec::new();

        if is_looping_reads {
            let files_str = over_read_files.join(" and ");
            msg_parts.push(format!("⚠️ LOOP DETECTED: You have read {} without writing any code. The file contents have not changed.", files_str));
        }

        if is_looping_turns {
            msg_parts.push(format!("You have spent {} consecutive turns reading and planning without producing any code changes.", self.turns_without_write));
        }

        intervention.push_str(&msg_parts.join("\n\n"));
        intervention.push_str(
            "\n\nSTOP READING. STOP PLANNING. Write your implementation now.\n</system-reminder>",
        );

        self.last_intervention_turn = self.total_turns;

        Some(intervention)
    }

    /// Returns a compressed version of a file's content if it has been read before.
    pub fn get_compressed_reread(&self, file_path: &str, full_content: &str) -> Option<String> {
        let read_count = self.file_reads.get(file_path).copied().unwrap_or(0);
        if read_count < 2 {
            return None;
        }

        let lines: Vec<&str> = full_content.lines().collect();
        let total_lines = lines.len();

        if total_lines <= 50 {
            return None; // Too small to compress meaningfully
        }

        let top = lines
            .iter()
            .take(30)
            .copied()
            .collect::<Vec<_>>()
            .join("\n");
        let bottom = lines
            .iter()
            .skip(total_lines.saturating_sub(20))
            .copied()
            .collect::<Vec<_>>()
            .join("\n");

        let omitted = total_lines.saturating_sub(50);

        let compressed = format!(
            "{}\n\n... [{} lines omitted — you have already read this file {} times, refer to your earlier reading] ...\n\n{}",
            top, omitted, read_count, bottom
        );

        Some(compressed)
    }
}

fn is_read_tool(name: &str) -> bool {
    matches!(
        name,
        "Read" | "read" | "ViewFile" | "view_file" | "read_file" | "ReadFile"
    )
}

fn is_write_or_command_tool(name: &str) -> bool {
    matches!(
        name,
        "Edit"
            | "edit"
            | "Write"
            | "write"
            | "write_to_file"
            | "WriteToFile"
            | "ReplaceFileContent"
            | "replace_file_content"
            | "multi_replace_file_content"
            | "Bash"
            | "bash"
            | "RunCommand"
            | "run_command"
            | "terminal"
            | "execute_command"
    )
}

fn extract_file_path(args: &Value) -> Option<String> {
    let keys = ["file_path", "path", "file", "AbsolutePath", "TargetFile"];
    for key in keys {
        if let Some(val) = args.get(key) {
            if let Some(s) = val.as_str() {
                return Some(s.to_string());
            }
        }
    }
    None
}
