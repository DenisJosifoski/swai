use std::path::PathBuf;

/// Scans a script text for common port patterns and returns the last valid port.
pub fn detect_port_from_script(text: &str) -> Option<u16> {
    let mut last_detected: Option<u16> = None;

    for line in text.lines() {
        // Strip inline comments (anything after '#')
        let code_part = match line.find('#') {
            Some(idx) => &line[..idx],
            None => line,
        };

        let trimmed = code_part.trim();
        if trimmed.is_empty() {
            continue;
        }

        // Pattern 1: PORT=... or export PORT=...
        for prefix in ["PORT=", "export PORT="] {
            if let Some(rest) = trimmed.strip_prefix(prefix) {
                if let Some(extracted) = extract_port_number(rest) {
                    last_detected = Some(extracted);
                }
            }
        }

        // Pattern 2: --port N or --port=N
        if let Some(idx) = trimmed.find("--port") {
            let after = &trimmed[idx + "--port".len()..];
            let after = after.trim_start();
            let after = after.strip_prefix('=').unwrap_or(after);
            if let Some(extracted) = extract_port_number(after) {
                last_detected = Some(extracted);
            }
        }

        // Pattern 3: -p N or -p=N (word boundary: preceded by space or start of line)
        let bytes = trimmed.as_bytes();
        let mut i = 0;
        while i < bytes.len() {
            if bytes[i] == b'-' && i + 1 < bytes.len() && bytes[i + 1] == b'p' {
                let prev_ok = i == 0 || bytes[i - 1].is_ascii_whitespace();
                let next_ok = i + 2 == bytes.len()
                    || bytes[i + 2].is_ascii_whitespace()
                    || bytes[i + 2] == b'=';
                if prev_ok && next_ok {
                    let after = &trimmed[i + 2..];
                    let after = after.trim_start();
                    let after = after.strip_prefix('=').unwrap_or(after);
                    if let Some(extracted) = extract_port_number(after) {
                        last_detected = Some(extracted);
                    }
                }
            }
            i += 1;
        }
    }

    last_detected
}

/// Helper: extract a leading sequence of digits from a string and parse as u16.
pub fn extract_port_number(s: &str) -> Option<u16> {
    let s = s.trim();
    let s = s.trim_matches(|c| c == '"' || c == '\'');
    let digits: String = s.chars().take_while(|c| c.is_ascii_digit()).collect();
    if digits.is_empty() {
        return None;
    }
    digits.parse::<u16>().ok().filter(|&p| p > 0)
}

/// Infer a human-readable display name from a script filename.
pub fn infer_name_from_path(path: &PathBuf) -> String {
    path.file_stem()
        .and_then(|s| s.to_str())
        .map(|s| {
            let words: Vec<String> = s
                .replace(['-', '_'], " ")
                .split_whitespace()
                .filter(|w| !w.is_empty())
                .map(|word| {
                    let mut chars = word.chars();
                    match chars.next() {
                        Some(c) => {
                            c.to_uppercase().collect::<String>() + &chars.as_str().to_lowercase()
                        }
                        None => String::new(),
                    }
                })
                .collect();
            words.join(" ")
        })
        .unwrap_or_else(|| "New Model".to_string())
}

/// Infer a model ID slug from a script filename.
pub fn infer_id_from_path(path: &PathBuf) -> String {
    path.file_stem()
        .and_then(|s| s.to_str())
        .map(|s| s.replace([' ', '_'], "-").to_lowercase())
        .unwrap_or_else(|| "new-model".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn test_detect_port_double_dash() {
        assert_eq!(detect_port_from_script("./llama --port 8080"), Some(8080));
    }

    #[test]
    fn test_detect_port_equals() {
        assert_eq!(detect_port_from_script("./llama --port=8080"), Some(8080));
    }

    #[test]
    fn test_detect_port_short() {
        assert_eq!(detect_port_from_script("./llama -p 8080"), Some(8080));
    }

    #[test]
    fn test_detect_port_comment_skipped() {
        let script = "# --port 9999\n./llama -p 8080";
        assert_eq!(detect_port_from_script(script), Some(8080));
    }

    #[test]
    fn test_detect_port_no_match() {
        assert_eq!(detect_port_from_script("./llama --ctx-size 4096"), None);
    }

    #[test]
    fn test_detect_port_small_number_ignored() {
        assert_eq!(detect_port_from_script("./llama --port 0"), None);
    }

    #[test]
    fn test_infer_name_from_path() {
        let path = PathBuf::from("/models/llama-3-8b-instruct.sh");
        assert_eq!(infer_name_from_path(&path), "Llama 3 8b Instruct");
    }

    #[test]
    fn test_infer_id_from_path() {
        let path = PathBuf::from("/models/Llama 3 8B.sh");
        assert_eq!(infer_id_from_path(&path), "llama-3-8b");
    }

    #[test]
    fn test_infer_name_root_path() {
        let path = PathBuf::from("model.sh");
        assert_eq!(infer_name_from_path(&path), "Model");
    }
}
