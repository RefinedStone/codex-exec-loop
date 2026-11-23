use super::*;

#[test]
fn command_only_rejects_before_resolving_or_creating_the_workspace() {
    let path = crate::test_utils::platform_safe_temp_dir().join(format!(
        "host-verification-command-only-{}-{:016x}",
        std::process::id(),
        rand::random::<u64>(),
    ));
    assert!(!path.exists());
    let kernel = CommandOnlyKernel {
        workspace: path.to_string_lossy().into_owned(),
        caller: CALLER_CLI,
    };
    let response = kernel
        .command(
            KernelCommandKind::RequestTaskVerification,
            json!({
                "type": "request_task_verification",
                "request_id": "no-runtime",
                "task_id": "task-1",
                "criterion_id": "C1",
                "scope_id": "a".repeat(64),
                "expected_acceptance_revision": 1
            }),
            Some("no-runtime".into()),
        )
        .unwrap();
    assert_eq!(response["rejection"]["reason"], "kernel_unreachable");
    assert!(response["operation"].is_null());
    assert!(response["result"].is_null());
    assert!(!path.exists());
}

#[test]
fn binding_pause_requires_a_live_owner_without_creating_workspace_or_receipt() {
    let path = crate::test_utils::platform_safe_temp_dir().join(format!(
        "binding-pause-command-only-{:016x}",
        rand::random::<u64>()
    ));
    let kernel = CommandOnlyKernel {
        workspace: path.to_string_lossy().into_owned(),
        caller: CALLER_CLI,
    };
    for paused in [true, false] {
        let response = kernel.command(KernelCommandKind::SetIssueBindingPaused, json!({
            "type":"set_issue_binding_paused", "command_id":"pause-no-owner", "binding_id":"binding-1",
            "paused":paused, "expected_revision":0
        }), Some("pause-no-owner".into())).unwrap();
        assert_eq!(response["rejection"]["reason"], "kernel_unreachable");
        assert!(response["operation"].is_null());
        assert!(response["result"].is_null());
        assert!(!path.exists());
    }
}
