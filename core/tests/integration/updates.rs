use swai_core::updates::*;

#[test]
fn update_check_config_defaults() {
    let cfg = UpdateCheckConfig::default();
    assert!(cfg.auto_update);
    assert_eq!(cfg.check_interval_hours, 24);
}

#[test]
fn update_check_config_from_toml() {
    let toml_str = r#"
        auto_update = false
        check_interval_hours = 12
    "#;

    let cfg: UpdateCheckConfig = toml::from_str(toml_str).unwrap();
    assert!(!cfg.auto_update);
    assert_eq!(cfg.check_interval_hours, 12);
}

#[test]
fn update_check_config_from_toml_invalid() {
    let result: Result<UpdateCheckConfig, _> = toml::from_str("[[invalid");
    assert!(result.is_err());
}
