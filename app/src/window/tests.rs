#[cfg(test)]
mod tests {
    use super::super::dialogs::append_model_to_config_at;
    use crate::import_wizard::ImportedModel;
    use std::io::Write;
    use swai_core::config::Config;

    /// Verify that adopting an unmanaged server writes the model into config.toml.
    #[test]
    fn test_adopt_model_registers_into_config() {
        let tmp = tempfile::tempdir().unwrap();
        let script_path = tmp.path().join("adopt-test.sh");
        let mut f = std::fs::File::create(&script_path).unwrap();
        writeln!(f, "#!/bin/sh\nllama-server --port 8096").unwrap();
        drop(f);

        // Create a minimal config.toml with no models.
        let config_path = tmp.path().join("config.toml");
        std::fs::write(&config_path, "schema_version = 1\n").unwrap();

        let model = ImportedModel {
            id: "adopted-model".to_string(),
            name: "Adopted Model".to_string(),
            script_path: script_path.clone(),
            port: 8096,
            health_timeout_sec: 30,
        };

        let result = append_model_to_config_at(&config_path, &model);
        assert!(
            result.is_ok(),
            "Adoption should succeed: {:?}",
            result.err()
        );

        // Reload and verify the model was registered.
        let content = std::fs::read_to_string(&config_path).unwrap();
        let config: Config = toml::from_str(&content).unwrap();
        assert_eq!(config.models.len(), 1);
        assert_eq!(config.models[0].id, "adopted-model");
        assert_eq!(config.models[0].name, "Adopted Model");
        assert_eq!(config.models[0].port, 8096);
        assert_eq!(config.models[0].script_path, script_path);
    }

    /// Verify that adopting a model with a duplicate port is rejected.
    #[test]
    fn test_adopt_model_duplicate_port_rejected() {
        let tmp = tempfile::tempdir().unwrap();
        let script_path = tmp.path().join("adopt-dup.sh");
        let mut f = std::fs::File::create(&script_path).unwrap();
        writeln!(f, "#!/bin/sh\nllama-server --port 9090").unwrap();
        drop(f);

        // Create a config.toml with an existing model on port 9090.
        let config_path = tmp.path().join("config.toml");
        let existing_script = tmp.path().join("existing.sh");
        std::fs::write(&existing_script, "#!/bin/sh\necho existing").unwrap();

        let initial_config = format!(
            "schema_version = 1\n\n[[models]]\nid = \"existing-model\"\nname = \"Existing Model\"\nscript_path = \"{}\"\nport = 9090\nhealth_timeout_sec = 30\n",
            existing_script.display()
        );
        std::fs::write(&config_path, initial_config).unwrap();

        // Attempt to adopt a model on the same port (9090) as the existing model.
        let model = ImportedModel {
            id: "conflicting-model".to_string(),
            name: "Conflicting Model".to_string(),
            script_path: script_path.clone(),
            port: 9090,
            health_timeout_sec: 30,
        };

        let result = append_model_to_config_at(&config_path, &model);
        assert!(
            result.is_err(),
            "Duplicate port adoption should have been rejected"
        );

        // Verify config was not modified.
        let content = std::fs::read_to_string(&config_path).unwrap();
        let config: Config = toml::from_str(&content).unwrap();
        assert_eq!(config.models.len(), 1);
    }

    /// Verify that adopting a model with a missing script is rejected by validation.
    #[test]
    fn test_adopt_model_missing_script_rejected() {
        let tmp = tempfile::tempdir().unwrap();
        let config_path = tmp.path().join("config.toml");
        std::fs::write(&config_path, "schema_version = 1\n").unwrap();

        let non_existent_script = tmp.path().join("does-not-exist.sh");

        let model = ImportedModel {
            id: "missing-script-model".to_string(),
            name: "Missing Script Model".to_string(),
            script_path: non_existent_script,
            port: 8097,
            health_timeout_sec: 30,
        };

        let result = append_model_to_config_at(&config_path, &model);
        assert!(result.is_err(), "Missing script should fail validation");
    }
}
