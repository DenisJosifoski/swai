use serde_json::Value;

pub fn format_messages_for_summarization(messages: &[Value]) -> String {
    let mut lines = Vec::new();

    for msg in messages {
        let role = msg
            .get("role")
            .and_then(|r| r.as_str())
            .unwrap_or("unknown");
        let content = msg.get("content");

        match (role, content) {
            ("user", Some(Value::String(text))) if !text.is_empty() => {
                lines.push(format!("[User]: {}", truncate_text(text, 200)));
            }
            ("user", Some(Value::Array(blocks))) => {
                for block in blocks {
                    if block.get("type").and_then(|t| t.as_str()) == Some("tool_result") {
                        let status = if let Some(is_error) =
                            block.get("isError").and_then(|v| v.as_bool())
                        {
                            if is_error {
                                "FAILED"
                            } else {
                                "passed"
                            }
                        } else {
                            "passed"
                        };
                        lines.push(format!("[Tool Result {}]", status));
                    }
                }
            }
            ("assistant", Some(Value::Array(blocks))) => {
                for block in blocks {
                    match block.get("type").and_then(|t| t.as_str()) {
                        Some("text") => {
                            if let Some(text) = block.get("text").and_then(|t| t.as_str()) {
                                lines.push(format!("[Assistant]: {}", truncate_text(text, 150)));
                            }
                        }
                        Some("tool_use") => {
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
                                                lines.push(format!("[Tool: Read {}]", path));
                                            } else {
                                                lines.push("[Tool: Read <unknown>]".to_string());
                                            }
                                        } else {
                                            lines.push("[Tool: Read <unknown>]".to_string());
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
                                                lines.push(format!("[Tool: Edited {}]", path));
                                            } else {
                                                lines.push("[Tool: Edited <unknown>]".to_string());
                                            }
                                        } else {
                                            lines.push("[Tool: Edited <unknown>]".to_string());
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
                                                lines.push(format!("[Tool: Wrote {}]", path));
                                            } else {
                                                lines.push("[Tool: Wrote <unknown>]".to_string());
                                            }
                                        } else {
                                            lines.push("[Tool: Wrote <unknown>]".to_string());
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
                                                lines.push(format!(
                                                    "[Tool: Ran command: {}]",
                                                    truncate_text(cmd, 100)
                                                ));
                                            } else {
                                                lines.push(
                                                    "[Tool: Ran command: <unknown>]".to_string(),
                                                );
                                            }
                                        } else {
                                            lines
                                                .push("[Tool: Ran command: <unknown>]".to_string());
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
                                                lines.push(format!(
                                                    "[Tool: Searched: {}]",
                                                    truncate_text(pat, 60)
                                                ));
                                            } else {
                                                lines.push("[Tool: Searched files]".to_string());
                                            }
                                        } else {
                                            lines.push("[Tool: Searched files]".to_string());
                                        }
                                    }
                                    _ => {
                                        lines.push(format!("[Tool: {}]", truncate_text(name, 50)));
                                    }
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }

    lines.join("\n")
}

/// Truncate a string to the given character count.
pub fn truncate_text(s: &str, max_len: usize) -> String {
    if s.len() <= max_len {
        s.to_string()
    } else {
        format!("{}…", &s[..max_len])
    }
}
