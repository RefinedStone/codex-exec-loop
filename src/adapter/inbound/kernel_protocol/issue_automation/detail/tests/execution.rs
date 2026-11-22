use super::*;
use crate::domain::issue_automation::coordinator::{IssueWorkSummary, issue_work_order_id};
use crate::domain::issue_automation::execution::*;

fn execution_detail() -> (IssueCollectionDetail, IssueExecutionRecord) {
    let issue = reference();
    let id = issue_work_order_id(&issue);
    let item = IssueCollectionItem {
        start_budget_wait: None,
        issue,
        binding_id: IssueAutomationBindingId::new("binding").unwrap(),
        tracked: None,
        current: None,
        observation_revision: None,
        tracked_revision: None,
        triage: None,
        work: Some(IssueWorkSummary {
            work_order_id: id.clone(),
            task_ids: vec!["task-1".into()],
            blocked_reason: None,
            delivery_granted: false,
        }),
        open_decisions: Vec::new(),
    };
    let mut record = IssueExecutionRecord::new(id);
    record.progress.revision = 8;
    record.progress.state = IssueExecutionState::InReview;
    record.progress.attempts.execution = 2;
    record.progress.attempts.verification = 4;
    record.progress.attempts.publication = 99;
    (detail(Some(item)), record)
}

#[test]
fn execution_wire_keeps_unknown_optional_values_null_and_absence_distinct() {
    let (mut detail, record) = execution_detail();
    let absent = serde_json::to_value(IssueState::from_domain(&detail)).unwrap();
    assert!(absent.get("execution").is_none());
    let item = detail.item.as_ref().unwrap();
    detail.execution = Some(IssueExecutionDetail::from_record(
        &item.issue,
        item.work.as_ref(),
        &record,
        None,
    ));
    let wire =
        serde_json::to_value(KernelQueryAnswer::Issue(IssueState::from_domain(&detail))).unwrap();
    let execution = &wire["execution"];
    assert_eq!(execution["status"], "ready");
    assert_eq!(execution["state_label"], "리뷰 중");
    assert_eq!(execution["execution_attempts"], 2);
    assert_eq!(execution["verification_attempts"], 4);
    for field in [
        "execution_attempt_limit",
        "review_pull_request_url",
        "terminal_class",
        "recovery",
    ] {
        assert!(
            execution.get(field).is_some_and(Value::is_null),
            "{field} must be explicitly null"
        );
    }
    assert!(execution.get("publication_attempts").is_none());
    assert!(execution.get("question").is_none());
    assert_wire_matches_spec(&wire);
    let decoded: IssueState =
        serde_json::from_value(serde_json::to_value(IssueState::from_domain(&detail)).unwrap())
            .unwrap();
    assert_eq!(decoded, IssueState::from_domain(&detail));
}

#[test]
fn execution_wire_preserves_done_recovery_and_external_open_as_independent_facts() {
    let (mut detail, mut record) = execution_detail();
    record.progress.state = IssueExecutionState::Done;
    record.progress.integration = Some(IssueExecutionIntegration {
        record_id: "integration-1".into(),
        source_sha: "a".repeat(40),
        integration_sha: "b".repeat(40),
        pull_request_url: Some("https://github.com/actual/review-target/pull/42".into()),
    });
    record.progress.terminal = Some(IssueExecutionTerminalRecord {
        record_id: "contract-recovery-1".into(),
        classification: IssueExecutionTerminalClass::ContractRecovery,
    });
    let item = detail.item.as_ref().unwrap();
    detail.execution = Some(IssueExecutionDetail::from_record(
        &item.issue,
        item.work.as_ref(),
        &record,
        Some(3),
    ));
    detail.feedback = Some(IssueFeedbackDetail {
        revision: 3,
        publication_attempts: 5,
        comment_id: None,
        reason: Some(IssueFeedbackDetailReason::FeedbackUnconfirmed),
        integration: record.progress.integration.clone(),
        external_state: IssueExternalState::Open,
    });
    let wire =
        serde_json::to_value(KernelQueryAnswer::Issue(IssueState::from_domain(&detail))).unwrap();
    assert_eq!(wire["execution"]["state"], "done");
    assert_eq!(wire["execution"]["state_label"], "통합 완료");
    assert_eq!(wire["execution"]["terminal_class"], "contract_recovery");
    assert_eq!(wire["execution"]["recovery"]["reason"], "contract_recovery");
    assert_eq!(wire["execution"]["recovery"]["requires_human"], true);
    assert!(
        wire["execution"]["recovery"]["next_summary"]
            .as_str()
            .is_some_and(|s| !s.is_empty())
    );
    assert_eq!(wire["feedback"]["external_state"], "open");
    assert_eq!(wire["feedback"]["reason"], "feedback_unconfirmed");
    assert_eq!(wire["feedback"]["publication_attempts"], 5);
    assert_eq!(
        wire["feedback"]["integration"]["record_id"],
        "integration-1"
    );
    assert_wire_matches_spec(&wire);
}

#[test]
fn execution_wire_unavailable_has_no_fabricated_state_or_counters() {
    let (mut detail, _) = execution_detail();
    for (reason, expected) in [
        (IssueExecutionUnavailableReason::ReadFailed, "read_failed"),
        (
            IssueExecutionUnavailableReason::WorkIdMismatch,
            "work_id_mismatch",
        ),
    ] {
        detail.execution = Some(IssueExecutionDetail::Unavailable { reason });
        let wire = serde_json::to_value(KernelQueryAnswer::Issue(IssueState::from_domain(&detail)))
            .unwrap();
        assert_eq!(
            wire["execution"],
            json!({"status":"unavailable","reason":expected})
        );
        assert_wire_matches_spec(&wire);
    }
}
