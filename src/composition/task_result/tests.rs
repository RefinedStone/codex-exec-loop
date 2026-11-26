use super::*;
use crate::application::port::outbound::planning_authority_port::PlanningAuthorityDistributorQueueRecord;
use crate::domain::parallel_mode::{
    ParallelModeAgentSessionDetailSnapshot, PrValidationCommitSha, PrValidationEvent,
    PrValidationFinding, PrValidationFindingKey, PrValidationFindingSource,
    PrValidationRemediationCorrelation, PrValidationTarget, PrValidationTargetShaSnapshot,
};
use serde_json::json;

fn record(key: &str, number: u64) -> PrValidationRecord {
    PrValidationRecord::register(
        PrValidationRecordKey::new(key).unwrap(),
        PrValidationTarget::new("synthetic/repository", number).unwrap(),
        PrValidationTargetShaSnapshot::new(
            PrValidationCommitSha::new("a".repeat(40)).unwrap(),
            PrValidationCommitSha::new("b".repeat(40)).unwrap(),
        ),
    )
}

fn source(key: &str, task: &str) -> PlanningValidationSource {
    PlanningValidationSource {
        validation_record_key: key.into(),
        source_task_id: task.into(),
        work_order_id: None,
    }
}

#[test]
fn immutable_source_and_remediation_ids_never_join_sibling_tasks() {
    let record = record("record-a", 42);
    assert_eq!(
        relation(&record, &source("record-a", "task-a"), "task-a"),
        Some(TaskResultPrRelation::Source)
    );
    assert_eq!(
        relation(&record, &source("record-a", "task-a"), "same-title-task"),
        None
    );
    let finding_key = PrValidationFindingKey::new(
        PrValidationFindingSource::new("ci").unwrap(),
        "failing-check",
    )
    .unwrap();
    let record = record
        .transition(PrValidationEvent::BeginPreMergeObservation)
        .unwrap()
        .transition(PrValidationEvent::FindingObserved(
            PrValidationFinding::new(
                finding_key.clone(),
                PrValidationCommitSha::new("a".repeat(40)).unwrap(),
                "Repair check",
            )
            .unwrap(),
        ))
        .unwrap()
        .transition(PrValidationEvent::RemediationQueued(
            PrValidationRemediationCorrelation::new(
                finding_key,
                PrValidationRecordKey::new("supplemental-task").unwrap(),
            ),
        ))
        .unwrap();
    assert_eq!(
        relation(&record, &source("record-a", "task-a"), "supplemental-task"),
        Some(TaskResultPrRelation::Remediation)
    );
    assert_eq!(
        relation(&record, &source("record-a", "task-a"), "other-supplemental"),
        None
    );
}

#[test]
fn verified_legacy_source_is_distinct_from_wrong_work_order() {
    let mut source = source("record-a", "task-a");
    assert!(source_matches_order(&source, None));
    source.work_order_id = Some("work-a".into());
    assert!(!source_matches_order(&source, None));
    let mut order = PlanningWorkOrderRecord {
        work_order_id: "work-a".into(),
        origin: "immutable".into(),
        link_reference: None,
        task_ids: vec!["task-a".into(), "supplemental-task".into()],
        permits: vec![],
        acceptance_receipt: None,
    };
    assert!(source_matches_order(&source, Some(&order)));
    order.work_order_id = "work-b".into();
    assert!(!source_matches_order(&source, Some(&order)));
    order.work_order_id = "work-a".into();
    order.task_ids = vec!["other-task".into()];
    assert!(!source_matches_order(&source, Some(&order)));
}

fn session(task: &str, branch: &str) -> ParallelModeAgentSessionDetailSnapshot {
    ParallelModeAgentSessionDetailSnapshot {
        session_key: format!("session-{task}-{branch}"),
        agent_id: "agent".into(),
        task_id: task.into(),
        task_title: "Identical title".into(),
        slot_id: "slot".into(),
        thread_id: None,
        worktree_path: "/not-exposed".into(),
        branch_name: branch.into(),
        lease_started_at: "2026-11-25T00:00:00Z".into(),
        state_label: "finished".into(),
        completion_state_label: "completed".into(),
        latest_summary: String::new(),
        validation_summary: "worker report".into(),
        authority_refresh_outcome: "ok".into(),
        distributor_outcome: None,
        history: vec![],
        updated_at: "2026-11-25T00:00:00Z".into(),
    }
}

fn delivery(task: &str, key: &str) -> PlanningAuthorityDistributorQueueRecord {
    serde_json::from_value(json!({
        "queue_item_id":key,"session_key":key,"agent_id":"agent","task_id":task,
        "task_title":"Identical title","branch_name":"renamed-branch","worktree_path":"/not-exposed",
        "commit_sha":"a".repeat(40),"validation_summary":"","authority_refresh_outcome":"",
        "queue_state":"blocked","integration_note":"awaiting evidence",
        "enqueued_at":"2026-11-25T00:00:00Z","updated_at":"2026-11-25T00:00:00Z"
    })).unwrap()
}

#[test]
fn runtime_filters_only_exact_task_ids_after_branch_changes() {
    let runtime = PlanningAuthorityRuntimeProjectionSnapshot {
        slot_leases: Default::default(),
        invalid_slot_leases: Default::default(),
        session_details: vec![
            session("task-a", "before"),
            session("task-a", "after"),
            session("task-b", "after"),
        ],
        task_dispatch_blocks: vec![],
        distributor_queue_records: vec![
            delivery("task-b", "queue-b"),
            delivery("task-a", "queue-a"),
        ],
        dispatch_commands: vec![],
        runtime_events: vec![],
    };
    let mut result = TaskResultState::unavailable("task-a".into());
    project_runtime(&mut result, &runtime);
    assert_eq!(result.sessions.len(), 2);
    assert_eq!(result.sessions[0].branch, "before");
    assert_eq!(result.sessions[1].branch, "after");
    assert_eq!(result.deliveries.len(), 1);
    assert_eq!(result.deliveries[0].queue_item_id, "queue-a");
    assert_eq!(
        result.deliveries[0].note.as_deref(),
        Some("awaiting evidence")
    );
}

#[test]
fn pr_projection_keeps_multiple_records_and_merge_facts_separate() {
    let first = record("first", 42)
        .transition(PrValidationEvent::BeginPreMergeObservation)
        .unwrap()
        .transition(PrValidationEvent::MergeObserved(
            PrValidationCommitSha::new("c".repeat(40)).unwrap(),
        ))
        .unwrap();
    let second = record("second", 43);
    let projected = project_pr(
        &first,
        &source("first", "task-a"),
        TaskResultPrRelation::Source,
        Some(first.key().clone()),
    );
    assert_eq!(projected.number, 42);
    assert_eq!(
        projected.merge_sha.as_deref(),
        Some("cccccccccccccccccccccccccccccccccccccccc")
    );
    let unresolved = project_pr(
        &second,
        &source("second", "task-a"),
        TaskResultPrRelation::Source,
        None,
    );
    assert_eq!(unresolved.number, 43);
    assert_eq!(unresolved.merge_sha, None);
    assert_eq!(unresolved.current, None);
    let archived = project_pr(
        &second,
        &source("second", "task-a"),
        TaskResultPrRelation::Source,
        Some(PrValidationRecordKey::new("successor").unwrap()),
    );
    assert_eq!(archived.current, Some(false));
    assert_eq!(archived.successor_record_id.as_deref(), Some("successor"));
}
