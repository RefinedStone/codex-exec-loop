//! 명령 전용 진입점의 확인·잠금·원문 보호 경계다. 실행용 모델은 만들지 않는다.
use super::*;
use crate::domain::issue_work::{IssueProjectScope, IssueProvider, IssueReference};

fn input(kernel: &CommandOnlyKernel, kind: &str) -> Value {
    let issue = IssueReference {
        provider: IssueProvider::GitHub,
        server_id: "github.com".into(),
        repository_id: "R_intervention".into(),
        issue_id: "I_intervention".into(),
        project_scope: IssueProjectScope {
            workspace_dir: kernel.workspace.clone(),
        },
    };
    json!({
        "type": kind,
        "issue": crate::domain::kernel_event::IssueEventReference::from(&issue),
        "command_id": "offline-intervention",
    })
}

#[test]
fn offline_intervention_cancel_without_confirm_changes_no_planning_or_audit_state() {
    let (_directory, kernel) = fixture();
    let revision = planning_revision(&kernel);
    for confirm in [None, Some(false)] {
        let mut command = input(&kernel, "cancel_issue_work");
        if let Some(confirm) = confirm {
            command["confirm"] = json!(confirm);
        }
        let error = kernel
            .command(KernelCommandKind::CancelIssueWork, command, None)
            .unwrap_err();
        let error = error
            .downcast_ref::<KernelCallError>()
            .expect("kernel confirmation error");
        assert!(error.message.contains("confirm"), "{}", error.message);
    }
    assert_eq!(planning_revision(&kernel), revision);
    assert!(
        kernel
            .records(crate::composition::production::kernel_event_log_port().as_ref())
            .unwrap()
            .is_empty()
    );
}

#[test]
fn offline_intervention_does_not_bypass_a_running_tui_kernel() {
    let (_directory, kernel) = fixture();
    let location =
        SqlitePlanningAuthorityAdapter::prepare_private_runtime_directory(&kernel.workspace)
            .unwrap();
    let _lock = RepositoryKernelLockAdapter::acquire_for_owner(
        location,
        &kernel.workspace,
        RepositoryKernelOwner::Tui,
    )
    .unwrap();
    let result = kernel
        .command(
            KernelCommandKind::HoldIssue,
            input(&kernel, "hold_issue"),
            None,
        )
        .unwrap();
    assert_eq!(
        result["rejection"]["reason"], "kernel_unreachable",
        "{result}"
    );
    assert!(
        kernel
            .records(crate::composition::production::kernel_event_log_port().as_ref())
            .unwrap()
            .is_empty()
    );
}

#[test]
fn offline_intervention_decoder_does_not_echo_guidance_even_when_the_kind_is_mismatched() {
    let (_directory, kernel) = fixture();
    let mut command = input(&kernel, "steer_issue");
    command["message"] = json!("raw-offline-steer-canary");
    command["raw-offline-steer-canary"] = json!("unexpected field");
    for kind in [KernelCommandKind::SteerIssue, KernelCommandKind::HoldIssue] {
        let error = kernel.command(kind, command.clone(), None).unwrap_err();
        assert_eq!(error.to_string(), "invalid issue steer parameters");
        assert!(!error.to_string().contains("raw-offline-steer-canary"));
    }
}
