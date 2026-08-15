#[cfg(test)]
mod tests {
use std::os::unix::net::UnixListener;
    use crate::config::Config;
    use crate::ipc::*;

    use std::io::{BufRead, BufReader};
    use std::os::unix::net::UnixStream;
    use tempfile::TempDir;

    // Helper: read a line from a UnixStream using BufReader.
    fn read_response(client: &mut UnixStream) -> String {
        let mut reader = BufReader::new(client);
        let mut buf = String::new();
        reader.read_line(&mut buf).unwrap();
        buf
    }

    // -- Serialization tests ------------------------------------------------

    #[test]
    fn test_action_request_status_serialization() {
        let req = ActionRequest {
            action: "status".to_string(),
            data: None,
        };
        let json = serde_json::to_string(&req).unwrap();
        assert!(json.contains("\"action\":\"status\""));
        assert!(!json.contains("\"data\"")); // data is skipped when None
    }

    #[test]
    fn test_action_request_start_serialization() {
        let req = ActionRequest {
            action: "start".to_string(),
            data: Some(serde_json::json!({"model_id": "llama-3"})),
        };
        let json = serde_json::to_string(&req).unwrap();
        assert!(json.contains("\"action\":\"start\""));
        assert!(json.contains("\"model_id\":\"llama-3\""));
    }

    #[test]
    fn test_action_request_deserialization() {
        let json = r#"{"action":"stop"}"#;
        let req: ActionRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.action, "stop");
        assert!(req.data.is_none());
    }

    #[test]
    fn test_action_request_deserialization_with_data() {
        let json = r#"{"action":"start","data":{"model_id":"qwen-2.5"}}"#;
        let req: ActionRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.action, "start");
        assert_eq!(
            req.data.unwrap()["model_id"].as_str().unwrap(),
            "qwen-2.5"
        );
    }

    #[test]
    fn test_action_response_ok_serialization() {
        let resp = ActionResponse::ok("all good", Some(serde_json::json!({"model": "llama"})));
        let json = serde_json::to_string(&resp).unwrap();
        assert!(json.contains("\"status\":\"ok\""));
        assert!(json.contains("\"message\":\"all good\""));
        assert!(json.contains("\"model\":\"llama\""));
    }

    #[test]
    fn test_action_response_error_serialization() {
        let resp = ActionResponse::error("something broke");
        let json = serde_json::to_string(&resp).unwrap();
        assert!(json.contains("\"status\":\"error\""));
        assert!(json.contains("\"message\":\"something broke\""));
        assert!(!json.contains("\"data\"")); // data is skipped when None
    }

    #[test]
    fn test_action_response_deserialization() {
        let json = r#"{"status":"ok","message":"ready","data":{"port":9080}}"#;
        let resp: ActionResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.status, "ok");
        assert_eq!(resp.message, "ready");
        assert_eq!(resp.data.unwrap()["port"].as_i64().unwrap(), 9080);
    }

    #[test]
    fn test_action_response_roundtrip() {
        let original = ActionResponse::ok(
            "switched",
            Some(serde_json::json!({
                "model_id": "test-model",
                "port": 1234,
                "proxy_port": 9080
            })),
        );
        let json = serde_json::to_string(&original).unwrap();
        let decoded: ActionResponse = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded.status, original.status);
        assert_eq!(decoded.message, original.message);
        assert_eq!(
            decoded.data.unwrap()["model_id"].as_str().unwrap(),
            "test-model"
        );
    }

    #[test]
    fn test_action_request_roundtrip() {
        let original = ActionRequest {
            action: "switch".to_string(),
            data: Some(serde_json::json!({"model_id": "phi-3"})),
        };
        let json = serde_json::to_string(&original).unwrap();
        let decoded: ActionRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded.action, original.action);
        assert_eq!(
            decoded.data.unwrap()["model_id"].as_str().unwrap(),
            "phi-3"
        );
    }

    // -- Socket handling tests ------------------------------------------------

    #[test]
    fn test_socket_path_is_in_config_dir() {
        let path = socket_path();
        assert!(path.to_string_lossy().contains(".config/swai/"));
        assert!(path.to_string_lossy().ends_with("swai.sock"));
    }

    #[test]
    fn test_cleanup_stale_socket() {
        let tmp = TempDir::new().unwrap();
        let socket = tmp.path().join("test.sock");

        // Create a dummy file at the socket path.
        std::fs::write(&socket, "stale").unwrap();
        assert!(socket.exists());

        cleanup_stale_socket(&socket);
        assert!(!socket.exists());
    }

    #[test]
    fn test_cleanup_stale_socket_nonexistent() {
        let tmp = TempDir::new().unwrap();
        let socket = tmp.path().join("nonexistent.sock");
        // Should not panic when the file doesn't exist.
        cleanup_stale_socket(&socket);
    }

    #[test]
    fn test_ipc_server_accepts_and_responds() {
        let tmp = TempDir::new().unwrap();
        let socket_path = tmp.path().join("test.sock");

        // Bind a Unix listener manually (simulating what start_ipc_server does).
        let listener = UnixListener::bind(&socket_path).unwrap();

        std::thread::spawn(move || {
            if let Ok((stream, _)) = listener.accept() {
                // Echo back a response.
                let response = ActionResponse::ok("connected", None);
                let body = serde_json::to_string(&response).unwrap();
                use std::io::Write;
                let mut writer = std::io::BufWriter::new(stream);
                writer.write_all(body.as_bytes()).unwrap();
                writer.write_all(b"\n").unwrap();
                writer.flush().unwrap();
            }
        });

        // Connect a client.
        let mut client = UnixStream::connect(&socket_path).unwrap();
        let request = ActionRequest {
            action: "status".to_string(),
            data: None,
        };
        let body = serde_json::to_string(&request).unwrap();
        use std::io::Write;
        client.write_all(body.as_bytes()).unwrap();
        client.write_all(b"\n").unwrap();
        client.flush().unwrap();

        // Read the response.
        let response_line = read_response(&mut client);
        let response: ActionResponse = serde_json::from_str(response_line.trim()).unwrap();

        assert_eq!(response.status, "ok");
        assert_eq!(response.message, "connected");
    }

    #[test]
    #[serial_test::serial]
    fn test_ipc_client_socket_not_found() {
        let old_home = std::env::var("HOME").ok();
        let req = ActionRequest {
            action: "status".to_string(),
            data: None,
        };
        // Without HOME set, socket_path will be relative — and the file won't exist.
        std::env::remove_var("HOME");
        let result = ipc_send(&req);
        if let Some(ref h) = old_home {
            std::env::set_var("HOME", h);
        }
        assert!(result.is_err());
        match result.unwrap_err() {
            IpcClientError::SocketNotFound => {} // expected
            other => panic!("expected SocketNotFound, got {:?}", other),
        }
    }

    #[test]
    fn test_ipc_state_creation() {
        // Create a minimal config for testing.
        let tmp = tempfile::tempdir().unwrap();
        let script_path = tmp.path().join("dummy.sh");
        std::fs::write(&script_path, "#!/bin/sh\necho ok\n").ok();
        #[cfg(unix)]
        std::process::Command::new("chmod")
            .arg("+x")
            .arg(&script_path)
            .status()
            .ok();

        let config_content = format!(
            "schema_version = 1\n\n[[models]]\nid = \"test\"\nname = \"Test\"\nscript_path = \"{}\"\nport = 9999\nhealth_timeout_sec = 5\n",
            script_path.display()
        );
        let config: Config = toml::from_str(&config_content).unwrap();
        let state = IpcState::new(config);

        assert_eq!(state.config.models.len(), 1);
        assert_eq!(state.config.models[0].id, "test");
    }

    #[test]
    fn test_error_display_messages() {
        assert!(format!("{}", IpcClientError::SocketNotFound).contains("SWAI"));
        assert!(format!("{}", IpcClientError::ConnectionRefused).contains("SWAI"));
        assert!(format!(
            "{}",
            IpcClientError::ServerError("test error".to_string())
        )
        .contains("test error"));
    }

    #[test]
    fn test_invalid_json_request_handling() {
        let tmp = TempDir::new().unwrap();
        let socket_path = tmp.path().join("test.sock");

        let listener = UnixListener::bind(&socket_path).unwrap();

        std::thread::spawn(move || {
            if let Ok((mut stream, _)) = listener.accept() {
                // Read the request.
                use std::io::Read;
                let mut reader = std::io::BufReader::new(&stream);
                let mut buf = Vec::new();
                let _ = reader.read_to_end(&mut buf);

                // Try to parse invalid JSON — should fail.
                let result: Result<ActionRequest, _> = serde_json::from_slice(&buf);
                assert!(result.is_err());

                // Send an error response.
                let response = ActionResponse::error("invalid request");
                use std::io::Write;
                let body = serde_json::to_string(&response).unwrap();
                stream.write_all(body.as_bytes()).unwrap();
                stream.write_all(b"\n").unwrap();
                stream.flush().unwrap();
            }
        });

        // Client: connect and send invalid JSON.
        let mut client = UnixStream::connect(&socket_path).unwrap();
        use std::io::Write;
        client.write_all(b"not valid json{{{").unwrap();
        client.write_all(b"\n").unwrap();
        client.flush().unwrap();

        // Close the write half to signal EOF to the server.
        drop(client);
    }

    #[test]
    fn test_response_data_serialization_skips_none() {
        // Verify that `data: None` is omitted from JSON output.
        let resp = ActionResponse::ok("no data", None);
        let json = serde_json::to_string(&resp).unwrap();
        assert!(!json.contains("\"data\""));

        // And verify it round-trips correctly (data becomes None after deserialization).
        let decoded: ActionResponse = serde_json::from_str(&json).unwrap();
        assert!(decoded.data.is_none());
    }

    #[test]
    fn test_response_data_serialization_includes_some() {
        // Verify that `data: Some(...)` is included in JSON output.
        let resp = ActionResponse::ok("has data", Some(serde_json::json!({"x": 1})));
        let json = serde_json::to_string(&resp).unwrap();
        assert!(json.contains("\"data\""));
        assert!(json.contains("\"x\":1"));
    }

    #[test]
    fn test_config_dir_fallback() {
        // Verify that config_dir returns a path containing ".config/swai".
        // We don't remove HOME here to avoid affecting other tests.
        let dir = config_dir();
        assert!(dir.to_string_lossy().contains(".config/swai"));
    }

    #[test]
    fn test_handle_request_sync_invalid_json() {
        let tmp = TempDir::new().unwrap();
        let socket_path = tmp.path().join("test.sock");

        // Create a minimal IpcState for the handler.
        let script_path = tmp.path().join("dummy.sh");
        std::fs::write(&script_path, "#!/bin/sh\necho ok\n").ok();
        #[cfg(unix)]
        std::process::Command::new("chmod")
            .arg("+x")
            .arg(&script_path)
            .status()
            .ok();
        let config_content = format!(
            "schema_version = 1\n\n[[models]]\nid = \"test\"\nname = \"Test\"\nscript_path = \"{}\"\nport = 9999\nhealth_timeout_sec = 5\n",
            script_path.display()
        );
        let config: Config = toml::from_str(&config_content).unwrap();
        let mut state = IpcState::new(config);

        let listener = UnixListener::bind(&socket_path).unwrap();

        // Server thread: accept, read invalid JSON, send error response.
        std::thread::spawn(move || {
            if let Ok((stream, _)) = listener.accept() {
                let _ = handle_request_sync(stream, &mut state);
            }
        });

        // Client: connect and send invalid JSON.
        let mut client = UnixStream::connect(&socket_path).unwrap();
        use std::io::Write;
        client.write_all(b"not valid json{{{").unwrap();
        client.write_all(b"\n").unwrap();
        client.flush().unwrap();

        // Close the write half to signal EOF to the server using safe Rust std::net::Shutdown.
        client.shutdown(std::net::Shutdown::Write).unwrap();

        // Read the error response.
        let response_line = read_response(&mut client);
        let response: ActionResponse = serde_json::from_str(response_line.trim()).unwrap();

        assert_eq!(response.status, "error");
        assert!(response.message.contains("invalid JSON"));
    }
}
