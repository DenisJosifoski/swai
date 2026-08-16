struct PortRegexes {
    /// Matches: PORT="${PORT:-N}", PORT=N, PORT="N", PORT='N', export PORT=N
    var_assign: std::sync::OnceLock<regex::Regex>,
    /// Matches: --port N or --port=N
    long_flag: std::sync::OnceLock<regex::Regex>,
    /// Matches: -p N (only when not preceded by another `-`, i.e. not `--something`)
    short_flag: std::sync::OnceLock<regex::Regex>,
}

impl Default for PortRegexes {
    fn default() -> Self {
        Self {
            var_assign: std::sync::OnceLock::new(),
            long_flag: std::sync::OnceLock::new(),
            short_flag: std::sync::OnceLock::new(),
        }
    }
}

impl PortRegexes {
    fn var_assign(&self) -> &regex::Regex {
        self.var_assign.get_or_init(|| {
            // Use [ \t] instead of \s to avoid matching newlines — since we
            // process line-by-line, a leading space/tab is the only valid prefix.
            regex::Regex::new(r#"(^|[ \t])(export[ \t]+)?PORT=("([^"]*)"|'([^']*)'|(\d+))"#)
                .expect("port var_assign regex must compile")
        })
    }

    fn long_flag(&self) -> &regex::Regex {
        self.long_flag.get_or_init(|| {
            // Match `--port=N` or `--port N`. The separator is `=` or whitespace.
            regex::Regex::new(r"--port[= \t]+(\d+)").expect("port long_flag regex must compile")
        })
    }

    fn short_flag(&self) -> &regex::Regex {
        self.short_flag.get_or_init(|| {
            // Match standalone `-p N` or `-p=N` (preceded by whitespace/start of line).
            // Prevents false positives on long flags like `--repeat-penalty 1.05`.
            regex::Regex::new(r"(^|[ \t])(-p[ \t=]+(\d+))")
                .expect("port short_flag regex must compile")
        })
    }
}

/// Synchronize the port value inside a `.sh` launch script.
///
/// Reads the current text of `port_entry` at click time (not at dialog
/// construction), then rewrites any matching port assignments in the script
/// file. The function is idempotent — running it twice with the same port
/// produces identical output.
///
/// Supported patterns:
/// - `PORT=N`, `PORT="N"`, `PORT='N'`, `export PORT=N`
/// - `--port N`, `--port=N`
/// - `-p N` (only when not preceded by `-`, to avoid matching `--something`)
///
/// Uses explicit `${1}` / `${2}` capture-group replacement syntax to avoid the
/// classic `$18091` bug where `$18` would be interpreted as capture group 18
/// instead of `$1` followed by literal `8`.
pub fn sync_port_in_script(script_path: &std::path::PathBuf, new_port: u16) -> Result<(), String> {
    let new_port_str = new_port.to_string();

    // Read current script content.
    let content = std::fs::read_to_string(script_path)
        .map_err(|e| format!("Failed to read script {}: {}", script_path.display(), e))?;

    let re = PortRegexes::default();

    // We apply replacements line-by-line so that `-p` on one line doesn't
    // accidentally match `--port` on another. This also preserves blank lines
    // and comments untouched.
    let mut updated_lines: Vec<String> = Vec::with_capacity(content.lines().count());

    for line in content.lines() {
        let trimmed = line.trim();

        // Skip comment-only lines entirely — never rewrite a comment.
        if trimmed.starts_with('#') {
            updated_lines.push(line.to_string());
            continue;
        }

        let mut current = line.to_string();

        // 1. Variable assignments: PORT=N, PORT="N", export PORT=N, etc.
        current = replace_first(&current, re.var_assign(), &new_port_str);

        // 2. Long flag: --port N or --port=N
        current = replace_first(&current, re.long_flag(), &new_port_str);

        // 3. Short flag: -p N (only when not preceded by `-`)
        current = replace_first(&current, re.short_flag(), &new_port_str);

        updated_lines.push(current);
    }

    let new_content = updated_lines.join("\n");

    // Only write if something actually changed — avoids unnecessary mtime bumps.
    if new_content != content {
        std::fs::write(script_path, &new_content)
            .map_err(|e| format!("Failed to write script {}: {}", script_path.display(), e))?;
        tracing::info!(
            "Synced port {} into script {}",
            new_port_str,
            script_path.display()
        );
    } else {
        tracing::debug!(
            "No port assignments found in {}; skipping rewrite.",
            script_path.display()
        );
    }

    Ok(())
}

/// Apply a single regex replacement to `input`, replacing only the capture
/// group that holds the port number with `replacement`. All other parts of
/// the match (prefix, surrounding text, non-port groups) are preserved
/// verbatim. Returns the original string unchanged when no match is found.
///
/// For unquoted ports (`PORT=8096`), replaces just the bare digits in group 3.
/// For quoted ports (`PORT="${PORT:-8096}"`), replaces only the bare digits
/// inside the quotes to preserve `${...}` shell syntax.
fn replace_first(input: &str, re: &regex::Regex, replacement: &str) -> String {
    let caps = match re.captures(input) {
        Some(c) => c,
        None => return input.to_string(),
    };

    // Check if group 3 captured a quoted value (starts with `"` or `'`).
    let group3 = caps.get(3);
    let is_quoted = group3
        .map(|m| m.as_str().starts_with('\"') || m.as_str().starts_with('\''))
        .unwrap_or(false);

    if is_quoted {
        // For quoted values like `"${PORT:-8096}"`, group 3 includes the quotes.
        // We need to replace only the bare digits inside, preserving `${...}` syntax.
        let digit_re = regex::Regex::new(r"\d+").unwrap();
        if let Some(g3) = group3 {
            if let Some(digit_match) = digit_re.find(g3.as_str()) {
                let g3_start = g3.start();
                let abs_start = g3_start + digit_match.start();
                let abs_end = g3_start + digit_match.end();
                let mut result = input.to_string();
                result.replace_range(abs_start..abs_end, replacement);
                return result;
            }
        }
        input.to_string()
    } else {
        // Unquoted: replace the group that contains just the bare digits.
        let port_group = find_port_capture_index(re, input);
        if port_group < caps.len() {
            if let Some(m) = caps.get(port_group) {
                let mut result = input.to_string();
                result.replace_range(m.range(), replacement);
                return result;
            }
        }
        input.to_string()
    }
}

/// Determine which capture-group index holds the port number for a given regex
/// and input line. The function inspects each group's matched text to find the
/// one that parses as a number — that's the port group.
///
/// Strategy: prefer groups whose entire match is digits only (most likely the
/// bare port number), then fall back to any group that parses as u16.
fn find_port_capture_index(re: &regex::Regex, line: &str) -> usize {
    let caps = re
        .captures(line)
        .expect("replace_first called without a match");

    // First pass: find groups whose entire text is digits (bare port numbers).
    // This handles long_flag group 1, short_flag group 3, var_assign group 6.
    for i in 0..caps.len() {
        if let Some(m) = caps.get(i) {
            if m.as_str().chars().all(|c| c.is_ascii_digit()) && !m.as_str().is_empty() {
                if let Ok(port) = m.as_str().parse::<u16>() {
                    if port > 0 {
                        return i;
                    }
                }
            }
        }
    }

    // Second pass: any group that parses as a valid port number.
    for i in 0..caps.len() {
        if let Some(m) = caps.get(i) {
            if let Ok(port) = m.as_str().parse::<u16>() {
                if port > 0 {
                    return i;
                }
            }
        }
    }

    // Fallback: no numeric group found — shouldn't happen if regex matched.
    0
}
