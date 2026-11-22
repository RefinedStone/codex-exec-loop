use super::*;
use crate::application::port::outbound::planning_authority_port::PlanningAuthorityDistributorQueueRecord;
use crate::domain::parallel_mode::{
    ParallelModeDeliveryTargetSnapshot, ParallelModeQueueItemState,
    ParallelModeRepositoryVisibility, ParallelModeSlotLeaseSnapshot, ParallelModeSlotLeaseState,
};
use crate::domain::planning::{
    PriorityQueueProjection, TaskActor, TaskAuthorityDocument, TaskDefinition,
};
use std::cell::Cell;

const STAMP: &str = "2026-11-21T00:00:00Z";

fn basis() -> PlanningWorkPermitBasis {
    PlanningWorkPermitBasis {
        authorization_epoch: 3,
        policy_version: "p".repeat(64),
        field_hash: format!("v1:{}", "f".repeat(64)),
        gate_revision: 7,
        source_sha: "a".repeat(40),
    }
}

fn tasks(status: TaskStatus) -> PlanningTaskAuthoritySnapshot {
    PlanningTaskAuthoritySnapshot {
        planning_revision: 1,
        task_authority: TaskAuthorityDocument {
            version: 1,
            tasks: vec![TaskDefinition {
                id: "task-1".into(),
                direction_id: "direction-1".into(),
                direction_relation_note: String::new(),
                title: "Issue work".into(),
                description: "Approved issue work".into(),
                status,
                base_priority: 1,
                dynamic_priority_delta: 0,
                priority_reason: String::new(),
                depends_on: Vec::new(),
                blocked_by: Vec::new(),
                created_by: TaskActor::User,
                last_updated_by: TaskActor::User,
                source_turn_id: None,
                provenance: Default::default(),
                updated_at: STAMP.into(),
            }],
        },
        queue_projection: PriorityQueueProjection {
            next_task: None,
            active_tasks: Vec::new(),
            proposed_tasks: Vec::new(),
            skipped_tasks: Vec::new(),
        },
    }
}

fn lease(state: ParallelModeSlotLeaseState) -> ParallelModeSlotLeaseSnapshot {
    let mut lease = ParallelModeSlotLeaseSnapshot::new(
        "slot-1",
        "task-1",
        "Issue work",
        "agent-1",
        "akra-agent/slot-1/work",
        "/work/pool/slot-1",
        state,
        STAMP,
        (state == ParallelModeSlotLeaseState::Running).then(|| STAMP.into()),
    );
    lease.delivery_target = Some(ParallelModeDeliveryTargetSnapshot::new(
        "origin",
        "owner/repo",
        ParallelModeRepositoryVisibility::Private,
        "prerelease",
        "a".repeat(40),
    ));
    lease
}

fn delivery() -> PlanningAuthorityDistributorQueueRecord {
    let lease = lease(ParallelModeSlotLeaseState::Running);
    serde_json::from_value(serde_json::json!({
        "queue_item_id": "delivery-1",
        "session_key": lease.session_key(), "slot_id": lease.slot_id,
        "agent_id": lease.agent_id, "task_id": lease.task_id, "task_title": lease.task_title,
        "branch_name": lease.branch_name, "worktree_path": lease.worktree_path,
        "commit_sha": "b".repeat(40), "source_commit_sha": "b".repeat(40),
        "source_base_commit_sha": "a".repeat(40), "validation_summary": "passed",
        "authority_refresh_outcome": "accepted", "queue_state": ParallelModeQueueItemState::Queued,
        "integration_state": "pending", "integration_note": "", "enqueued_at": STAMP,
        "updated_at": STAMP, "pull_request_number": 2548,
    }))
    .unwrap()
}

fn no_fetch() -> Result<String> {
    panic!("assigned, running, or frozen work must preserve its actual source")
}

#[test]
fn saved_dispatch_basis_does_not_hide_a_changed_pre_dispatch_source() {
    for status in [TaskStatus::Ready, TaskStatus::Proposed] {
        let calls = Cell::new(0);
        let result = execution_snapshot(
            vec!["task-1".into()],
            Some(&basis()),
            &PlanningAuthorityRuntimeProjectionSnapshot::default(),
            || Ok(Some(tasks(status))),
            || {
                calls.set(calls.get() + 1);
                Ok("c".repeat(40))
            },
        )
        .unwrap();
        assert_eq!(calls.get(), 1);
        assert_eq!(result.task_ids, ["task-1"]);
        assert_eq!(result.dispatch_source, Some("c".repeat(40)));
        assert_eq!(result.saved_source, Some("a".repeat(40)));
        assert_eq!(result.saved_gate_revision, 7);
        assert_eq!(result.saved_authorization_epoch, 3);
        assert!(result.frozen_source.is_none());
    }
}

#[test]
fn assigned_work_never_fetches_a_new_dispatch_base_even_before_running() {
    for state in [
        ParallelModeSlotLeaseState::Leased,
        ParallelModeSlotLeaseState::Running,
    ] {
        let mut runtime = PlanningAuthorityRuntimeProjectionSnapshot::default();
        runtime.slot_leases.insert("slot-1".into(), lease(state));
        let result = execution_snapshot(
            vec!["task-1".into()],
            Some(&basis()),
            &runtime,
            || Ok(Some(tasks(TaskStatus::Ready))),
            no_fetch,
        )
        .unwrap();
        assert!(result.dispatch_source.is_none());
        assert!(result.frozen_source.is_none());
        assert_eq!(result.saved_source, Some("a".repeat(40)));
        assert_eq!(result.saved_gate_revision, 7);
        assert_eq!(result.saved_authorization_epoch, 3);
    }
}

#[test]
fn frozen_delivery_source_is_preserved_without_a_live_lease() {
    let runtime = PlanningAuthorityRuntimeProjectionSnapshot {
        distributor_queue_records: vec![delivery()],
        ..Default::default()
    };
    let result = execution_snapshot(
        vec!["task-1".into()],
        Some(&basis()),
        &runtime,
        || Ok(Some(tasks(TaskStatus::Ready))),
        no_fetch,
    )
    .unwrap();
    assert_eq!(result.frozen_source, Some("b".repeat(40)));
    assert_eq!(result.saved_source, Some("a".repeat(40)));
    assert!(result.dispatch_source.is_none());
}

#[test]
fn non_dispatchable_task_states_do_not_fetch_a_new_base() {
    for status in [
        TaskStatus::Blocked,
        TaskStatus::InProgress,
        TaskStatus::Done,
        TaskStatus::Cancelled,
        TaskStatus::AwaitingUser,
    ] {
        let result = execution_snapshot(
            vec!["task-1".into()],
            Some(&basis()),
            &PlanningAuthorityRuntimeProjectionSnapshot::default(),
            || Ok(Some(tasks(status))),
            no_fetch,
        )
        .unwrap();
        assert!(result.dispatch_source.is_none());
        assert_eq!(result.saved_source, Some("a".repeat(40)));
    }
}

#[test]
fn unregistered_work_reads_the_dispatch_source_without_task_authority() {
    let calls = Cell::new(0);
    let result = execution_snapshot(
        Vec::new(),
        None,
        &PlanningAuthorityRuntimeProjectionSnapshot::default(),
        || panic!("unregistered work has no task authority to inspect"),
        || {
            calls.set(calls.get() + 1);
            Ok("a".repeat(40))
        },
    )
    .unwrap();
    assert_eq!(calls.get(), 1);
    assert_eq!(result.dispatch_source, Some("a".repeat(40)));
    assert!(result.saved_source.is_none());
    assert_eq!(result.saved_authorization_epoch, 0);
}

#[test]
fn missing_owned_task_fails_closed_without_fetching() {
    for missing_document in [false, true] {
        let result = execution_snapshot(
            vec!["task-1".into()],
            Some(&basis()),
            &PlanningAuthorityRuntimeProjectionSnapshot::default(),
            || {
                if missing_document {
                    Ok(None)
                } else {
                    let mut tasks = tasks(TaskStatus::Ready);
                    tasks.task_authority.tasks[0].id = "unrelated-task".into();
                    Ok(Some(tasks))
                }
            },
            no_fetch,
        );
        assert!(result.is_err());
    }
}
