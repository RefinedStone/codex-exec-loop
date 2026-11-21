use super::*;
use std::collections::{BTreeMap, BTreeSet};

use crate::application::port::outbound::planning_authority_port::PlanningAuthorityRuntimeProjectionSnapshot;
use crate::domain::issue_automation::execution::{
    IssueExecutionPhase, IssueExecutionQuestion, IssueWorkExecutionObservation,
};
use crate::domain::parallel_mode::{
    ParallelModeLiveSessionDetailDefaults, ParallelModeSlotLeaseState,
};

const STAMP: &str = "2026-11-21T10:00:00Z";

fn lease(task_id: &str, slot_id: &str, generation: char) -> ParallelModeSlotLeaseSnapshot {
    ParallelModeSlotLeaseSnapshot::new(
        slot_id,
        task_id,
        task_id,
        "agent",
        "akra-agent/work",
        "/work/pool/slot",
        ParallelModeSlotLeaseState::Running,
        STAMP,
        Some(STAMP.into()),
    )
    .with_lease_generation(generation.to_string().repeat(64))
}

fn detail(
    lease: &ParallelModeSlotLeaseSnapshot,
    thread_id: Option<&str>,
) -> ParallelModeAgentSessionDetailSnapshot {
    let mut detail = ParallelModeAgentSessionDetailSnapshot::assigned_for_lease(
        lease,
        ParallelModeLiveSessionDetailDefaults {
            validation_summary: "pending",
            authority_refresh_outcome: "pending",
        },
    );
    detail.thread_id = thread_id.map(str::to_owned);
    detail
}

fn state(status: TaskStatus) -> ObservedExecution {
    ObservedExecution {
        observation: IssueWorkExecutionObservation {
            phase: IssueExecutionPhase::Running,
            task_ids: vec!["past".into(), "current".into()],
            active_task_id: Some("current".into()),
            ..IssueWorkExecutionObservation::default()
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

fn with_lease(status: TaskStatus) -> (ObservedExecution, ParallelModeSlotLeaseSnapshot) {
    let mut current = state(status);
    let lease = lease("current", "slot-b", 'b');
    current
        .runtime
        .slot_leases
        .insert(lease.slot_id.clone(), lease.clone());
    current
        .runtime
        .session_details
        .push(detail(&lease, Some("current-thread")));
    (current, lease)
}

fn question(lease: &ParallelModeSlotLeaseSnapshot) -> IssueExecutionQuestion {
    IssueExecutionQuestion {
        question_id: "question-current".into(),
        task_id: lease.task_id.clone(),
        session_key: lease.session_key(),
        thread_id: "current-thread".into(),
        slot_id: lease.slot_id.clone(),
        worktree_path: lease.worktree_path.clone(),
        branch_name: lease.branch_name.clone(),
        lease_generation: lease.lease_generation.clone().unwrap(),
        awaiting_task_updated_at: STAMP.into(),
    }
}

#[test]
fn current_task_conversation_ignores_previous_tasks_and_old_generations() {
    let (mut current, _) = with_lease(TaskStatus::InProgress);
    current.runtime.session_details.push(detail(
        &lease("past", "slot-a", 'a'),
        Some("old-task-thread"),
    ));
    current.runtime.session_details.push(detail(
        &lease("current", "slot-b", 'a'),
        Some("old-generation-thread"),
    ));
    current.runtime.session_details.push(detail(
        &lease("past", "slot-invalid", 'c'),
        Some(" invalid-old-id"),
    ));
    assert_eq!(select(&current).unwrap().as_deref(), Some("current-thread"));
}

#[test]
fn a_fresh_question_must_still_match_the_exact_current_session() {
    let (mut current, lease) = with_lease(TaskStatus::AwaitingUser);
    current.observation.question = Some(question(&lease));
    assert_eq!(select(&current).unwrap().as_deref(), Some("current-thread"));
    for field in 0..7 {
        let mut changed = question(&lease);
        match field {
            0 => changed.thread_id.push_str("-other"),
            1 => changed.session_key.push_str("-other"),
            2 => changed.slot_id.push_str("-other"),
            3 => changed.branch_name.push_str("-other"),
            4 => changed.worktree_path.push_str("-other"),
            5 => changed.lease_generation = "a".repeat(64),
            _ => changed.task_id = "past".into(),
        }
        current.observation.question = Some(changed);
        assert!(select(&current).is_err(), "question field {field}");
    }
}

#[test]
fn multiple_current_leases_or_an_ambiguous_current_task_never_choose_a_thread() {
    let (mut current, _) = with_lease(TaskStatus::InProgress);
    let second = lease("current", "slot-c", 'c');
    current
        .runtime
        .slot_leases
        .insert(second.slot_id.clone(), second);
    assert!(select(&current).is_err());
    current.runtime.slot_leases.clear();
    current.observation.active_task_id = None;
    assert!(select(&current).is_err());
}

#[test]
fn a_current_session_coordinate_mismatch_cannot_fall_back_to_history() {
    for field in 0..7 {
        let (mut current, _) = with_lease(TaskStatus::InProgress);
        let candidate = &mut current.runtime.session_details[0];
        match field {
            0 => candidate.task_id = "past".into(),
            1 => candidate.slot_id.push_str("-other"),
            2 => candidate.agent_id.push_str("-other"),
            3 => candidate.worktree_path.push_str("-other"),
            4 => candidate.branch_name.push_str("-other"),
            5 => candidate.lease_started_at.push_str("-other"),
            _ => candidate.thread_id = Some("bad thread".into()),
        }
        current.runtime.session_details.push(detail(
            &lease("current", "old-slot", 'a'),
            Some("old-thread"),
        ));
        assert!(select(&current).is_err(), "session field {field}");
    }
    let (mut current, _) = with_lease(TaskStatus::InProgress);
    current
        .runtime
        .session_details
        .push(current.runtime.session_details[0].clone());
    assert!(select(&current).is_err());
}

#[test]
fn cleaned_up_completed_or_cancelled_work_uses_only_its_current_task_thread() {
    for status in [TaskStatus::Done, TaskStatus::Cancelled] {
        let mut current = state(status);
        current
            .runtime
            .session_details
            .push(detail(&lease("past", "slot-a", 'a'), Some("past-thread")));
        current.runtime.session_details.push(detail(
            &lease("current", "slot-b", 'b'),
            Some("current-thread"),
        ));
        current.runtime.session_details.push(detail(
            &lease("current", "slot-b", 'c'),
            Some("current-thread"),
        ));
        assert_eq!(select(&current).unwrap().as_deref(), Some("current-thread"));
        current.runtime.session_details.push(detail(
            &lease("current", "slot-b", 'd'),
            Some("another-current-task-thread"),
        ));
        assert!(select(&current).is_err());
    }
}

#[test]
fn queued_tasks_without_a_current_lease_offer_a_draft_instead_of_a_past_thread() {
    for status in [TaskStatus::Ready, TaskStatus::Proposed] {
        let mut current = state(status);
        current.runtime.session_details.push(detail(
            &lease("current", "slot-b", 'b'),
            Some("past-thread"),
        ));
        current.runtime.session_details.push(detail(
            &lease("current", "slot-b", 'c'),
            Some("other-past-thread"),
        ));
        assert_eq!(select(&current).unwrap(), None);
    }
}

#[test]
fn missing_active_lease_never_reuses_a_running_or_waiting_tasks_history() {
    for status in [
        TaskStatus::InProgress,
        TaskStatus::AwaitingUser,
        TaskStatus::Blocked,
    ] {
        let mut current = state(status);
        current.runtime.session_details.push(detail(
            &lease("current", "slot-b", 'b'),
            Some("past-thread"),
        ));
        assert!(select(&current).is_err());
    }
}

#[test]
fn an_unknown_continuation_can_be_inspected_without_offering_another_question() {
    let (mut current, _) = with_lease(TaskStatus::AwaitingUser);
    current.runtime.session_details[0].state_label = "continuation_unknown".into();
    current.runtime.session_details[0].completion_state_label = "continuation_unknown".into();
    assert!(current.observation.question.is_none());
    assert_eq!(select(&current).unwrap().as_deref(), Some("current-thread"));
}

#[test]
fn an_unprepared_current_thread_does_not_reuse_an_old_generation() {
    let (mut current, _) = with_lease(TaskStatus::InProgress);
    current.runtime.session_details[0].thread_id = None;
    current
        .runtime
        .session_details
        .push(detail(&lease("current", "slot-b", 'a'), Some("old-thread")));
    assert_eq!(select(&current).unwrap(), None);
}
