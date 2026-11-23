use super::*;
use std::io::Write;

#[test]
fn global_temporary_handle_can_protect_its_acl_without_reopening() {
    let root = temporary_directory("global-temporary-private-acl");
    let path = root.join("home").join(".config.toml.test.tmp");
    ensure_config_parent(&path, ConfigScope::Global).unwrap();

    let mut file = create_private_temporary_file(&path, ConfigScope::Global).unwrap();
    file.write_all(b"schema_version = 1\n").unwrap();
    file.sync_all().unwrap();
    crate::private_fs::set_windows_private_acl(&file, false).unwrap();
    crate::private_fs::validate_windows_private_owner_and_acl(&path, &file).unwrap();
    crate::private_fs::validate_windows_path_identity_only(&path, &file, false).unwrap();

    let collision = create_private_temporary_file(&path, ConfigScope::Global)
        .expect_err("a temporary file must still be created exclusively");
    assert_eq!(collision.kind(), std::io::ErrorKind::AlreadyExists);
    drop(file);
    assert_eq!(
        read_config_file_if_present(&path, ConfigScope::Global)
            .unwrap()
            .as_deref(),
        Some("schema_version = 1\n"),
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn global_create_replace_and_unset_preserve_the_private_owner_acl() {
    let root = temporary_directory("global-atomic-private-acl");
    // An explicit global-only path keeps this filesystem contract independent
    // of Git discovery, legacy Git settings, and the ambient configuration.
    let paths = ConfigPaths {
        global: root.join("home").join(CONFIG_FILE_NAME),
        project: None,
        workspace_root: None,
    };
    let key = SettingKey::ParallelEvidenceMode;

    for (raw_value, mode) in [
        ("observe", TaskEvidenceDeliveryMode::Observe),
        ("enforce", TaskEvidenceDeliveryMode::Enforce),
    ] {
        ConfigurationService::set_with_paths(paths.clone(), ConfigScope::Global, key, raw_value)
            .unwrap();
        assert_private_global_file(&paths.global);
        let resolved = resolve_global_configuration(&paths);
        assert_eq!(resolved.config.parallel.evidence_mode, mode);
        assert_eq!(resolved.origin_for(key), &SettingOrigin::GlobalToml);
    }

    ConfigurationService::unset_with_paths(paths.clone(), ConfigScope::Global, key).unwrap();
    assert_private_global_file(&paths.global);
    let contents = read_config_file_if_present(&paths.global, ConfigScope::Global)
        .unwrap()
        .unwrap();
    assert!(!contents.contains("evidence_mode"), "{contents}");
    let resolved = resolve_global_configuration(&paths);
    assert_eq!(
        resolved.config.parallel.evidence_mode,
        TaskEvidenceDeliveryMode::Observe,
    );
    assert_eq!(resolved.origin_for(key), &SettingOrigin::Builtin);
    let _ = fs::remove_dir_all(root);
}

fn assert_private_global_file(path: &Path) {
    let file = open_config_file_for_read(path).unwrap();
    crate::private_fs::validate_windows_path_identity_only(path, &file, false).unwrap();
    crate::private_fs::validate_windows_private_owner_and_acl(path, &file).unwrap();
}

fn resolve_global_configuration(paths: &ConfigPaths) -> ResolvedAkraConfig {
    ConfigurationService::resolve_with_paths_and_environment(
        paths.clone(),
        &[],
        EnvironmentConfigAdapter::from_values(&[]).unwrap(),
    )
    .unwrap()
}
