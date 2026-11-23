use super::*;

#[test]
fn host_capture_uses_separate_bytes_and_namespace_from_worker_reports() {
    let fixture = Fixture::new();
    let incoming = fixture.prepare();
    fs::write(incoming, b"worker supplied status: pass").unwrap();
    let worker = fixture.capture().unwrap();
    let bytes = b"host generated command/environment/exit observation";
    let host = fixture
        .adapter
        .seal_host_capture("workspace", &fixture.attempt.attempt_id, bytes)
        .unwrap();
    assert_ne!(host, worker);
    fixture
        .adapter
        .verify_host_capture("workspace", &fixture.attempt.attempt_id, &host)
        .unwrap();
    fixture
        .adapter
        .verify_sealed("workspace", &fixture.attempt, &worker)
        .unwrap();
    assert_eq!(
        fs::read(
            fixture
                .root
                .join("evidence/sealed/verification")
                .join(&fixture.attempt.attempt_id)
                .join(ARTIFACT_FILE)
        )
        .unwrap(),
        bytes
    );
}

#[test]
fn host_capture_replay_is_exact_and_cannot_overwrite_a_previous_execution() {
    let fixture = Fixture::new();
    let id = &fixture.attempt.attempt_id;
    let first = fixture
        .adapter
        .seal_host_capture("workspace", id, b"first execution")
        .unwrap();
    assert_eq!(
        first,
        fixture
            .adapter
            .seal_host_capture("workspace", id, b"first execution")
            .unwrap()
    );
    assert!(
        fixture
            .adapter
            .seal_host_capture("workspace", id, b"different execution")
            .is_err()
    );
    fixture
        .adapter
        .verify_host_capture("workspace", id, &first)
        .unwrap();
    let directory = fixture.root.join("evidence/sealed/verification").join(id);
    assert_eq!(
        fs::read(directory.join(ARTIFACT_FILE)).unwrap(),
        b"first execution"
    );
    assert!(!directory.join("capture").exists());
    assert!(!fixture.root.join("evidence/incoming").exists());
}

#[test]
fn host_capture_rejects_invalid_empty_or_oversized_inputs_before_allocation() {
    let fixture = Fixture::new();
    assert!(
        fixture
            .adapter
            .seal_host_capture("workspace", "../arbitrary", b"bytes")
            .is_err()
    );
    assert_eq!(
        fixture
            .adapter
            .seal_host_capture("workspace", &fixture.attempt.attempt_id, b"")
            .unwrap_err()
            .downcast_ref::<TaskEvidenceError>(),
        Some(&TaskEvidenceError::ArtifactMissing)
    );
    assert_eq!(
        fixture
            .adapter
            .seal_host_capture(
                "workspace",
                &fixture.attempt.attempt_id,
                &vec![b'x'; MAX_TASK_EVIDENCE_ARTIFACT_BYTES as usize + 1]
            )
            .unwrap_err()
            .downcast_ref::<TaskEvidenceError>(),
        Some(&TaskEvidenceError::Limit)
    );
    assert!(!fixture.root.join("evidence").exists());
}

#[test]
fn host_capture_verification_detects_tampering_and_missing_bytes_without_repair() {
    let fixture = Fixture::new();
    let id = &fixture.attempt.attempt_id;
    let artifact = fixture
        .adapter
        .seal_host_capture("workspace", id, b"host execution")
        .unwrap();
    let path = fixture
        .root
        .join("evidence/sealed/verification")
        .join(id)
        .join(ARTIFACT_FILE);
    fs::write(&path, b"forged receipt").unwrap();
    assert!(
        fixture
            .adapter
            .verify_host_capture("workspace", id, &artifact)
            .is_err()
    );
    assert_eq!(fs::read(&path).unwrap(), b"forged receipt");
    fs::remove_file(&path).unwrap();
    let error = fixture
        .adapter
        .verify_host_capture("workspace", id, &artifact)
        .unwrap_err();
    assert_eq!(
        error.downcast_ref::<TaskEvidenceError>(),
        Some(&TaskEvidenceError::ArtifactMissing)
    );
    assert!(!path.exists());
}

#[cfg(windows)]
#[test]
fn host_windows_real_runtime_resolver_and_long_capture_paths_preserve_replay() {
    use crate::adapter::outbound::db::SqlitePlanningAuthorityAdapter;
    let fixture = Fixture::new();
    let workspace = fixture.root.to_string_lossy().into_owned();
    let runtime = SqlitePlanningAuthorityAdapter::prepare_private_runtime_directory(&workspace)
        .unwrap()
        .runtime_dir;
    let adapter = FilesystemTaskEvidenceArtifactAdapter::new(Arc::new(|workspace| {
        Ok(PathBuf::from(
            SqlitePlanningAuthorityAdapter::prepare_private_runtime_directory(workspace)?
                .runtime_dir,
        ))
    }));
    let id = &fixture.attempt.attempt_id;
    let artifact = adapter
        .seal_host_capture(&workspace, id, b"host only immutable bytes")
        .unwrap();
    assert_eq!(
        artifact,
        adapter
            .seal_host_capture(&workspace, id, b"host only immutable bytes")
            .unwrap()
    );
    assert!(
        adapter
            .seal_host_capture(&workspace, id, b"replacement")
            .is_err()
    );
    adapter
        .verify_host_capture(&workspace, id, &artifact)
        .unwrap();
    let _ = fs::remove_dir_all(runtime);
}
