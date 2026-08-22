#[cfg(test)]
mod tests {
    use crate::config::{Config, GlobalSettings, PreferencesConfig};
    use crate::council::CouncilPipelineConfig;
    use crate::process_manager::*;

    #[test]
    fn test_port_state_free() {
        // Port 9999 should be free (unless something is running on it)
        let state = ProcessManager::check_port(9999);
        assert_eq!(state, PortState::Free);
    }

    #[test]
    fn test_process_error_variants() {
        let err = ProcessError::AnotherModelRunning;
        assert!(err.to_string().contains("another model"));

        let err = ProcessError::NotRunning("m1".to_string());
        assert!(err.to_string().contains("m1"));

        let err = ProcessError::PortOccupiedByUnknownProcess {
            pid: 1234,
            port: 8081,
        };
        assert!(err.to_string().contains("8081"));
        assert!(err.to_string().contains("1234"));
    }

    #[test]
    fn test_process_manager_multi_model_state() {
        // Verify the ProcessManager struct supports multiple running models.
        let _tmp = tempfile::tempdir().unwrap();
        let config = Config {
            schema_version: 1,
            models: vec![],
            global: GlobalSettings::default(),
            preferences: PreferencesConfig {
                auto_follow_logs: true,
                enable_notifications: true,
                notify_on_switch: true,
                autostart_on_login: false,
                max_concurrent_models: 4,
                checkpoint_summarizer_model: None,
                enable_checkpointing: true,
                enable_council: true,
            },
            council: CouncilPipelineConfig::default(),
        };
        let pm = ProcessManager::new(config);
        assert_eq!(pm.running_count(), 0);
        assert!(pm.get_primary_model().is_none());
        assert!(pm.get_primary_model_id().is_none());
    }
}
