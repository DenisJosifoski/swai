//! Message compaction and checkpoint injection for Anthropic Messages API.
//!
//! When conversations grow beyond the model's context window, older messages
//! must be evicted (compacted). This module:
//!
//! 1. Extracts plain-text action summaries from dropped message slices
//!    (e.g., "Read src/lib.rs", "Edited main.rs", "Ran command: cargo build").
//! 2. Provides a deterministic fallback synthesizer (`serialize_dropped_slice`)
//!    that compiles bullet points even if LLM summarization is skipped.
//! 3. Injects formatted `[Session checkpoint]` blocks into Anthropic Messages
//!    API payloads immediately after the system prompt, so the model retains
//!    awareness of earlier work without re-sending full history.
//!
//! ## Data flow
//!
//! ```text
//! messages → compact_messages_anthropic() → [checkpoint entries, remaining messages]
//! checkpoint → format_for_injection() → "[Session checkpoint — ...]" string
//! remaining + checkpoint → inject_checkpoint_into_payload() → modified JSON payload
//! ```

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// A single Anthropic Messages API message.
///
/// The `content` field mirrors the Anthropic API format: either a plain string
/// or an array of content blocks (text, image, tool use, etc.).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub role: String,
    #[serde(default)]
    pub content: Vec<Value>,
}

impl Message {
    /// Extract the first plain-text string from a message's content.
    ///
    /// Anthropic messages can have content as either:
    /// - A plain string: `"Hello world"`
    /// - An array of content blocks: `[{"type": "text", "text": "Hello"}]`
    pub fn first_text(&self) -> Option<String> {
        // Content is a single string value.
        if self.content.len() == 1 {
            if let Some(s) = self.content[0].as_str() {
                return Some(s.to_string());
            }
        }

        // Content is an array — find the first text block.
        for item in &self.content {
            if let Some(text) = item.get("text").and_then(|t| t.as_str()) {
                return Some(text.to_string());
            }
        }

        None
    }

    /// Check if this message contains a tool_use block.
    pub fn has_tool_use(&self) -> bool {
        self.content.iter().any(|item| {
            item.get("type").and_then(|t| t.as_str()) == Some("tool_use")
        })
    }

    /// Extract the tool name from a tool_use block if present.
    pub fn tool_use_name(&self) -> Option<String> {
        for item in &self.content {
            if item.get("type").and_then(|t| t.as_str()) == Some("tool_use") {
                return item.get("name").and_then(|n| n.as_str()).map(String::from);
            }
        }
        None
    }

    /// Check if this message is a tool_result.
    pub fn is_tool_result(&self) -> bool {
        self.role == "user" && self.content.iter().any(|item| {
            item.get("type").and_then(|t| t.as_str()) == Some("tool_result")
        })
    }

    /// Extract the first tool result status (success/failure).
    pub fn tool_result_status(&self) -> Option<String> {
        for item in &self.content {
            if item.get("type").and_then(|t| t.as_str()) == Some("tool_result") {
                // Check for explicit content with isError field.
                if let Some(content) = item.get("content") {
                    if let Some(arr) = content.as_array() {
                        for block in arr {
                            if let Some(is_error) = block.get("isError").and_then(|v| v.as_bool()) {
                                return Some(if is_error { "failed" } else { "passed" }.to_string());
                            }
                        }
                    }
                }
                // If no content block with isError, treat as success.
                return Some("passed".to_string());
            }
        }
        None
    }

    /// Extract the command string from a RunCommand tool_use block if present.
    pub fn run_command_string(&self) -> Option<String> {
        for item in &self.content {
            if item.get("type").and_then(|t| t.as_str()) == Some("tool_use") {
                if let Some(name) = item.get("name").and_then(|n| n.as_str()) {
                    if name == "RunCommand" || name == "run_command" {
                        if let Some(input) = item.get("input") {
                            return input.get("command").and_then(|c| c.as_str()).map(String::from);
                        }
                    }
                }
            }
        }
        None
    }

    /// Extract the file path from a Read/ViewFile tool_use block if present.
    pub fn read_file_path(&self) -> Option<String> {
        for item in &self.content {
            if item.get("type").and_then(|t| t.as_str()) == Some("tool_use") {
                if let Some(name) = item.get("name").and_then(|n| n.as_str()) {
                    match name {
                        "Read" | "view_file" | "ViewFile" => {
                            if let Some(input) = item.get("input") {
                                return input
                                    .get("file_path")
                                    .or_else(|| input.get("path"))
                                    .and_then(|p| p.as_str())
                                    .map(String::from);
                            }
                        }
                        _ => {}
                    }
                }
            }
        }
        None
    }

    /// Extract the file path from an Edit/ReplaceFileContent tool_use block if present.
    pub fn edit_file_path(&self) -> Option<String> {
        for item in &self.content {
            if item.get("type").and_then(|t| t.as_str()) == Some("tool_use") {
                if let Some(name) = item.get("name").and_then(|n| n.as_str()) {
                    match name {
                        "Edit" | "replace_file_content" | "ReplaceFileContent" => {
                            if let Some(input) = item.get("input") {
                                return input
                                    .get("file_path")
                                    .or_else(|| input.get("path"))
                                    .and_then(|p| p.as_str())
                                    .map(String::from);
                            }
                        }
                        _ => {}
                    }
                }
            }
        }
        None
    }
}

/// Configuration for message compaction behavior.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompactionConfig {
    /// Whether compaction is enabled.
    #[serde(default)]
    pub enabled: bool,
    /// Maximum token count before triggering compaction.
    #[serde(default = "default_max_tokens")]
    pub max_tokens: usize,
    /// Target summary length in characters per dropped slice.
    #[serde(default = "default_summary_length")]
    pub summary_length: usize,
}

fn default_max_tokens() -> usize {
    100_000
}

fn default_summary_length() -> usize {
    2_000
}

impl Default for CompactionConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            max_tokens: default_max_tokens(),
            summary_length: default_summary_length(),
        }
    }
}

