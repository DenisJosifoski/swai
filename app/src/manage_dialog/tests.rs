#[cfg(test)]
mod tests {
    use super::super::sync_port::*;

    use std::io::Write;

    #[test]
    fn test_sync_port_preserves_assignment_syntax() {
        // Create a temp script with the expected PORT assignment syntax
        let tmp = tempfile::tempdir().unwrap();
        let script_path = tmp.path().join("test_sync_port.sh");
        let mut f = std::fs::File::create(&script_path).unwrap();
        writeln!(f, "#!/usr/bin/env bash").unwrap();
        writeln!(f, "set -euo pipefail").unwrap();
        writeln!(f, "").unwrap();
        writeln!(f, "MODEL_PATH=\"${{MODEL_PATH:-/tmp/test.gguf}}\"").unwrap();
        writeln!(f, "PORT=\"${{PORT:-8096}}\"").unwrap();
        writeln!(f, "PARALLEL_SLOTS=\"${{PARALLEL_SLOTS:-1}}\"").unwrap();
        writeln!(f, "").unwrap();
        writeln!(f, "exec llama-server \\").unwrap();
        writeln!(f, "  --model \"$MODEL_PATH\" \\").unwrap();
        writeln!(f, "  --port \"$PORT\" \\").unwrap();
        writeln!(f, "  --host 127.0.0.1").unwrap();
        drop(f);

        // Sync port to 9999
        sync_port_in_script(&script_path, 9999).unwrap();

        let content = std::fs::read_to_string(&script_path).unwrap();

        // Verify the PORT assignment line is preserved with correct syntax
        assert!(
            content.contains("PORT=\"${PORT:-9999}\""),
            "Expected PORT assignment with shell expansion syntax, got:\n{}",
            content
        );

        // Verify --port flag is also updated
        assert!(
            content.contains("--port \"$PORT\""),
            "Expected --port flag to reference $PORT variable"
        );
    }

    #[test]
    fn test_sync_port_unquoted_value() {
        let tmp = tempfile::tempdir().unwrap();
        let script_path = tmp.path().join("test_sync_port_unquoted.sh");
        let mut f = std::fs::File::create(&script_path).unwrap();
        writeln!(f, "PORT=8096").unwrap();
        writeln!(f, "--port 8096").unwrap();
        drop(f);

        sync_port_in_script(&script_path, 4567).unwrap();

        let content = std::fs::read_to_string(&script_path).unwrap();
        assert!(
            content.contains("PORT=4567"),
            "Unquoted PORT assignment not updated: {}",
            content
        );
        assert!(
            content.contains("--port 4567"),
            "Unquoted --port flag not updated: {}",
            content
        );
    }

    #[test]
    fn test_sync_port_export_syntax() {
        let tmp = tempfile::tempdir().unwrap();
        let script_path = tmp.path().join("test_sync_port_export.sh");
        let mut f = std::fs::File::create(&script_path).unwrap();
        writeln!(f, "export PORT=3000").unwrap();
        drop(f);

        sync_port_in_script(&script_path, 7777).unwrap();

        let content = std::fs::read_to_string(&script_path).unwrap();
        assert!(
            content.contains("export PORT=7777"),
            "Exported PORT assignment not updated: {}",
            content
        );
    }

    #[test]
    fn test_sync_port_no_match_unchanged() {
        let tmp = tempfile::tempdir().unwrap();
        let script_path = tmp.path().join("test_sync_port_no_match.sh");
        let original = "#!/bin/bash\necho hello\n";
        std::fs::write(&script_path, original).unwrap();

        sync_port_in_script(&script_path, 9999).unwrap();

        let content = std::fs::read_to_string(&script_path).unwrap();
        // .lines() strips trailing newlines, so compare without the final \n
        assert_eq!(
            content.trim_end_matches('\n'),
            original.trim_end_matches('\n'),
            "Script without port assignments should be unchanged"
        );
    }
}
