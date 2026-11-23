use super::*;
use crate::domain::planning::evidence::TaskEvidenceScope;
use std::fs;
use std::path::Path;

mod host_capture;

struct Fixture {
    root: PathBuf,
    adapter: FilesystemTaskEvidenceArtifactAdapter,
    attempt: TaskEvidenceAttempt,
}

impl Fixture {
    fn new() -> Self {
        let root = crate::test_utils::platform_safe_temp_dir().join(format!(
            "akra-task-artifact-{}-{:016x}",
            std::process::id(),
            rand::random::<u64>(),
        ));
        crate::private_directory::ensure_private_directory(&root).unwrap();
        let root = fs::canonicalize(root).unwrap();
        let runtime = root.clone();
        let adapter =
            FilesystemTaskEvidenceArtifactAdapter::new(Arc::new(move |_| Ok(runtime.clone())));
        let attempt = TaskEvidenceAttempt {
            attempt_id: "a".repeat(64),
            request_id: "prepare-1".into(),
            scope: TaskEvidenceScope {
                task_id: "task-1".into(),
                scope_id: "b".repeat(64),
                acceptance_revision: 1,
                criteria_sha256: "c".repeat(64),
            },
            criterion_id: "A01".into(),
            attempt: 1,
            slot_id: "slot-1".into(),
            lease_generation: "d".repeat(64),
            session_key: "slot-1@generation".into(),
            prepared_tree: TaskEvidenceTree {
                head_commit_sha: "e".repeat(40),
                tree_sha: "f".repeat(40),
                worktree_clean: false,
            },
            prepared_at: "2026-11-23T00:00:00Z".into(),
        };
        Self {
            root,
            adapter,
            attempt,
        }
    }

    fn prepare(&self) -> PathBuf {
        self.adapter
            .prepare_artifact("workspace", &self.attempt)
            .unwrap()
    }
    fn capture(&self) -> Result<TaskEvidenceArtifact> {
        self.adapter
            .observe_and_seal("workspace", &self.attempt, &self.attempt.prepared_tree)
    }
    fn sealed(&self) -> PathBuf {
        self.root
            .join("evidence/sealed")
            .join(&self.attempt.attempt_id)
            .join(ARTIFACT_FILE)
    }
    fn assert_no_capture_scratch(&self) {
        assert!(!self.sealed().parent().unwrap().join("capture").exists());
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn write_private(path: &Path, contents: &[u8]) {
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    #[cfg(windows)]
    {
        use crate::private_fs::*;
        use std::os::windows::fs::OpenOptionsExt;
        options.access_mode(WINDOWS_GENERIC_WRITE | WINDOWS_READ_CONTROL | WINDOWS_WRITE_DAC);
    }
    use std::io::Write;
    let mut file = options.open(path).unwrap();
    // Model a host-created private capture, including its explicit owner ACL.
    #[cfg(windows)]
    crate::private_fs::set_windows_private_acl(&file, false).unwrap();
    file.write_all(contents).unwrap();
}

#[test]
fn preparation_exposes_only_the_generation_incoming_root_and_preserves_existing_output() {
    let mut fixture = Fixture::new();
    fixture.attempt.scope.task_id = "../../raw task id".into();
    fixture.attempt.criterion_id = "criterion/with separators".into();
    let incoming = fixture
        .adapter
        .prepare_lease_incoming_root("workspace", &fixture.attempt.lease_generation)
        .unwrap();
    assert_eq!(
        incoming,
        fixture
            .root
            .join("evidence/incoming")
            .join(&fixture.attempt.lease_generation)
    );
    assert!(!fixture.root.join("evidence/sealed").exists());
    let path = fixture.prepare();
    assert_eq!(
        path,
        incoming
            .join(&fixture.attempt.attempt_id)
            .join(ARTIFACT_FILE)
    );
    fs::write(&path, "test report\n").unwrap();
    assert_eq!(fixture.prepare(), path);
    assert_eq!(fs::read(path).unwrap(), b"test report\n");
}

#[test]
fn capture_hashes_binary_bytes_and_keeps_a_separate_immutable_copy() {
    let fixture = Fixture::new();
    let path = fixture.prepare();
    let bytes = b"test report\0\xff\n";
    fs::write(&path, bytes).unwrap();
    let artifact = fixture.capture().unwrap();
    assert_eq!(artifact.sha256, format!("{:x}", Sha256::digest(bytes)));
    assert_eq!(artifact.size_bytes, bytes.len() as u64);
    assert_eq!(fs::read(fixture.sealed()).unwrap(), bytes);
    fs::write(path, "worker changed the incoming file").unwrap();
    fixture
        .adapter
        .verify_sealed("workspace", &fixture.attempt, &artifact)
        .unwrap();
    assert_eq!(fs::read(fixture.sealed()).unwrap(), bytes);
    fixture.assert_no_capture_scratch();
}

#[test]
fn retry_verifies_the_same_receipt_and_never_overwrites_different_sealed_bytes() {
    let fixture = Fixture::new();
    let path = fixture.prepare();
    fs::write(&path, b"first report").unwrap();
    let first = fixture.capture().unwrap();
    assert_eq!(fixture.capture().unwrap(), first);
    fs::write(path, b"different report").unwrap();
    assert!(fixture.capture().is_err());
    assert_eq!(fs::read(fixture.sealed()).unwrap(), b"first report");
    fixture
        .adapter
        .verify_sealed("workspace", &fixture.attempt, &first)
        .unwrap();
    fixture.assert_no_capture_scratch();
}

#[test]
fn absent_empty_and_oversized_artifacts_never_publish_a_receipt() {
    let fixture = Fixture::new();
    assert!(fixture.capture().is_err());
    let path = fixture.prepare();
    assert!(fixture.capture().is_err());
    fs::OpenOptions::new()
        .write(true)
        .open(&path)
        .unwrap()
        .set_len(MAX_TASK_EVIDENCE_ARTIFACT_BYTES + 1)
        .unwrap();
    assert!(fixture.capture().is_err());
    fs::remove_file(path).unwrap();
    assert!(fixture.capture().is_err());
    assert!(!fixture.sealed().exists());
}

#[test]
fn changed_tree_or_head_is_rejected_before_copying_any_artifact() {
    let fixture = Fixture::new();
    fs::write(fixture.prepare(), b"report").unwrap();
    for changed in [
        TaskEvidenceTree {
            tree_sha: "1".repeat(40),
            ..fixture.attempt.prepared_tree.clone()
        },
        TaskEvidenceTree {
            head_commit_sha: "2".repeat(40),
            ..fixture.attempt.prepared_tree.clone()
        },
    ] {
        assert!(
            fixture
                .adapter
                .observe_and_seal("workspace", &fixture.attempt, &changed)
                .is_err()
        );
    }
    assert!(!fixture.sealed().exists());
}

#[test]
fn a_same_size_rewrite_with_restored_mtime_is_rejected_during_capture() {
    let fixture = Fixture::new();
    let path = fixture.prepare();
    fs::write(&path, b"old report").unwrap();
    let modified = fs::metadata(&path).unwrap().modified().unwrap();
    let result = fixture.adapter.capture(
        "workspace",
        &fixture.attempt,
        &fixture.attempt.prepared_tree,
        || {
            fs::write(&path, b"new report").unwrap();
            fs::OpenOptions::new()
                .write(true)
                .open(&path)
                .unwrap()
                .set_times(fs::FileTimes::new().set_modified(modified))
                .unwrap();
        },
    );
    assert!(result.is_err());
    assert!(!fixture.sealed().exists());
    fixture.assert_no_capture_scratch();
}

#[test]
fn same_bytes_in_a_replacement_file_are_not_the_opened_artifact() {
    let fixture = Fixture::new();
    let path = fixture.prepare();
    fs::write(&path, b"same report").unwrap();
    let result = fixture.adapter.capture(
        "workspace",
        &fixture.attempt,
        &fixture.attempt.prepared_tree,
        || {
            fs::rename(&path, path.with_file_name("previous")).unwrap();
            write_private(&path, b"same report");
        },
    );
    assert!(result.is_err());
    assert!(!fixture.sealed().exists());
    fixture.assert_no_capture_scratch();
}

#[test]
fn an_incomplete_private_capture_is_reclaimed_without_overwriting_the_sealed_file() {
    let fixture = Fixture::new();
    fs::write(fixture.prepare(), b"complete report").unwrap();
    let sealed = fixture
        .adapter
        .sealed("workspace", &fixture.attempt, true)
        .unwrap();
    let scratch = sealed.path().join("capture");
    write_private(&scratch, b"partial previous process bytes");
    drop(sealed);
    let artifact = fixture.capture().unwrap();
    fixture
        .adapter
        .verify_sealed("workspace", &fixture.attempt, &artifact)
        .unwrap();
    assert_eq!(fs::read(fixture.sealed()).unwrap(), b"complete report");
    fixture.assert_no_capture_scratch();
}

#[test]
fn verification_never_repairs_missing_or_modified_sealed_output() {
    let fixture = Fixture::new();
    let expected = artifact_for_bytes(b"report").unwrap();
    assert!(
        fixture
            .adapter
            .verify_sealed("workspace", &fixture.attempt, &expected)
            .is_err()
    );
    assert!(!fixture.root.join("evidence").exists());
    fs::write(fixture.prepare(), b"report").unwrap();
    let artifact = fixture.capture().unwrap();
    fs::write(fixture.sealed(), b"different").unwrap();
    assert!(
        fixture
            .adapter
            .verify_sealed("workspace", &fixture.attempt, &artifact)
            .is_err()
    );
    fs::remove_file(fixture.sealed()).unwrap();
    assert!(
        fixture
            .adapter
            .verify_sealed("workspace", &fixture.attempt, &artifact)
            .is_err()
    );
    assert!(!fixture.sealed().exists());
}

#[test]
fn invalid_host_identifiers_do_not_create_paths() {
    let mut fixture = Fixture::new();
    let uppercase = "A".repeat(64);
    for generation in ["../outside", uppercase.as_str(), "short"] {
        assert!(
            fixture
                .adapter
                .prepare_lease_incoming_root("workspace", generation)
                .is_err()
        );
    }
    fixture.attempt.attempt_id = "../outside".into();
    assert!(
        fixture
            .adapter
            .prepare_artifact("workspace", &fixture.attempt)
            .is_err()
    );
    assert!(!fixture.root.join("evidence").exists());
}

#[test]
fn a_sealed_retry_needs_no_second_capture_file() {
    let fixture = Fixture::new();
    fs::write(fixture.prepare(), b"report").unwrap();
    let artifact = fixture.capture().unwrap();
    let capture = fixture.sealed().parent().unwrap().join("capture");
    fs::create_dir(&capture).unwrap();
    assert_eq!(fixture.capture().unwrap(), artifact);
    assert!(capture.is_dir());
}

#[test]
fn missing_sealed_artifact_has_the_typed_missing_reason() {
    let fixture = Fixture::new();
    let artifact = artifact_for_bytes(b"report").unwrap();
    let error = fixture
        .adapter
        .verify_sealed("workspace", &fixture.attempt, &artifact)
        .unwrap_err();
    assert_eq!(
        error.downcast_ref::<TaskEvidenceError>(),
        Some(&TaskEvidenceError::ArtifactMissing)
    );
}

#[test]
fn rewriting_the_same_bytes_and_restoring_mtime_still_invalidates_capture() {
    let fixture = Fixture::new();
    let path = fixture.prepare();
    fs::write(&path, b"old report").unwrap();
    let modified = fs::metadata(&path).unwrap().modified().unwrap();
    let result = fixture.adapter.capture(
        "workspace",
        &fixture.attempt,
        &fixture.attempt.prepared_tree,
        || {
            fs::write(&path, b"new report").unwrap();
            fs::write(&path, b"old report").unwrap();
            fs::OpenOptions::new()
                .write(true)
                .open(&path)
                .unwrap()
                .set_times(fs::FileTimes::new().set_modified(modified))
                .unwrap();
        },
    );
    assert!(result.is_err());
    assert!(!fixture.sealed().exists());
    fixture.assert_no_capture_scratch();
}

#[cfg(unix)]
#[test]
fn symlink_and_hardlink_artifacts_are_rejected_without_touching_their_target() {
    use std::os::unix::fs::symlink;
    let fixture = Fixture::new();
    let path = fixture.prepare();
    let target = fixture.root.join("outside");
    write_private(&target, b"outside bytes");
    fs::remove_file(&path).unwrap();
    symlink(&target, &path).unwrap();
    assert!(fixture.capture().is_err());
    assert!(
        fixture
            .adapter
            .prepare_artifact("workspace", &fixture.attempt)
            .is_err()
    );
    fs::remove_file(&path).unwrap();
    fs::hard_link(&target, &path).unwrap();
    assert!(fixture.capture().is_err());
    assert_eq!(fs::read(target).unwrap(), b"outside bytes");
    assert!(!fixture.sealed().exists());
}

#[cfg(unix)]
#[test]
fn symlinked_intermediate_directory_is_rejected_before_creating_descendants() {
    use std::os::unix::fs::symlink;
    let fixture = Fixture::new();
    let outside = fixture.root.join("outside");
    crate::private_directory::ensure_private_directory(&outside).unwrap();
    symlink(&outside, fixture.root.join("evidence")).unwrap();
    assert!(
        fixture
            .adapter
            .prepare_artifact("workspace", &fixture.attempt)
            .is_err()
    );
    assert!(fs::read_dir(outside).unwrap().next().is_none());
}

#[cfg(unix)]
#[test]
fn a_sealed_destination_symlink_is_never_followed_or_replaced() {
    use std::os::unix::fs::symlink;
    let fixture = Fixture::new();
    fs::write(fixture.prepare(), b"report").unwrap();
    let target = fixture.root.join("outside");
    write_private(&target, b"outside");
    let sealed = fixture
        .adapter
        .sealed("workspace", &fixture.attempt, true)
        .unwrap();
    symlink(&target, sealed.path().join(ARTIFACT_FILE)).unwrap();
    drop(sealed);
    assert!(fixture.capture().is_err());
    assert_eq!(fs::read(target).unwrap(), b"outside");
    assert!(
        fs::symlink_metadata(fixture.sealed())
            .unwrap()
            .file_type()
            .is_symlink()
    );
    fixture.assert_no_capture_scratch();
}

#[cfg(unix)]
#[test]
fn unsafe_existing_file_permissions_are_rejected_without_repair() {
    use std::os::unix::fs::PermissionsExt;
    let fixture = Fixture::new();
    let path = fixture.prepare();
    fs::write(&path, b"report").unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
    assert!(fixture.capture().is_err());
    assert!(
        fixture
            .adapter
            .prepare_artifact("workspace", &fixture.attempt)
            .is_err()
    );
    assert_eq!(
        fs::metadata(path).unwrap().permissions().mode() & 0o777,
        0o644
    );
}

#[cfg(unix)]
#[test]
fn replacing_an_incoming_directory_during_capture_fails_the_whole_observation() {
    let fixture = Fixture::new();
    let path = fixture.prepare();
    fs::write(&path, b"report").unwrap();
    let parent = path.parent().unwrap();
    let result = fixture.adapter.capture(
        "workspace",
        &fixture.attempt,
        &fixture.attempt.prepared_tree,
        || {
            fs::rename(parent, parent.with_extension("old")).unwrap();
            crate::private_directory::ensure_private_directory(parent).unwrap();
            write_private(&path, b"report");
        },
    );
    assert!(result.is_err());
    assert!(!fixture.sealed().exists());
    fixture.assert_no_capture_scratch();
}

#[cfg(unix)]
#[test]
fn fifo_artifacts_fail_without_waiting_for_a_writer() {
    use std::os::unix::ffi::OsStrExt;
    let fixture = Fixture::new();
    let path = fixture.prepare();
    fs::remove_file(&path).unwrap();
    let path = std::ffi::CString::new(path.as_os_str().as_bytes()).unwrap();
    // SAFETY: the NUL-terminated fixture path remains valid for mkfifo.
    assert_eq!(unsafe { libc::mkfifo(path.as_ptr(), 0o600) }, 0);
    assert!(fixture.capture().is_err());
    assert!(!fixture.sealed().exists());
}

#[cfg(windows)]
#[test]
fn junction_ancestors_are_rejected_before_any_artifact_child_is_created() {
    use std::process::{Command, Stdio};
    let fixture = Fixture::new();
    let target = fixture.root.join("outside");
    crate::private_directory::ensure_private_directory(&target).unwrap();
    fs::write(target.join("sentinel"), b"unchanged").unwrap();
    let link = fixture.root.join("evidence");
    let link_text = link.to_string_lossy();
    let target_text = target.to_string_lossy();
    let mut command = Command::new("cmd");
    command
        .args([
            "/C",
            "mklink",
            "/J",
            link_text.strip_prefix(r"\\?\").unwrap_or(&link_text),
            target_text.strip_prefix(r"\\?\").unwrap_or(&target_text),
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let output = crate::subprocess::wait_with_output_timeout(
        crate::subprocess::spawn(&mut command).unwrap(),
        "task evidence junction fixture",
        std::time::Duration::from_secs(10),
    )
    .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        fixture
            .adapter
            .prepare_artifact("workspace", &fixture.attempt)
            .is_err()
    );
    assert_eq!(fs::read(target.join("sentinel")).unwrap(), b"unchanged");
    assert!(!target.join("incoming").exists());
}

#[cfg(windows)]
#[test]
fn windows_runtime_resolver_and_capture_pins_share_one_prepared_root() {
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
    let path = adapter
        .prepare_artifact(&workspace, &fixture.attempt)
        .unwrap();
    assert!(
        adapter
            .observe_and_seal(&workspace, &fixture.attempt, &fixture.attempt.prepared_tree)
            .is_err()
    );
    let bytes = b"host capture with the real Windows runtime resolver\n";
    fs::write(&path, bytes).unwrap();
    let artifact = adapter
        .observe_and_seal(&workspace, &fixture.attempt, &fixture.attempt.prepared_tree)
        .unwrap();
    assert_eq!(artifact.sha256, format!("{:x}", Sha256::digest(bytes)));
    assert_eq!(
        adapter
            .observe_and_seal(&workspace, &fixture.attempt, &fixture.attempt.prepared_tree)
            .unwrap(),
        artifact
    );
    adapter
        .verify_sealed(&workspace, &fixture.attempt, &artifact)
        .unwrap();
    fs::write(path, b"different incoming bytes").unwrap();
    assert!(
        adapter
            .observe_and_seal(&workspace, &fixture.attempt, &fixture.attempt.prepared_tree)
            .is_err()
    );
    adapter
        .verify_sealed(&workspace, &fixture.attempt, &artifact)
        .unwrap();
    let _ = fs::remove_dir_all(runtime);
}

#[cfg(windows)]
#[test]
fn long_nonverbatim_runtime_preserves_pins_and_immutable_artifact_publication() {
    use std::os::windows::ffi::OsStrExt;

    let fixture = Fixture::new();
    let mut runtime = PathBuf::from(
        crate::trusted_executable::windows_process_argument_path(&fixture.root).unwrap(),
    );
    for _ in 0..4 {
        runtime.push("long-runtime-component".repeat(3));
    }
    assert!(!runtime.to_string_lossy().starts_with(r"\\?\"));
    assert!(runtime.as_os_str().encode_wide().count() > 260);
    let pinned = PinnedPrivateDirectory::open(&runtime, true).unwrap();
    let renamed = runtime.with_file_name("renamed-runtime");
    assert!(fs::rename(&runtime, &renamed).is_err());
    pinned.verify().unwrap();
    drop(pinned);
    fs::rename(&runtime, &renamed).unwrap();
    fs::rename(&renamed, &runtime).unwrap();

    let sealed = runtime
        .join("evidence")
        .join("sealed")
        .join(&fixture.attempt.attempt_id)
        .join(ARTIFACT_FILE);
    let adapter =
        FilesystemTaskEvidenceArtifactAdapter::new(Arc::new(move |_| Ok(runtime.clone())));
    let path = adapter
        .prepare_artifact("workspace", &fixture.attempt)
        .unwrap();
    let bytes = b"immutable capture through a long non-verbatim Windows path\n";
    fs::write(&path, bytes).unwrap();
    let artifact = adapter
        .observe_and_seal(
            "workspace",
            &fixture.attempt,
            &fixture.attempt.prepared_tree,
        )
        .unwrap();
    assert_eq!(artifact.sha256, format!("{:x}", Sha256::digest(bytes)));
    assert_eq!(artifact.size_bytes, bytes.len() as u64);
    assert_eq!(
        adapter
            .observe_and_seal(
                "workspace",
                &fixture.attempt,
                &fixture.attempt.prepared_tree
            )
            .unwrap(),
        artifact
    );
    fs::write(path, b"changed incoming bytes").unwrap();
    assert!(
        adapter
            .observe_and_seal(
                "workspace",
                &fixture.attempt,
                &fixture.attempt.prepared_tree
            )
            .is_err()
    );
    adapter
        .verify_sealed("workspace", &fixture.attempt, &artifact)
        .unwrap();
    assert_eq!(fs::read(&sealed).unwrap(), bytes);
    assert!(!sealed.parent().unwrap().join("capture").exists());
}
