use super::*;
use std::path::PathBuf;

struct Fixture(PathBuf);

impl Fixture {
    fn new() -> Self {
        let path = crate::test_utils::platform_safe_temp_dir().join(format!(
            "akra-worker-evidence-policy-{}-{:016x}",
            std::process::id(),
            rand::random::<u64>(),
        ));
        crate::private_directory::ensure_private_directory(&path).unwrap();
        Self(std::fs::canonicalize(path).unwrap())
    }

    fn root(&self) -> PathBuf {
        PinnedPrivateDirectory::open(&self.0, false)
            .unwrap()
            .child("evidence", true)
            .unwrap()
            .child("incoming", true)
            .unwrap()
            .child(&"d".repeat(64), true)
            .unwrap()
            .path()
            .to_path_buf()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn read_only_and_full_access_never_add_an_evidence_capability() {
    for mode in [
        SandboxModeValue::ReadOnly,
        SandboxModeValue::DangerFullAccess,
    ] {
        assert!(
            WorkerEvidenceRoot::prepare(Some(Path::new("not-an-absolute-directory")), mode)
                .unwrap()
                .is_none()
        );
    }
    assert!(
        WorkerEvidenceRoot::prepare(None, SandboxModeValue::WorkspaceWrite)
            .unwrap()
            .is_none()
    );
}

#[test]
fn private_generation_root_produces_only_one_additional_writable_root() {
    let fixture = Fixture::new();
    let path = fixture.root();
    let root = WorkerEvidenceRoot::prepare(Some(&path), SandboxModeValue::WorkspaceWrite)
        .unwrap()
        .unwrap();
    let mut workspace = crate::adapter::outbound::app_server::ProtectedThreadWorkspace {
        cwd: fixture.0.to_str().unwrap().into(),
        config: Default::default(),
    };
    root.apply_config(&mut workspace).unwrap();
    assert_eq!(
        workspace.config["sandbox_workspace_write"],
        serde_json::json!({"writable_roots":[path.to_str().unwrap()]})
    );
    let mut request = ConversationRuntimeConfigurationRequest::default();
    root.record_request(&mut request);
    assert!(
        matches!(request.sandbox, ConversationRuntimeRequestedValue::Value(
        ConversationRuntimeSandboxPolicy::WorkspaceWrite { writable_roots: Some(ref roots), .. }
    ) if roots == &vec![path.to_str().unwrap().to_string()])
    );
}

#[test]
fn missing_truncated_or_broader_applied_roots_do_not_satisfy_the_requested_capability() {
    let fixture = Fixture::new();
    let path = fixture.root();
    let root = WorkerEvidenceRoot::prepare(Some(&path), SandboxModeValue::WorkspaceWrite)
        .unwrap()
        .unwrap();
    let cwd = fixture.0.to_str().unwrap();
    let mut envelope = ConversationRuntimeEnvelope::unobserved();
    assert!(root.verify_applied(&envelope, cwd).is_err());
    for (roots, truncated, valid) in [
        (vec![root.value().into()], false, true),
        (vec![root.value().into(), cwd.into()], false, true),
        (vec![root.value().into()], true, false),
        (vec![], false, false),
        (
            vec![
                root.value().into(),
                fixture.0.parent().unwrap().to_str().unwrap().into(),
            ],
            false,
            false,
        ),
    ] {
        envelope.applied.sandbox = ConversationRuntimeObservedValue::Observed(
            ConversationRuntimeSandboxPolicy::WorkspaceWrite {
                network_access: Some(false),
                writable_roots: Some(roots),
                writable_roots_truncated: truncated,
                exclude_tmpdir_env_var: Some(false),
                exclude_slash_tmp: Some(false),
            },
        );
        assert_eq!(root.verify_applied(&envelope, cwd).is_ok(), valid);
    }
}
