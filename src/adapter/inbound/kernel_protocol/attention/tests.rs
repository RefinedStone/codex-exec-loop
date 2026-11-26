use serde_json::json;

use super::*;
use crate::domain::conversation::ConversationApprovalRequestKind;
use crate::domain::parallel_mode::{PrValidationPhase, PrValidationRecoveryAction};
use crate::domain::planning::TaskStatus;

fn thread(thread_id: &str, approval_id: Option<&str>) -> ThreadActivitySummary {
    ThreadActivitySummary {
        thread_id: thread_id.to_string(),
        running: true,
        active_turn_id: None,
        approval: approval_id.map(|approval_id| ConversationApprovalRequest {
            approval_id: approval_id.to_string(),
            server_request_id: "request-1".to_string(),
            method: "item/commandExecution/requestApproval".to_string(),
            kind: ConversationApprovalRequestKind::CommandExecution,
            summary: "Run cargo test".to_string(),
            details: Vec::new(),
        }),
        auto_follow_active: false,
    }
}

fn proposed_task(task_id: &str, updated_at: &str) -> PriorityQueueTask {
    PriorityQueueTask {
        rank: 1,
        task_id: task_id.to_string(),
        direction_id: "direction-1".to_string(),
        direction_title: "Direction".to_string(),
        task_title: "Add the attention query".to_string(),
        status: TaskStatus::Proposed,
        combined_priority: 0,
        updated_at: updated_at.to_string(),
        rank_reasons: Vec::new(),
    }
}

fn pr_validation(
    state: PrValidationOperatorState,
    blocker: Option<&str>,
) -> PrValidationOperatorSummary {
    PrValidationOperatorSummary {
        akra_id: "akra-unit-42".to_string(),
        canonical_pr_url: "https://github.com/acme/widgets/pull/42".to_string(),
        pull_request_number: 42,
        state,
        phase: PrValidationPhase::Blocked,
        target_short_sha: "aaaaaaaaaaaa".to_string(),
        integration_method: None,
        evidence_short_sha: None,
        merge_short_sha: None,
        finding_count: 0,
        remediation_count: 0,
        reason: "pull request was closed".to_string(),
        blocker: blocker.map(str::to_string),
        recovery: PrValidationRecoveryAction::ReopenOrReplacePullRequest,
        recovery_action: "reopen or replace the pull request".to_string(),
        correlations: Vec::new(),
        observation_revision: 7,
        post_merge_checkpoint_observed: false,
    }
}

#[test]
fn nothing_to_answer_is_an_empty_list() {
    assert!(attention_items(&[thread("thread-1", None)], None, &[]).is_empty());
}

#[test]
fn a_pending_approval_is_a_thread_item_keyed_by_its_approval() {
    let items = attention_items(&[thread("thread-1", Some("approval-9"))], None, &[]);

    assert_eq!(
        serde_json::to_value(&items).expect("items should serialize"),
        json!([{
            "id": "thread:thread-1:approval-9",
            "cause": "approval_pending",
            "subject": {"kind": "thread", "id": "thread-1"},
            "revision": "approval-9",
            "summary": "Run cargo test",
            "next": [{"step": "query", "name": "thread_activity"}, {"step": "ask_user"}, {"step": "command", "name": "decide_approval"}],
        }])
    );
}

#[test]
fn a_proposed_task_carries_its_update_time_as_revision_and_time() {
    let items = attention_items(
        &[],
        None,
        &[proposed_task("task-1", "2026-11-26T01:02:03Z")],
    );

    assert_eq!(items.len(), 1);
    assert_eq!(items[0].cause, AttentionCause::TaskProposed);
    assert_eq!(items[0].id, "task:task-1:2026-11-26T01:02:03Z");
    assert_eq!(items[0].updated_at.as_deref(), Some("2026-11-26T01:02:03Z"));
    assert_eq!(items[0].summary, "Add the attention query");
}

#[test]
fn a_changed_source_revision_changes_the_item_id() {
    let before = attention_items(
        &[],
        None,
        &[proposed_task("task-1", "2026-11-26T01:02:03Z")],
    );
    let again = attention_items(
        &[],
        None,
        &[proposed_task("task-1", "2026-11-26T01:02:03Z")],
    );
    let after = attention_items(
        &[],
        None,
        &[proposed_task("task-1", "2026-11-26T05:00:00Z")],
    );

    assert_eq!(before[0].id, again[0].id);
    assert_ne!(before[0].id, after[0].id);
}

#[test]
fn only_pull_request_states_that_need_a_person_become_items() {
    let item = |state| {
        attention_items(
            &[],
            Some(&pr_validation(state, Some("pull request closed"))),
            &[],
        )
    };

    let blocked = item(PrValidationOperatorState::Blocked);
    assert_eq!(blocked.len(), 1);
    assert_eq!(blocked[0].cause, AttentionCause::PrValidationBlocked);
    assert_eq!(blocked[0].id, "pull_request:42:7");
    assert_eq!(blocked[0].summary, "pull request closed");
    assert_eq!(blocked[0].updated_at, None);
    assert_eq!(
        item(PrValidationOperatorState::Failed)[0].cause,
        AttentionCause::PrValidationFailed
    );
    for state in [
        PrValidationOperatorState::Pending,
        PrValidationOperatorState::Blocking,
        PrValidationOperatorState::Remediation,
        PrValidationOperatorState::Terminal,
    ] {
        assert!(item(state).is_empty(), "{state:?} is handled by the kernel");
    }
}

#[test]
fn a_pull_request_without_a_blocker_uses_its_reason() {
    let items = attention_items(
        &[],
        Some(&pr_validation(PrValidationOperatorState::Failed, None)),
        &[],
    );

    assert_eq!(items[0].summary, "pull request was closed");
}

#[test]
fn items_are_ordered_approval_then_pull_request_then_task() {
    let items = attention_items(
        &[thread("thread-1", Some("approval-9"))],
        Some(&pr_validation(PrValidationOperatorState::Blocked, None)),
        &[proposed_task("task-1", "2026-11-26T01:02:03Z")],
    );

    let causes: Vec<_> = items.iter().map(|item| item.cause).collect();
    assert_eq!(
        causes,
        [
            AttentionCause::ApprovalPending,
            AttentionCause::PrValidationBlocked,
            AttentionCause::TaskProposed,
        ]
    );
}
