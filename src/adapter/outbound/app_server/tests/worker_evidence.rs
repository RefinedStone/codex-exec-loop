use super::*;
use crate::adapter::outbound::filesystem::FilesystemTaskEvidenceArtifactAdapter;
use crate::application::port::outbound::task_evidence_artifact_port::TaskEvidenceArtifactPort;

fn incoming_root(fake: &FakeCodex) -> PathBuf {
    let runtime = fake.temp_dir.join("private-runtime");
    crate::private_directory::ensure_private_directory(&runtime).unwrap();
    let runtime = fs::canonicalize(runtime).unwrap();
    let adapter =
        FilesystemTaskEvidenceArtifactAdapter::new(Arc::new(move |_| Ok(runtime.clone())));
    adapter
        .prepare_lease_incoming_root("workspace", &"d".repeat(64))
        .unwrap()
}

fn adapter(fake: &FakeCodex, sandbox_mode: SandboxModeValue) -> CodexAppServerAdapter {
    CodexAppServerAdapter::with_configs(
        "test-client",
        "test-version",
        fake.connection_config(),
        AppServerExecutionPolicy {
            sandbox_mode,
            ..AppServerExecutionPolicy::default()
        },
    )
}

fn assert_wire_roots(fake: &FakeCodex, thread_method: &str, root: &Path) {
    let requests = fake.logged_requests();
    let thread = requests
        .iter()
        .find(|request| request["method"] == thread_method)
        .unwrap();
    let turn = requests
        .iter()
        .find(|request| request["method"] == "turn/start")
        .unwrap();
    let expected = serde_json::json!([root.to_str().unwrap()]);
    assert_eq!(thread["params"]["sandbox"], "workspace-write");
    assert_eq!(
        thread["params"]["config"]["sandbox_workspace_write"]["writable_roots"],
        expected
    );
    assert_eq!(
        turn["params"]["sandboxPolicy"],
        serde_json::json!({"type":"workspaceWrite","writableRoots":expected})
    );
    let serialized = serde_json::to_string(&requests).unwrap();
    assert!(!serialized.contains("/evidence/sealed"));
    assert!(!serialized.contains("planning-authority.sqlite"));
}

#[test]
fn initial_worker_applies_only_its_incoming_generation_to_thread_and_turn() {
    let fake = FakeCodex::install("worker-evidence-initial");
    let root = incoming_root(&fake);
    let (tx, rx) = conversation_stream_channel();
    let receipt = adapter(&fake, SandboxModeValue::WorkspaceWrite)
        .run_isolated_new_thread_stream(
            ParallelAgentWorkerStreamRequest {
                cwd: "/repo",
                evidence_incoming_root: Some(&root),
                prompt: "perform the accepted task",
                developer_instructions: "Report evidence through the host commands.",
                service_name: "worker-evidence-fixture",
            },
            tx,
        )
        .unwrap();
    assert!(receipt.is_completed_and_confirmed());
    let events = rx.try_iter().collect::<Vec<_>>();
    let envelope = prepared_runtime_envelope(&events, "started-thread");
    let requested = &envelope.thread_request.sandbox;
    assert!(matches!(requested,
        ConversationRuntimeRequestedValue::Value(ConversationRuntimeSandboxPolicy::WorkspaceWrite {
            writable_roots: Some(roots), ..
        }) if roots == &vec![root.to_str().unwrap().to_string()]
    ));
    assert_wire_roots(&fake, "thread/start", &root);
}

#[test]
fn resumed_worker_reuses_the_same_generation_root_and_never_starts_another_thread() {
    let fake = FakeCodex::install_with_scenario("worker-evidence-resume", "continuation_active");
    let root = incoming_root(&fake);
    let (tx, rx) = conversation_stream_channel();
    let receipt = adapter(&fake, SandboxModeValue::WorkspaceWrite)
        .run_isolated_resumed_thread_stream(
            ParallelAgentWorkerResumeRequest {
                cwd: "/repo",
                evidence_incoming_root: Some(&root),
                thread_id: "waiting-thread",
                prompt: "answer and continue",
                service_name: "worker-evidence-fixture",
            },
            tx,
            &allow_worker_continuation_start,
        )
        .unwrap();
    assert!(receipt.is_completed_and_confirmed());
    assert!(has_thread_prepared(
        &rx.try_iter().collect::<Vec<_>>(),
        "waiting-thread"
    ));
    assert_wire_roots(&fake, "thread/resume", &root);
    assert!(
        !fake
            .logged_methods()
            .iter()
            .any(|method| method == "thread/start")
    );
}

#[test]
fn evidence_never_promotes_read_only_or_changes_an_existing_full_access_policy() {
    for (mode, thread_label, turn_label) in [
        (SandboxModeValue::ReadOnly, "read-only", "readOnly"),
        (
            SandboxModeValue::DangerFullAccess,
            "danger-full-access",
            "dangerFullAccess",
        ),
    ] {
        let fake = FakeCodex::install("worker-evidence-policy");
        let (tx, _rx) = conversation_stream_channel();
        adapter(&fake, mode)
            .run_isolated_new_thread_stream(
                ParallelAgentWorkerStreamRequest {
                    cwd: "/repo",
                    evidence_incoming_root: Some(Path::new("/unused/not-a-capability")),
                    prompt: "run",
                    developer_instructions: "",
                    service_name: "worker-evidence-fixture",
                },
                tx,
            )
            .unwrap();
        let requests = fake.logged_requests();
        let thread = requests
            .iter()
            .find(|request| request["method"] == "thread/start")
            .unwrap();
        let turn = requests
            .iter()
            .find(|request| request["method"] == "turn/start")
            .unwrap();
        assert_eq!(thread["params"]["sandbox"], thread_label);
        assert!(thread["params"]["config"]["sandbox_workspace_write"].is_null());
        assert_eq!(
            turn["params"]["sandboxPolicy"],
            serde_json::json!({"type":turn_label})
        );
    }
}

#[test]
fn workers_without_evidence_keep_the_existing_workspace_write_payload() {
    let fake = FakeCodex::install("worker-evidence-none");
    let (tx, _rx) = conversation_stream_channel();
    adapter(&fake, SandboxModeValue::WorkspaceWrite)
        .run_isolated_new_thread_stream(
            ParallelAgentWorkerStreamRequest {
                cwd: "/repo",
                evidence_incoming_root: None,
                prompt: "run",
                developer_instructions: "",
                service_name: "worker-evidence-fixture",
            },
            tx,
        )
        .unwrap();
    let requests = fake.logged_requests();
    let turn = requests
        .iter()
        .find(|request| request["method"] == "turn/start")
        .unwrap();
    assert_eq!(
        turn["params"]["sandboxPolicy"],
        serde_json::json!({"type":"workspaceWrite"})
    );
}

#[test]
fn missing_broader_or_replaced_applied_root_never_starts_a_model_turn() {
    for scenario in [
        "evidence_root_missing",
        "evidence_root_broad",
        "evidence_root_replaced",
    ] {
        let fake = FakeCodex::install_with_scenario(scenario, scenario);
        let root = incoming_root(&fake);
        let (tx, _rx) = conversation_stream_channel();
        let result = adapter(&fake, SandboxModeValue::WorkspaceWrite)
            .run_isolated_new_thread_stream(
                ParallelAgentWorkerStreamRequest {
                    cwd: "/repo",
                    evidence_incoming_root: Some(&root),
                    prompt: "run",
                    developer_instructions: "",
                    service_name: "worker-evidence-fixture",
                },
                tx,
            );
        assert!(result.is_err(), "{scenario}");
        assert!(
            !fake
                .logged_methods()
                .iter()
                .any(|method| method == "turn/start")
        );
    }
}

#[test]
fn arbitrary_parent_or_other_lease_paths_are_not_additional_worker_capabilities() {
    let fake = FakeCodex::install("worker-evidence-path-shape");
    let root = incoming_root(&fake);
    for invalid in [
        root.parent().unwrap(),
        root.parent().unwrap().parent().unwrap(),
        Path::new("relative"),
    ] {
        let (tx, _rx) = conversation_stream_channel();
        assert!(
            adapter(&fake, SandboxModeValue::WorkspaceWrite)
                .run_isolated_new_thread_stream(
                    ParallelAgentWorkerStreamRequest {
                        cwd: "/repo",
                        evidence_incoming_root: Some(invalid),
                        prompt: "run",
                        developer_instructions: "",
                        service_name: "worker-evidence-fixture",
                    },
                    tx
                )
                .is_err()
        );
    }
    assert!(
        !fake
            .logged_methods()
            .iter()
            .any(|method| method == "thread/start" || method == "turn/start")
    );
}
