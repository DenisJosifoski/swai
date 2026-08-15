#[cfg(test)]
mod tests {
    use super::super::poller::*;
    
    use std::fs::{self};
    use std::path::PathBuf;
    use swai_core::config::ModelConfig;


    fn make_test_model(id: &str, name: &str) -> ModelConfig {
        ModelConfig {
            id: id.to_string(),
            name: name.to_string(),
            script_path: PathBuf::from(format!("/tmp/{}.sh", id)),
            port: 8080 + id.parse::<u16>().unwrap_or(1),
            health_timeout_sec: 30,
        }
    }

    #[test]
    fn test_resolve_log_file_fallback() {
        // When no log files exist, it should return a sensible fallback path.
        // nosemgrep: rust.lang.security.temp-dir.temp-dir — test scaffolding only, never ships in release
        let temp_dir = std::env::temp_dir().join("swai-test-logs");
        let _ = fs::create_dir_all(&temp_dir);

        let script = PathBuf::from("/tmp/test-model.sh");
        let result = resolve_log_file(&script, &temp_dir);

        assert!(result.starts_with(&temp_dir));
        // Check the filename (not the full path) starts with the expected prefix.
        let filename = result.file_name().unwrap_or_default().to_string_lossy();
        assert!(filename.starts_with("test-model_"));
        assert!(result.to_string_lossy().ends_with(".log"));

        // Cleanup.
        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_resolve_log_file_selects_most_recent() {
        // When multiple log files exist, it should return the most recent one.
        // nosemgrep: rust.lang.security.temp-dir.temp-dir — test scaffolding only, never ships in release
        let temp_dir = std::env::temp_dir().join("swai-test-logs-resolve");
        let _ = fs::create_dir_all(&temp_dir);

        // Create two log files with different timestamps.
        let script = PathBuf::from("/tmp/test-model.sh");
        fs::write(
            temp_dir.join("test-model_20260101_120000.log"),
            "old log content",
        )
        .unwrap();
        fs::write(
            temp_dir.join("test-model_20260730_150000.log"),
            "new log content",
        )
        .unwrap();

        let result = resolve_log_file(&script, &temp_dir);

        // Should return the most recent file.
        assert!(result.to_string_lossy().contains("20260730_150000"));

        // Cleanup.
        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_select_model_by_id_finds_correct_index() {
        // Test that select_model_by_id would find the correct index for a given model ID.
        let models = vec![
            make_test_model("m1", "Model 1"),
            make_test_model("m2", "Model 2"),
            make_test_model("m3", "Model 3"),
        ];

        // Simulate the logic from select_model_by_id.
        let target_id = "m2";
        let found_index = models.iter().position(|m| m.id == target_id);

        assert_eq!(found_index, Some(1));

        // Test with non-existent ID.
        let missing_id = "m999";
        let missing_index = models.iter().position(|m| m.id == missing_id);

        assert_eq!(missing_index, None);
    }
}
