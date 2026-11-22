use super::*;
use crate::domain::issue_automation::execution::{
    IssueExecutionIntegration, IssueExecutionTerminalRecord,
};
use crate::domain::issue_work::{IssueProjectScope, IssueProvider};

fn fixture() -> (IssueReference, IssueWorkSummary, IssueExecutionRecord) {
    let issue = IssueReference {
        provider: IssueProvider::GitHub,
        server_id: "github.com".into(),
        repository_id: "R_execution".into(),
        issue_id: "I_execution".into(),
        project_scope: IssueProjectScope {
            workspace_dir: "/work/execution".into(),
        },
    };
    let id = issue_work_order_id(&issue);
    let work = IssueWorkSummary {
        work_order_id: id.clone(),
        task_ids: vec!["task-1".into()],
        blocked_reason: None,
        delivery_granted: false,
    };
    let mut record = IssueExecutionRecord::new(id);
    record.progress.revision = 4;
    record.progress.state = IssueExecutionState::InReview;
    record.progress.attempts.execution = 2;
    record.progress.attempts.verification = 7;
    record.progress.attempts.publication = 99;
    (issue, work, record)
}

fn ready(detail: IssueExecutionDetail) -> IssueExecutionReadyDetail {
    match detail {
        IssueExecutionDetail::Ready(ready) => *ready,
        other => panic!("expected ready execution detail, got {other:?}"),
    }
}

#[test]
fn execution_detail_rejects_foreign_or_missing_work_identity() {
    let (issue, work, record) = fixture();
    let unavailable = IssueExecutionDetail::Unavailable {
        reason: IssueExecutionUnavailableReason::WorkIdMismatch,
    };
    let mut foreign_record = record.clone();
    foreign_record.work_order_id = "unrelated-work".into();
    assert_eq!(
        IssueExecutionDetail::from_record(&issue, Some(&work), &foreign_record, Some(3)),
        unavailable
    );
    let mut foreign_work = work.clone();
    foreign_work.work_order_id = "unrelated-work".into();
    assert_eq!(
        IssueExecutionDetail::from_record(&issue, Some(&foreign_work), &record, Some(3)),
        unavailable
    );
    assert_eq!(
        IssueExecutionDetail::from_record(&issue, None, &record, Some(3)),
        unavailable
    );
    let mut foreign_issue = issue;
    foreign_issue.repository_id = "R_other".into();
    assert_eq!(
        IssueExecutionDetail::from_record(&foreign_issue, Some(&work), &record, Some(3)),
        unavailable
    );
}

#[test]
fn execution_detail_preserves_counts_and_unknown_current_policy_limit() {
    let (issue, work, mut record) = fixture();
    record.progress.review_pull_request_url =
        Some("https://github.com/actual/review-target/pull/42".into());
    let before = record.clone();
    let detail = ready(IssueExecutionDetail::from_record(
        &issue,
        Some(&work),
        &record,
        None,
    ));
    assert_eq!(detail.state, IssueExecutionState::InReview);
    assert_eq!(detail.state_label, "리뷰 중");
    assert_eq!(detail.execution_attempts, 2);
    assert_eq!(detail.execution_attempt_limit, None);
    assert_eq!(detail.verification_attempts, 7);
    assert_eq!(
        detail.review_pull_request_url,
        record.progress.review_pull_request_url
    );
    assert!(detail.recovery.is_none());
    assert_eq!(
        record, before,
        "a display projection does not update execution progress"
    );
}

#[test]
fn execution_detail_keeps_done_and_contract_recovery_together() {
    let (issue, work, mut record) = fixture();
    record.progress.state = IssueExecutionState::Done;
    record.progress.integration = Some(IssueExecutionIntegration {
        record_id: "integration-1".into(),
        source_sha: "a".repeat(40),
        integration_sha: "b".repeat(40),
        pull_request_url: Some("https://github.com/actual/review-target/pull/42".into()),
    });
    record.progress.terminal = Some(IssueExecutionTerminalRecord {
        record_id: "contract-1".into(),
        classification: IssueExecutionTerminalClass::ContractRecovery,
    });
    let detail = ready(IssueExecutionDetail::from_record(
        &issue,
        Some(&work),
        &record,
        Some(3),
    ));
    assert_eq!(detail.state, IssueExecutionState::Done);
    assert_eq!(detail.state_label, "통합 완료");
    assert_eq!(
        detail.terminal_class,
        Some(IssueExecutionTerminalClass::ContractRecovery)
    );
    let recovery = detail.recovery.unwrap();
    assert_eq!(
        recovery.reason,
        IssueExecutionRecoveryReason::ContractRecovery
    );
    assert!(recovery.requires_human);
    assert!(!recovery.summary.is_empty());
    assert_eq!(
        recovery.next_summary,
        IssueExecutionTerminalClass::ContractRecovery.recovery_summary()
    );
}

#[test]
fn execution_detail_exhaustion_does_not_erase_the_terminal_cause() {
    let (issue, work, mut record) = fixture();
    record.progress.state = IssueExecutionState::Failed;
    record.progress.attempts_exhausted = true;
    record.progress.terminal = Some(IssueExecutionTerminalRecord {
        record_id: "rework-1".into(),
        classification: IssueExecutionTerminalClass::ReworkNeeded,
    });
    let detail = ready(IssueExecutionDetail::from_record(
        &issue,
        Some(&work),
        &record,
        Some(2),
    ));
    assert_eq!(
        detail.terminal_class,
        Some(IssueExecutionTerminalClass::ReworkNeeded)
    );
    let recovery = detail.recovery.unwrap();
    assert_eq!(
        recovery.reason,
        IssueExecutionRecoveryReason::AttemptsExhausted
    );
    assert!(recovery.requires_human);
    assert!(recovery.next_summary.contains("직접 맡기"));
}

#[test]
fn execution_detail_does_not_turn_invalid_stored_completion_into_ready() {
    let (issue, work, mut record) = fixture();
    record.progress.state = IssueExecutionState::Done;
    assert_eq!(
        IssueExecutionDetail::from_record(&issue, Some(&work), &record, Some(3)),
        IssueExecutionDetail::Unavailable {
            reason: IssueExecutionUnavailableReason::ReadFailed
        },
    );
}

#[test]
fn execution_detail_exhaustion_preserves_later_terminal_diagnosis_in_visible_summary() {
    use crate::domain::issue_automation::execution::{
        IssueExecutionPhase, IssueWorkExecutionObservation,
    };

    for (classification, summary) in [
        (
            IssueExecutionTerminalClass::PullRequestClosed,
            "PR이 통합되지 않은 채 닫혔습니다.",
        ),
        (
            IssueExecutionTerminalClass::ContractRecovery,
            "통합 증거 또는 필수 검증 설정에 복구가 필요합니다.",
        ),
    ] {
        let (issue, work, mut record) = fixture();
        let rework = IssueExecutionTerminalRecord {
            record_id: "last-allowed-rework".into(),
            classification: IssueExecutionTerminalClass::ReworkNeeded,
        };
        let exhausted = IssueWorkExecutionObservation {
            phase: IssueExecutionPhase::Terminal,
            task_ids: work.task_ids.clone(),
            active_task_id: Some("task-1".into()),
            terminals: vec![rework.clone()],
            current_terminal_id: Some(rework.record_id.clone()),
            ..Default::default()
        };
        record.progress = record.progress.reconcile(&exhausted, 0, 0, 2).unwrap();
        assert!(record.progress.attempts_exhausted);

        let later_terminal = IssueExecutionTerminalRecord {
            record_id: "later-terminal".into(),
            classification,
        };
        let observed = IssueWorkExecutionObservation {
            phase: IssueExecutionPhase::Terminal,
            task_ids: work.task_ids.clone(),
            active_task_id: Some("task-1".into()),
            terminals: vec![rework, later_terminal.clone()],
            current_terminal_id: Some(later_terminal.record_id.clone()),
            ..Default::default()
        };
        record.progress = record.progress.reconcile(&observed, 0, 0, 2).unwrap();
        record.validate().unwrap();
        assert!(record.progress.attempts_exhausted);
        let detail = ready(IssueExecutionDetail::from_record(
            &issue,
            Some(&work),
            &record,
            Some(2),
        ));
        assert_eq!(detail.state, IssueExecutionState::Failed);
        assert_eq!(detail.terminal_class, Some(classification));
        let recovery = detail.recovery.unwrap();
        assert_eq!(
            recovery.reason,
            IssueExecutionRecoveryReason::AttemptsExhausted
        );
        assert!(recovery.summary.contains(summary));
        assert!(
            recovery
                .summary
                .contains("허용된 실행 횟수를 모두 사용했습니다.")
        );
        assert!(
            recovery
                .next_summary
                .contains(classification.recovery_summary())
        );
        assert!(recovery.next_summary.contains("직접 맡기"));
        assert!(recovery.requires_human);
    }
}
