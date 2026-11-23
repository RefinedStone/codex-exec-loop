use super::*;

#[cfg(windows)]
mod windows_private_write;

#[test]
fn evidence_mode_unset_removes_inline_and_block_leaves_without_losing_siblings() {
    for body in [
        "# retained configuration note\nschema_version = 1\nparallel = { evidence_mode = 'enforce', integration_branch = 'prerelease' }\n",
        "# retained configuration note\nschema_version = 1\n[parallel]\nevidence_mode = 'enforce'\nintegration_branch = 'prerelease'\n",
    ] {
        let mut document = body.parse::<DocumentMut>().unwrap();
        let mut layer = toml::from_str::<AkraConfigLayer>(body).unwrap();
        layer.unset_value(SettingKey::ParallelEvidenceMode);
        sync_document_layer(&mut document, &layer);

        let rewritten = document.to_string();
        let parsed = toml::from_str::<AkraConfigLayer>(&rewritten).unwrap();
        let parallel = parsed.parallel.unwrap();
        assert_eq!(parallel.evidence_mode, None, "{rewritten}");
        assert_eq!(
            parallel.integration_branch.as_deref(),
            Some("prerelease"),
            "{rewritten}",
        );
        assert!(rewritten.contains("# retained configuration note"));

        layer.unset_value(SettingKey::ParallelIntegrationBranch);
        sync_document_layer(&mut document, &layer);
        assert!(document.get("parallel").is_none(), "{document}");
        assert_eq!(document["schema_version"].as_integer(), Some(1));
    }
}

#[test]
fn evidence_mode_unset_removes_a_newly_written_explicit_default_or_override() {
    for mode in [
        TaskEvidenceDeliveryMode::Observe,
        TaskEvidenceDeliveryMode::Enforce,
    ] {
        let mut document = DocumentMut::new();
        let mut layer = AkraConfigLayer::default();
        layer.set_value(
            SettingKey::ParallelEvidenceMode,
            SettingValue::EvidenceMode(mode),
        );
        sync_document_layer(&mut document, &layer);
        let written = toml::from_str::<AkraConfigLayer>(&document.to_string()).unwrap();
        assert_eq!(written.parallel.unwrap().evidence_mode, Some(mode));

        layer.unset_value(SettingKey::ParallelEvidenceMode);
        sync_document_layer(&mut document, &layer);
        let rewritten = document.to_string();
        let parsed = toml::from_str::<AkraConfigLayer>(&rewritten).unwrap();
        assert!(parsed.parallel.is_none(), "{rewritten}");
        assert!(document.get("parallel").is_none(), "{document}");
    }
}

#[test]
fn evidence_mode_defaults_to_observe_and_preserves_layer_precedence() {
    let root = temporary_directory("evidence-mode-precedence");
    let workspace = root.join("workspace");
    fake_git_worktree(&workspace);
    let fixture = ConfigFixture::new(&root.join("home"));
    let key = SettingKey::ParallelEvidenceMode;

    let default = fixture.resolve_read_only(&workspace, &[]).unwrap();
    assert_eq!(
        default.config.parallel.evidence_mode,
        TaskEvidenceDeliveryMode::Observe
    );
    assert_eq!(default.origin_for(key), &SettingOrigin::Builtin);
    assert!(!fixture.global.exists());

    fixture
        .set(&workspace, ConfigScope::Global, key, "enforce")
        .unwrap();
    let global_before = fs::read(&fixture.global).unwrap();
    fixture
        .set(&workspace, ConfigScope::Project, key, "observe")
        .unwrap();
    let project = fixture.resolve_read_only(&workspace, &[]).unwrap();
    assert_eq!(
        project.config.parallel.evidence_mode,
        TaskEvidenceDeliveryMode::Observe
    );
    assert_eq!(project.origin_for(key), &SettingOrigin::ProjectToml);
    let overrides = [ConfigOverride::parse("parallel.evidence_mode=enforce").unwrap()];
    let command = fixture.resolve_read_only(&workspace, &overrides).unwrap();
    assert_eq!(
        command.config.parallel.evidence_mode,
        TaskEvidenceDeliveryMode::Enforce
    );
    assert_eq!(command.origin_for(key), &SettingOrigin::CommandLine);

    fixture
        .unset(&workspace, ConfigScope::Project, key)
        .unwrap();
    let project_layer = fixture
        .layer_for_scope(&workspace, ConfigScope::Project)
        .unwrap()
        .unwrap();
    assert!(project_layer.value_for(key).is_none());
    assert_eq!(fs::read(&fixture.global).unwrap(), global_before);
    let inherited = fixture.resolve_read_only(&workspace, &[]).unwrap();
    assert_eq!(
        inherited.config.parallel.evidence_mode,
        TaskEvidenceDeliveryMode::Enforce
    );
    assert_eq!(inherited.origin_for(key), &SettingOrigin::GlobalToml);
    assert!(
        inherited
            .effective_settings()
            .iter()
            .any(|setting| { setting.key == key && setting.value == "enforce" })
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn evidence_mode_rejects_invalid_values_without_replacing_valid_configuration() {
    let root = temporary_directory("evidence-mode-invalid");
    let workspace = root.join("workspace");
    fake_git_worktree(&workspace);
    let fixture = ConfigFixture::new(&root.join("home"));
    let key = SettingKey::ParallelEvidenceMode;
    let path = fixture
        .set(&workspace, ConfigScope::Project, key, "enforce")
        .unwrap();
    let before = fs::read(&path).unwrap();

    for value in [
        "",
        "Observe",
        "ENFORCE",
        "enforce\n",
        " enforce",
        "observe ",
        "on",
        "off",
    ] {
        assert!(
            fixture
                .set(&workspace, ConfigScope::Project, key, value)
                .is_err(),
            "{value:?}"
        );
        assert!(
            ConfigOverride::parse(&format!("parallel.evidence_mode={value}")).is_err(),
            "{value:?}"
        );
        assert_eq!(fs::read(&path).unwrap(), before);
    }
    for body in [
        "[parallel]\nevidence_mode = true\n",
        "[parallel]\nevidence_mode = 'ENFORCE'\n",
        "[parallel]\nevidence_mode = 'observe '\n",
        "[parallel]\nevidence_mode = 0\n",
    ] {
        fs::write(&path, body).unwrap();
        let override_value = ConfigOverride::parse("parallel.evidence_mode=observe").unwrap();
        assert!(
            fixture
                .resolve_read_only(&workspace, &[override_value])
                .is_err(),
            "{body}"
        );
    }
    fs::remove_file(&path).unwrap();
    fs::create_dir(&path).unwrap();
    assert!(
        fixture.resolve_read_only(&workspace, &[]).is_err(),
        "an unreadable layer must not become observe"
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn resolved_evidence_mode_stays_pinned_after_selecting_the_target_workspace() {
    let root = temporary_directory("evidence-mode-pinned");
    let workspace = root.join("canonical");
    let caller = root.join("linked-worktree");
    fake_git_worktree(&workspace);
    fake_git_worktree(&caller);
    let fixture = ConfigFixture::new(&root.join("home"));
    let key = SettingKey::ParallelEvidenceMode;
    fixture
        .set(&caller, ConfigScope::Project, key, "observe")
        .unwrap();
    fixture
        .set(&workspace, ConfigScope::Project, key, "enforce")
        .unwrap();
    let process = fixture.resolve_read_only(&caller, &[]).unwrap();
    let selected = process.resolve_for_workspace(&workspace).unwrap();
    assert_eq!(
        selected.config.parallel.evidence_mode,
        TaskEvidenceDeliveryMode::Enforce
    );

    fixture
        .set(&workspace, ConfigScope::Project, key, "observe")
        .unwrap();
    fixture
        .set(&caller, ConfigScope::Project, key, "enforce")
        .unwrap();
    assert_eq!(
        selected.config.parallel.evidence_mode,
        TaskEvidenceDeliveryMode::Enforce
    );
    assert_eq!(
        selected
            .resolve_for_workspace(&workspace.join("nested"))
            .unwrap()
            .config
            .parallel
            .evidence_mode,
        TaskEvidenceDeliveryMode::Enforce,
    );
    assert_eq!(
        process.config.parallel.evidence_mode,
        TaskEvidenceDeliveryMode::Observe
    );
    assert_eq!(
        fixture
            .resolve_read_only(&workspace, &[])
            .unwrap()
            .config
            .parallel
            .evidence_mode,
        TaskEvidenceDeliveryMode::Observe,
    );
    let _ = fs::remove_dir_all(root);
}
