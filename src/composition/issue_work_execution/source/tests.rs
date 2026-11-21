use super::*;
use crate::domain::parallel_mode::{
    ParallelModeQueueItemState, ParallelModeSlotLeaseSnapshot, ParallelModeSlotLeaseState,
};
use std::cell::Cell;
use std::collections::{BTreeMap, BTreeSet};

const STAMP: &str = "2026-11-21T00:00:00Z";

fn observed(
    task_ids: &[&str],
    active: &str,
    phase: IssueExecutionPhase,
    status: TaskStatus,
) -> observation::ObservedExecution {
    observation::ObservedExecution {
        observation: IssueWorkExecutionObservation {
            task_ids: task_ids.iter().map(|id| (*id).into()).collect(),
            active_task_id: Some(active.into()),
            phase,
            ..Default::default()
        },
        runtime: PlanningAuthorityRuntimeProjectionSnapshot {
            slot_leases: BTreeMap::new(),
            invalid_slot_leases: BTreeSet::new(),
            session_details: Vec::new(),
            task_dispatch_blocks: Vec::new(),
            distributor_queue_records: Vec::new(),
            dispatch_commands: Vec::new(),
            runtime_events: Vec::new(),
        },
        active_task_status: Some(status),
    }
}

fn basis(source: char) -> PlanningWorkPermitBasis {
    PlanningWorkPermitBasis {
        authorization_epoch: 3,
        policy_version: "p".repeat(64),
        field_hash: format!("v1:{}", "f".repeat(64)),
        gate_revision: 7,
        source_sha: source.to_string().repeat(40),
    }
}

fn lease(task: &str, generation: char) -> ParallelModeSlotLeaseSnapshot {
    ParallelModeSlotLeaseSnapshot::new(
        "slot-1",
        task,
        task,
        "agent-1",
        "akra-agent/slot-1/work",
        "/work/pool/slot-1",
        ParallelModeSlotLeaseState::Running,
        STAMP,
        Some(STAMP.into()),
    )
    .with_lease_generation(generation.to_string().repeat(64))
}

fn delivery(lease: &ParallelModeSlotLeaseSnapshot, source: char) -> crate::application::port::outbound::planning_authority_port::PlanningAuthorityDistributorQueueRecord{
    serde_json::from_value(serde_json::json!({
        "queue_item_id": format!("delivery-{}", lease.session_key()),
        "session_key": lease.session_key(), "slot_id": lease.slot_id,
        "agent_id": lease.agent_id, "task_id": lease.task_id, "task_title": lease.task_title,
        "branch_name": lease.branch_name, "worktree_path": lease.worktree_path,
        "commit_sha": source.to_string().repeat(40), "source_commit_sha": source.to_string().repeat(40),
        "source_base_commit_sha": "0".repeat(40), "validation_summary": "passed",
        "authority_refresh_outcome": "accepted", "queue_state": ParallelModeQueueItemState::Queued,
        "integration_state": "pending", "integration_note": "", "enqueued_at": STAMP,
        "updated_at": STAMP, "pull_request_number": 2551,
    })).unwrap()
}

fn no_fetch() -> Result<String> {
    panic!("assigned, frozen, or running work must preserve its actual source")
}

#[test]
fn successive_tasks_use_only_the_current_tasks_frozen_source() {
    let mut loaded = observed(
        &["initial", "supplement"],
        "supplement",
        IssueExecutionPhase::InReview,
        TaskStatus::Done,
    );
    loaded.runtime.distributor_queue_records = vec![
        delivery(&lease("initial", 'a'), 'a'),
        delivery(&lease("supplement", 'b'), 'b'),
    ];
    let result = project(Some(&loaded), Some(&basis('a')), no_fetch).unwrap();
    assert_eq!(result.frozen_source, Some("b".repeat(40)));
    assert_eq!(result.saved_source, Some("a".repeat(40)));
    assert_eq!(result.task_ids, ["initial", "supplement"]);
    assert!(result.dispatch_source.is_none());
}

#[test]
fn a_ready_supplement_fetches_the_current_base_and_ignores_parent_delivery() {
    let mut loaded = observed(
        &["initial", "supplement"],
        "supplement",
        IssueExecutionPhase::Queued,
        TaskStatus::Ready,
    );
    loaded
        .runtime
        .distributor_queue_records
        .push(delivery(&lease("initial", 'a'), 'a'));
    let calls = Cell::new(0);
    let result = project(Some(&loaded), Some(&basis('a')), || {
        calls.set(calls.get() + 1);
        Ok("c".repeat(40))
    })
    .unwrap();
    assert_eq!(calls.get(), 1);
    assert!(result.frozen_source.is_none());
    assert_eq!(result.dispatch_source, Some("c".repeat(40)));
    assert_eq!(result.saved_source, Some("a".repeat(40)));
    assert_eq!(result.saved_authorization_epoch, 3);
    assert_eq!(result.saved_gate_revision, 7);
}

#[test]
fn base_advance_before_initial_dispatch_is_rechecked_even_with_a_saved_basis() {
    for status in [TaskStatus::Ready, TaskStatus::Proposed] {
        let loaded = observed(&["initial"], "initial", IssueExecutionPhase::Queued, status);
        let calls = Cell::new(0);
        let result = project(Some(&loaded), Some(&basis('a')), || {
            calls.set(calls.get() + 1);
            Ok("b".repeat(40))
        })
        .unwrap();
        assert_eq!(calls.get(), 1);
        assert_eq!(result.task_ids, ["initial"]);
        assert_eq!(result.dispatch_source, Some("b".repeat(40)));
        assert_eq!(result.saved_source, Some("a".repeat(40)));
        assert_eq!(result.saved_gate_revision, 7);
        assert_eq!(result.saved_authorization_epoch, 3);
        assert!(result.frozen_source.is_none());
    }
}

#[test]
fn a_ready_or_proposed_task_with_a_lease_preserves_its_saved_dispatch_source() {
    for status in [TaskStatus::Ready, TaskStatus::Proposed] {
        let mut loaded = observed(&["initial"], "initial", IssueExecutionPhase::Queued, status);
        let mut current = lease("initial", 'a');
        current.state = ParallelModeSlotLeaseState::Leased;
        current.running_started_at = None;
        loaded
            .runtime
            .slot_leases
            .insert(current.slot_id.clone(), current);

        let result = project(Some(&loaded), Some(&basis('a')), no_fetch).unwrap();

        assert!(result.dispatch_source.is_none());
        assert!(result.frozen_source.is_none());
        assert_eq!(result.saved_source, Some("a".repeat(40)));
        assert_eq!(result.saved_gate_revision, 7);
        assert_eq!(result.saved_authorization_epoch, 3);
        assert_eq!(result.task_ids, ["initial"]);
    }
}

#[test]
fn a_leased_task_without_a_saved_basis_does_not_fetch_a_new_dispatch_source() {
    let mut loaded = observed(
        &["initial"],
        "initial",
        IssueExecutionPhase::Queued,
        TaskStatus::Ready,
    );
    let mut current = lease("initial", 'a');
    current.state = ParallelModeSlotLeaseState::Leased;
    current.running_started_at = None;
    loaded
        .runtime
        .slot_leases
        .insert(current.slot_id.clone(), current);

    let result = project(Some(&loaded), None, no_fetch).unwrap();

    assert!(result.dispatch_source.is_none());
    assert!(result.frozen_source.is_none());
    assert!(result.saved_source.is_none());
    assert_eq!(result.saved_gate_revision, 0);
    assert_eq!(result.saved_authorization_epoch, 0);
    assert_eq!(result.task_ids, ["initial"]);
}

#[test]
fn a_ready_supplement_fetches_its_base_with_only_an_older_tasks_lease() {
    for state in [
        ParallelModeSlotLeaseState::Running,
        ParallelModeSlotLeaseState::CleanupPending,
    ] {
        let mut loaded = observed(
            &["initial", "supplement"],
            "supplement",
            IssueExecutionPhase::Queued,
            TaskStatus::Ready,
        );
        let mut previous = lease("initial", 'a');
        previous.state = state;
        loaded
            .runtime
            .distributor_queue_records
            .push(delivery(&previous, 'a'));
        loaded
            .runtime
            .slot_leases
            .insert(previous.slot_id.clone(), previous);
        let calls = Cell::new(0);

        let result = project(Some(&loaded), Some(&basis('a')), || {
            calls.set(calls.get() + 1);
            Ok("c".repeat(40))
        })
        .unwrap();

        assert_eq!(calls.get(), 1);
        assert!(result.frozen_source.is_none());
        assert_eq!(result.dispatch_source, Some("c".repeat(40)));
        assert_eq!(result.saved_source, Some("a".repeat(40)));
        assert_eq!(result.saved_gate_revision, 7);
        assert_eq!(result.saved_authorization_epoch, 3);
        assert_eq!(result.task_ids, ["initial", "supplement"]);
    }
}

#[test]
fn a_running_task_preserves_its_saved_dispatch_source() {
    let loaded = observed(
        &["initial"],
        "initial",
        IssueExecutionPhase::Running,
        TaskStatus::InProgress,
    );
    let result = project(Some(&loaded), Some(&basis('a')), no_fetch).unwrap();
    assert!(result.dispatch_source.is_none());
    assert!(result.frozen_source.is_none());
    assert_eq!(result.saved_source, Some("a".repeat(40)));
}

#[test]
fn a_current_generation_never_reuses_a_previous_generations_delivery_source() {
    let current = lease("initial", 'b');
    let mut loaded = observed(
        &["initial"],
        "initial",
        IssueExecutionPhase::Running,
        TaskStatus::InProgress,
    );
    loaded
        .runtime
        .slot_leases
        .insert(current.slot_id.clone(), current.clone());
    loaded
        .runtime
        .distributor_queue_records
        .push(delivery(&lease("initial", 'a'), 'a'));
    let running = project(Some(&loaded), Some(&basis('b')), no_fetch).unwrap();
    assert!(running.frozen_source.is_none());
    loaded
        .runtime
        .distributor_queue_records
        .push(delivery(&current, 'b'));
    let delivered = project(Some(&loaded), Some(&basis('b')), no_fetch).unwrap();
    assert_eq!(delivered.frozen_source, Some("b".repeat(40)));
}

#[test]
fn a_frozen_single_task_source_is_preserved_after_its_lease_is_cleaned() {
    for status in [TaskStatus::Ready, TaskStatus::Done] {
        let mut loaded = observed(
            &["initial"],
            "initial",
            IssueExecutionPhase::InReview,
            status,
        );
        loaded
            .runtime
            .distributor_queue_records
            .push(delivery(&lease("initial", 'a'), 'b'));
        let result = project(Some(&loaded), Some(&basis('a')), no_fetch).unwrap();
        assert_eq!(result.frozen_source, Some("b".repeat(40)));
        assert_eq!(result.saved_source, Some("a".repeat(40)));
        assert!(result.dispatch_source.is_none());
    }
}

#[test]
fn conflicting_sources_or_lease_coordinates_fail_closed() {
    let current = lease("initial", 'a');
    let mut loaded = observed(
        &["initial"],
        "initial",
        IssueExecutionPhase::InReview,
        TaskStatus::Done,
    );
    loaded.runtime.distributor_queue_records =
        vec![delivery(&current, 'a'), delivery(&current, 'b')];
    assert!(project(Some(&loaded), Some(&basis('a')), no_fetch).is_err());
    loaded.runtime.distributor_queue_records.truncate(1);
    loaded
        .runtime
        .slot_leases
        .insert(current.slot_id.clone(), current);
    loaded.runtime.distributor_queue_records[0].worktree_path = "/other-slot".into();
    assert!(project(Some(&loaded), Some(&basis('a')), no_fetch).is_err());
}

#[test]
fn ambiguous_current_work_never_falls_back_to_a_parent_source() {
    let mut loaded = observed(
        &["initial", "supplement"],
        "supplement",
        IssueExecutionPhase::Running,
        TaskStatus::InProgress,
    );
    loaded.observation.active_task_id = None;
    assert!(project(Some(&loaded), Some(&basis('a')), no_fetch).is_err());
}

#[test]
fn unregistered_work_fetches_a_dispatch_source_once() {
    let calls = Cell::new(0);
    let result = project(None, None, || {
        calls.set(calls.get() + 1);
        Ok("a".repeat(40))
    })
    .unwrap();
    assert_eq!(calls.get(), 1);
    assert!(result.task_ids.is_empty());
    assert_eq!(result.dispatch_source, Some("a".repeat(40)));
    assert!(result.saved_source.is_none());
    assert_eq!(result.saved_gate_revision, 0);
    assert_eq!(result.saved_authorization_epoch, 0);
}

#[test]
fn a_terminal_task_without_delivery_does_not_look_like_new_dispatch() {
    for status in [
        TaskStatus::Blocked,
        TaskStatus::InProgress,
        TaskStatus::Done,
        TaskStatus::Cancelled,
        TaskStatus::AwaitingUser,
    ] {
        let loaded = observed(&["initial"], "initial", IssueExecutionPhase::Queued, status);
        let result = project(Some(&loaded), Some(&basis('a')), no_fetch).unwrap();
        assert!(result.dispatch_source.is_none());
        assert_eq!(result.saved_source, Some("a".repeat(40)));
    }
}
