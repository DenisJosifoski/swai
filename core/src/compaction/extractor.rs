use serde_json::Value;

pub fn extract_action_lines(messages: &[Value]) -> Vec<String> {
    let mut lines = Vec::new();

    for msg in messages {
        let role = msg
            .get("role")
            .and_then(|r| r.as_str())
            .unwrap_or("unknown");
        let content = msg.get("content");

        match (role, content) {
            ("user", Some(Value::String(text))) if !text.is_empty() => {
                // Truncate long user messages to 200 chars.
                let truncated: String = text.chars().take(200).collect();
                lines.push(truncated);
            }

            ("user", Some(Value::Array(blocks))) => {
                for block in blocks {
                    if block.get("type").and_then(|t| t.as_str()) == Some("tool_result") {
                        // Check for isError on the block itself.
                        if let Some(is_error) = block.get("isError").and_then(|v| v.as_bool()) {
                            if is_error {
                                lines.push("Result: failed".to_string());
                            } else {
                                lines.push("Result: passed".to_string());
                            }
                            continue;
                        }

                        // Check for isError inside content blocks.
                        if let Some(content) = block.get("content") {
                            if let Some(arr) = content.as_array() {
                                let mut handled = false;
                                for inner in arr {
                                    if let Some(is_error) =
                                        inner.get("isError").and_then(|v| v.as_bool())
                                    {
                                        if is_error {
                                            // Extract first line of error text.
                                            if let Some(err_text) =
                                                inner.get("text").and_then(|t| t.as_str())
                                            {
                                                let first_line: String =
                                                    err_text.chars().take(100).collect();
                                                lines.push(format!(
                                                    "Result: failed: {}",
                                                    first_line
                                                ));
                                            } else {
                                                lines.push("Result: failed".to_string());
                                            }
                                        } else {
                                            lines.push("Result: passed".to_string());
                                        }
                                        handled = true;
                                        break;
                                    }
                                }
                                if handled {
                                    continue;
                                }
                            }
                        }

                        // No isError field found → assume success.
                        lines.push("Result: passed".to_string());
                    }
                }
            }

            ("assistant", Some(Value::Array(blocks))) => {
                let mut has_tool_use = false;
                for block in blocks {
                    if block.get("type").and_then(|t| t.as_str()) == Some("tool_use") {
                        has_tool_use = true;
                        if let Some(name) = block.get("name").and_then(|n| n.as_str()) {
                            match name {
                                "Read" | "read" | "ViewFile" | "view_file" | "read_file"
                                | "ReadFile" => {
                                    if let Some(input) = block.get("input") {
                                        if let Some(path) = input
                                            .get("file_path")
                                            .or_else(|| input.get("path"))
                                            .or_else(|| input.get("file"))
                                            .or_else(|| input.get("AbsolutePath"))
                                            .and_then(|p| p.as_str())
                                        {
                                            lines.push(format!("Read {}", path));
                                        } else {
                                            lines.push("Read <unknown file>".to_string());
                                        }
                                    } else {
                                        lines.push("Read <unknown file>".to_string());
                                    }
                                }
                                "Edit"
                                | "edit"
                                | "ReplaceFileContent"
                                | "replace_file_content"
                                | "multi_replace_file_content" => {
                                    if let Some(input) = block.get("input") {
                                        if let Some(path) = input
                                            .get("file_path")
                                            .or_else(|| input.get("path"))
                                            .or_else(|| input.get("TargetFile"))
                                            .or_else(|| input.get("file"))
                                            .and_then(|p| p.as_str())
                                        {
                                            lines.push(format!("Edited {}", path));
                                        } else {
                                            lines.push("Edited <unknown file>".to_string());
                                        }
                                    } else {
                                        lines.push("Edited <unknown file>".to_string());
                                    }
                                }
                                "Write" | "write" | "write_to_file" | "WriteToFile" => {
                                    if let Some(input) = block.get("input") {
                                        if let Some(path) = input
                                            .get("file_path")
                                            .or_else(|| input.get("path"))
                                            .or_else(|| input.get("TargetFile"))
                                            .or_else(|| input.get("file"))
                                            .and_then(|p| p.as_str())
                                        {
                                            lines.push(format!("Wrote {}", path));
                                        } else {
                                            lines.push("Wrote <unknown file>".to_string());
                                        }
                                    } else {
                                        lines.push("Wrote <unknown file>".to_string());
                                    }
                                }
                                "Bash" | "bash" | "RunCommand" | "run_command" | "terminal"
                                | "execute_command" => {
                                    if let Some(input) = block.get("input") {
                                        if let Some(cmd) = input
                                            .get("command")
                                            .or_else(|| input.get("cmd"))
                                            .or_else(|| input.get("CommandLine"))
                                            .and_then(|c| c.as_str())
                                        {
                                            let truncated: String = cmd.chars().take(100).collect();
                                            lines.push(format!("Ran command: {}", truncated));
                                        } else {
                                            lines.push("Ran command: <unknown>".to_string());
                                        }
                                    } else {
                                        lines.push("Ran command: <unknown>".to_string());
                                    }
                                }
                                "Grep" | "grep" | "grep_search" | "Glob" | "glob" => {
                                    if let Some(input) = block.get("input") {
                                        if let Some(pat) = input
                                            .get("pattern")
                                            .or_else(|| input.get("query"))
                                            .or_else(|| input.get("Query"))
                                            .and_then(|p| p.as_str())
                                        {
                                            let truncated: String = pat.chars().take(60).collect();
                                            lines.push(format!("Searched: {}", truncated));
                                        } else {
                                            lines.push("Searched files".to_string());
                                        }
                                    } else {
                                        lines.push("Searched files".to_string());
                                    }
                                }
                                _ => {
                                    // Unknown tool — record generically.
                                    let truncated: String = name.chars().take(50).collect();
                                    lines.push(format!("Used tool: {}", truncated));
                                }
                            }
                        }
                    } else if block.get("type").and_then(|t| t.as_str()) == Some("text") {
                        // Skip generic assistant chatter; only track meaningful tool actions
                    }
                }
                // If the assistant had tool_use but no text, add a generic line.
                if has_tool_use && lines.is_empty() {
                    lines.push("Used tools".to_string());
                }
            }

            _ => {
                // Unrecognized pattern — skip silently.
            }
        }
    }

    lines
}

/// Deterministic fallback synthesizer for dropped message slices.
///
/// Provides a zero-inference baseline that compiles bullet points even if
/// LLM summarization is skipped or fails. This ensures compaction always
/// produces useful output, regardless of whether a summarization LLM is
/// available.
///
/// Group messages into atomic eviction units: either a single non-tool turn,
/// or an `(assistant tool_use, user tool_result)` pair that must always be
/// dropped or kept together to preserve structural protocol validity.
pub fn build_eviction_units(messages: &[Value]) -> Vec<(usize, usize)> {
    let mut units = Vec::new();
    let mut i = 0;
    while i < messages.len() {
        let has_tool_use = messages[i]
            .get("content")
            .and_then(|c| c.as_array())
            .map(|arr| {
                arr.iter()
                    .any(|b| b.get("type").and_then(|t| t.as_str()) == Some("tool_use"))
            })
            .unwrap_or(false);

        if has_tool_use && i + 1 < messages.len() {
            let next_has_tool_result = messages[i + 1]
                .get("content")
                .and_then(|c| c.as_array())
                .map(|arr| {
                    arr.iter()
                        .any(|b| b.get("type").and_then(|t| t.as_str()) == Some("tool_result"))
                })
                .unwrap_or(false);
            if next_has_tool_result {
                units.push((i, i + 1));
                i += 2;
                continue;
            }
        }
        units.push((i, i));
        i += 1;
    }
    units
}

/// Deterministic fallback synthesizer for dropped message slices.
pub fn serialize_dropped_slice(dropped: &[Value]) -> Vec<String> {
    extract_action_lines(dropped)
}
