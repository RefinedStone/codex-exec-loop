use std::path::{Path, PathBuf};
use std::sync::{
    Mutex,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};

use crate::application::port::inbound::stored_command_port::issue_confirmation::VerifiedHumanConfirmation;
use crate::application::port::inbound::task_evidence_query_port::TaskEvidenceQueryPort;
use crate::application::port::outbound::planning_authority_port::NoopPlanningAuthorityPort;
use crate::application::port::outbound::task_evidence_repository_port::{
    StoredTaskEvidenceAttempt, TaskEvidenceStoredSnapshot,
};
use crate::domain::planning::evidence::*;

use super::*;

mod fixtures;
use fixtures::*;

#[test]
fn unstarted_criteria_are_pending_without_observing_git_or_creating_artifacts() {
    let f = Fixture::new();
    let state = f
        .service
        .planning_snapshot(&TaskEvidenceSnapshotQuery::default())
        .unwrap();
    assert_eq!(
        state.tasks[0].criteria[0].state,
        Some(TaskCriterionEvidenceState::Pending)
    );
    assert_eq!(state.tasks[0].unavailable, None);
    assert_eq!(f.git.reads.load(Ordering::SeqCst), 0);
    assert_eq!(f.artifacts.reads.load(Ordering::SeqCst), 0);
}

#[test]
fn queued_source_uses_frozen_tree_and_preserves_reported_status_separately_from_host_observation() {
    for (reported, expected) in [
        (TaskEvidenceStatus::Pass, TaskCriterionEvidenceState::Pass),
        (TaskEvidenceStatus::Fail, TaskCriterionEvidenceState::Fail),
        (
            TaskEvidenceStatus::Blocked,
            TaskCriterionEvidenceState::Blocked,
        ),
    ] {
        let f = Fixture::new();
        f.install_record(reported);
        let state = f
            .service
            .planning_snapshot(&TaskEvidenceSnapshotQuery::default())
            .unwrap();
        assert_eq!(state.tasks[0].criteria[0].state, Some(expected));
        assert_eq!(
            state.tasks[0].source.as_ref().unwrap().tree.head_commit_sha,
            "b".repeat(40)
        );
        assert_eq!(f.git.reads.load(Ordering::SeqCst), 1);
        assert_eq!(f.artifacts.reads.load(Ordering::SeqCst), 1);
    }
}

#[test]
fn current_tree_or_session_change_marks_old_record_stale() {
    let f = Fixture::new();
    f.install_record(TaskEvidenceStatus::Pass);
    *f.git.tree.lock().unwrap() = "d".repeat(40);
    let state = f
        .service
        .planning_snapshot(&TaskEvidenceSnapshotQuery::default())
        .unwrap();
    assert_eq!(
        state.tasks[0].criteria[0].state,
        Some(TaskCriterionEvidenceState::Stale)
    );
    *f.git.tree.lock().unwrap() = "c".repeat(40);
    f.repository.snapshot.lock().unwrap().tasks[0].queue_records[0].session_key =
        "another-session".into();
    let state = f
        .service
        .planning_snapshot(&TaskEvidenceSnapshotQuery::default())
        .unwrap();
    assert_eq!(
        state.tasks[0].criteria[0].state,
        Some(TaskCriterionEvidenceState::Stale)
    );
}

#[test]
fn deleted_and_recreated_acceptance_cannot_reactivate_an_identical_old_record() {
    let f = Fixture::new();
    f.install_record(TaskEvidenceStatus::Pass);
    f.repository.snapshot.lock().unwrap().tasks[0]
        .scope
        .scope_id = "9".repeat(64);
    let state = f
        .service
        .planning_snapshot(&TaskEvidenceSnapshotQuery::default())
        .unwrap();
    assert_eq!(
        state.tasks[0].criteria[0].state,
        Some(TaskCriterionEvidenceState::Stale)
    );
    assert_eq!(f.git.reads.load(Ordering::SeqCst), 0);
}

#[test]
fn missing_seal_or_missing_frozen_source_never_preserves_a_previous_pass() {
    let f = Fixture::new();
    f.install_record(TaskEvidenceStatus::Pass);
    f.artifacts.available.store(false, Ordering::SeqCst);
    let state = f
        .service
        .planning_snapshot(&TaskEvidenceSnapshotQuery::default())
        .unwrap();
    assert_eq!(state.tasks[0].criteria[0].state, None);
    assert_eq!(
        state.tasks[0].unavailable,
        Some(TaskEvidenceError::ArtifactMissing)
    );
    f.artifacts.available.store(true, Ordering::SeqCst);
    f.repository.snapshot.lock().unwrap().tasks[0].queue_records[0]
        .source_commit_sha
        .clear();
    let state = f
        .service
        .planning_snapshot(&TaskEvidenceSnapshotQuery::default())
        .unwrap();
    assert_eq!(state.tasks[0].criteria[0].state, None);
    assert_eq!(
        state.tasks[0].unavailable,
        Some(TaskEvidenceError::Unavailable)
    );
}

#[test]
fn ambiguous_queues_and_a_new_leased_generation_are_unavailable() {
    let f = Fixture::new();
    f.install_record(TaskEvidenceStatus::Pass);
    {
        let mut snapshot = f.repository.snapshot.lock().unwrap();
        let queue = snapshot.tasks[0].queue_records[0].clone();
        snapshot.tasks[0].queue_records.push(queue);
    }
    let state = f
        .service
        .planning_snapshot(&TaskEvidenceSnapshotQuery::default())
        .unwrap();
    assert_eq!(state.tasks[0].criteria[0].state, None);
    {
        let mut snapshot = f.repository.snapshot.lock().unwrap();
        snapshot.tasks[0].queue_records.pop();
        let mut active = lease();
        active.state = ParallelModeSlotLeaseState::Leased;
        snapshot.tasks[0].leases.push(active);
    }
    let state = f
        .service
        .planning_snapshot(&TaskEvidenceSnapshotQuery::default())
        .unwrap();
    assert_eq!(state.tasks[0].criteria[0].state, None);
}

#[test]
fn a_missing_second_artifact_clears_all_current_results_in_that_task() {
    let f = Fixture::new();
    f.install_record(TaskEvidenceStatus::Pass);
    {
        let mut snapshot = f.repository.snapshot.lock().unwrap();
        let task = &mut snapshot.tasks[0];
        let mut second = task.task.acceptance[0].clone();
        second.id = "criterion 2".into();
        task.task.acceptance.push(second);
        task.scope.criteria_sha256 = criteria_sha256(&task.task.acceptance).unwrap();
        task.evidence[0].evidence.attempt.scope = task.scope.clone();
        let mut receipt = task.evidence[0].clone();
        receipt.event_id = "evidence-event-2".into();
        receipt.evidence_revision = 2;
        receipt.evidence.attempt.criterion_id = "criterion 2".into();
        receipt.evidence.attempt.attempt_id = "5".repeat(64);
        *f.artifacts.unavailable_attempt.lock().unwrap() =
            Some(receipt.evidence.attempt.attempt_id.clone());
        task.evidence.push(receipt);
        snapshot.evidence_revision = 2;
    }
    let state = f
        .service
        .planning_snapshot(&TaskEvidenceSnapshotQuery::default())
        .unwrap();
    assert_eq!(
        state.tasks[0].unavailable,
        Some(TaskEvidenceError::ArtifactMissing)
    );
    assert!(
        state.tasks[0]
            .criteria
            .iter()
            .all(|row| row.state.is_none())
    );
    assert_eq!(
        state.tasks[0].criteria[0].evidence.as_ref().unwrap().status,
        TaskEvidenceStatus::Pass
    );
}

#[test]
fn acceptance_or_evidence_mutation_during_query_discards_the_whole_page() {
    let f = Fixture::new();
    f.install_record(TaskEvidenceStatus::Pass);
    f.repository
        .change_on_second_read
        .store(true, Ordering::SeqCst);
    let state = f
        .service
        .planning_snapshot(&TaskEvidenceSnapshotQuery::default())
        .unwrap();
    assert_eq!(state.unavailable, Some(TaskEvidenceError::Stale));
    assert!(state.tasks.is_empty());
}

#[test]
fn a_duplicate_record_returns_its_first_receipt_after_scope_and_source_have_changed() {
    let f = Fixture::new();
    let original = f.install_record(TaskEvidenceStatus::Pass);
    f.repository.snapshot.lock().unwrap().tasks.clear();
    let mut later = context();
    later.caller = Some("later-caller".into());
    later.at = "2026-11-24T00:00:00Z".into();
    let result = f
        .service
        .record_evidence(
            RecordTaskEvidenceRequest {
                attempt_id: original.evidence.attempt.attempt_id.clone(),
                status: TaskEvidenceStatus::Pass,
                note: Some("  worker log  ".into()),
            },
            &later,
        )
        .unwrap();
    assert!(!result.changed);
    assert_eq!(result.value, original);
    assert_eq!(f.git.reads.load(Ordering::SeqCst), 0);
    assert_eq!(f.artifacts.reads.load(Ordering::SeqCst), 0);
    let error = f
        .service
        .record_evidence(
            RecordTaskEvidenceRequest {
                attempt_id: original.evidence.attempt.attempt_id,
                status: TaskEvidenceStatus::Fail,
                note: Some("worker log".into()),
            },
            &later,
        )
        .unwrap_err();
    assert_eq!(
        error.downcast_ref::<TaskEvidenceError>(),
        Some(&TaskEvidenceError::Conflict)
    );
}

#[test]
fn waiver_requires_native_exact_target_and_never_treats_caller_as_confirmation() {
    let f = Fixture::new();
    let request = waiver_request();
    let mut ctx = context();
    ctx.caller = Some("human".into());
    let error = f
        .service
        .waive_criterion(request.clone(), &ctx)
        .unwrap_err();
    assert_eq!(
        error.downcast_ref::<TaskEvidenceError>(),
        Some(&TaskEvidenceError::HumanConfirmationRequired)
    );
    let preview = f.service.prepare_waiver(&request).unwrap();
    let confirmation_ref = format!("desktop-confirmation:{}", "a".repeat(32));
    ctx.human_confirmation = Some(
        VerifiedHumanConfirmation::from_verified(preview.target, confirmation_ref.clone()).unwrap(),
    );
    let mut changed_reason = request.clone();
    changed_reason.reason = "different reason".into();
    assert!(f.service.waive_criterion(changed_reason, &ctx).is_err());
    let receipt = f.service.waive_criterion(request, &ctx).unwrap();
    assert_eq!(receipt.value.waiver.reason, "manual exception");
    assert_eq!(receipt.value.waiver.confirmation_ref, confirmation_ref);
    let state = f
        .service
        .planning_snapshot(&TaskEvidenceSnapshotQuery::default())
        .unwrap();
    assert_eq!(
        state.tasks[0].criteria[0].state,
        Some(TaskCriterionEvidenceState::Waived)
    );
    assert_eq!(f.git.reads.load(Ordering::SeqCst), 0);
    f.repository.snapshot.lock().unwrap().tasks[0]
        .scope
        .scope_id = "9".repeat(64);
    let state = f
        .service
        .planning_snapshot(&TaskEvidenceSnapshotQuery::default())
        .unwrap();
    assert_eq!(
        state.tasks[0].criteria[0].state,
        Some(TaskCriterionEvidenceState::Stale)
    );
}

#[test]
fn stale_prepare_scope_and_missing_running_lease_are_rejected_before_file_creation() {
    let f = Fixture::new();
    let mut request = PrepareTaskEvidenceRequest {
        request_id: "prepare-1".into(),
        task_id: "task-1".into(),
        criterion_id: "criterion 1".into(),
        scope_id: "9".repeat(64),
        expected_acceptance_revision: 1,
    };
    assert_eq!(
        f.service
            .prepare_evidence(request.clone(), &context())
            .unwrap_err()
            .downcast_ref::<TaskEvidenceError>(),
        Some(&TaskEvidenceError::ScopeChanged)
    );
    request.scope_id = "1".repeat(64);
    assert_eq!(
        f.service
            .prepare_evidence(request, &context())
            .unwrap_err()
            .downcast_ref::<TaskEvidenceError>(),
        Some(&TaskEvidenceError::LeaseChanged)
    );
    assert_eq!(f.artifacts.reads.load(Ordering::SeqCst), 0);
}
