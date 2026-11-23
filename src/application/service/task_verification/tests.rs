use super::*;
use std::sync::atomic::AtomicUsize;

use crate::application::port::outbound::planning_work_order_repository_port::{
    PlanningWorkPermitBlockReason, PlanningWorkPermitDecision,
};
use crate::application::port::outbound::task_evidence_repository_port::*;
use crate::application::port::outbound::task_verification_repository_port::*;
use crate::domain::planning::evidence::*;
use crate::domain::planning::verification::*;

mod capture;
mod fixtures;
mod repository;
pub(crate) use fixtures::Fixture;
use fixtures::{AT, context, request};
pub(crate) use repository::Repository;

#[test]
fn host_verification_construction_is_inert_and_only_explicit_owner_initialization_recovers() {
    let f = Fixture::new();
    assert_eq!(f.repository.recoveries.load(Ordering::SeqCst), 0);
    assert!(!f.service.has_pending());
    assert!(f.service.next_deadline().is_none());
    let error = f
        .service
        .request_verification(request(), &context())
        .unwrap_err();
    assert_eq!(
        error.downcast_ref::<TaskEvidenceError>(),
        Some(&TaskEvidenceError::Unavailable)
    );
    assert_eq!(f.repository.claims.load(Ordering::SeqCst), 0);
    f.service.initialize_live_owner(AT).unwrap();
    f.service
        .initialize_live_owner("2026-11-23T13:00:00Z")
        .unwrap();
    assert_eq!(f.repository.recoveries.load(Ordering::SeqCst), 1);
    assert_eq!(f.spawn_calls.load(Ordering::SeqCst), 0);
    assert_eq!(f.runner.prepares.load(Ordering::SeqCst), 0);
}

#[test]
fn host_verification_request_replay_spawns_once_and_busy_workspace_rejects_another_request() {
    let f = Fixture::new();
    let claim = f.start();
    assert!(claim.changed);
    assert!(f.service.has_pending());
    assert!(f.service.next_deadline().is_some());
    assert_eq!(
        f.repository.reads.load(Ordering::SeqCst),
        0,
        "request does not snapshot a worker tree"
    );
    let replay = f
        .service
        .request_verification(request(), &context())
        .unwrap();
    assert!(!replay.changed);
    assert!(replay.execution.is_none());
    assert_eq!(claim.stored, replay.stored);
    let before = f.repository.claims.load(Ordering::SeqCst);
    let error = f
        .service
        .request_verification(
            RequestTaskVerificationRequest {
                request_id: "another-request".into(),
                ..request()
            },
            &context(),
        )
        .unwrap_err();
    assert_eq!(
        error.downcast_ref::<TaskEvidenceError>(),
        Some(&TaskEvidenceError::Limit)
    );
    assert_eq!(f.repository.claims.load(Ordering::SeqCst), before);
    let error = f
        .service
        .request_verification(
            RequestTaskVerificationRequest {
                criterion_id: "another criterion".into(),
                ..request()
            },
            &context(),
        )
        .unwrap_err();
    assert_eq!(
        error.downcast_ref::<TaskEvidenceError>(),
        Some(&TaskEvidenceError::Conflict)
    );
    assert_eq!(f.spawn_calls.load(Ordering::SeqCst), 1);
    assert_eq!(f.jobs.lock().unwrap().len(), 1);
    assert_eq!(f.runner.prepares.load(Ordering::SeqCst), 0);
}

#[test]
fn host_verification_spawn_failure_is_a_durable_not_run_without_dispatch() {
    let f = Fixture::new();
    f.spawn_fails.store(true, Ordering::SeqCst);
    let claim = f.start();
    assert!(claim.changed);
    assert!(f.jobs.lock().unwrap().is_empty());
    assert_eq!(f.wakes.load(Ordering::SeqCst), 1);
    assert!(f.service.poll(Instant::now()));
    assert!(!f.service.has_pending());
    let stored = f
        .repository
        .load_host_verification("/repo", &claim.stored.attempt.attempt_id)
        .unwrap()
        .unwrap();
    assert_eq!(
        stored.receipt.unwrap().result,
        TaskVerificationResult::NotRun {
            reason: TaskVerificationNotRunReason::SpawnFailed
        }
    );
    assert_eq!(f.repository.outbox.lock().unwrap().len(), 1);
    assert_eq!(f.runner.prepares.load(Ordering::SeqCst), 0);
    assert!(
        !f.service
            .request_verification(request(), &context())
            .unwrap()
            .changed
    );
    assert_eq!(f.spawn_calls.load(Ordering::SeqCst), 1);
}

#[test]
fn host_verification_failed_finish_retries_the_same_observation_and_timestamp_without_rerun() {
    let f = Fixture::new();
    let claim = f.start();
    let original = f.deliver_executed(&claim);
    f.repository.fail_finish_once.store(true, Ordering::SeqCst);
    let now = Instant::now();
    assert!(!f.service.poll(now));
    assert!(f.service.has_pending());
    assert_eq!(
        f.service.next_deadline(),
        Some(now + Duration::from_secs(1))
    );
    assert!(f.repository.outbox.lock().unwrap().is_empty());
    assert!(!f.service.poll(now + Duration::from_millis(999)));
    assert_eq!(f.repository.finish_calls.lock().unwrap().len(), 1);
    assert!(
        !f.service
            .request_verification(request(), &context())
            .unwrap()
            .changed
    );
    assert!(f.service.poll(now + Duration::from_secs(1)));
    assert!(!f.service.has_pending());
    assert!(f.service.next_deadline().is_none());
    let calls = f.repository.finish_calls.lock().unwrap();
    assert_eq!(calls.len(), 2);
    for call in calls.iter() {
        assert_eq!(call.execution, original.execution);
        assert_eq!(call.result, original.result);
        assert_eq!(call.artifact, original.artifact);
        assert_eq!(call.at, original.at);
    }
    drop(calls);
    let receipt = f
        .repository
        .load_host_verification("/repo", &claim.stored.attempt.attempt_id)
        .unwrap()
        .unwrap()
        .receipt
        .unwrap();
    assert!(receipt.passed());
    assert_eq!(receipt.finished_at, original.at);
    assert_eq!(f.repository.outbox.lock().unwrap().len(), 1);
    assert_eq!(f.spawn_calls.load(Ordering::SeqCst), 1);
    assert_eq!(f.runner.prepares.load(Ordering::SeqCst), 0);
    assert!(
        !f.service
            .request_verification(request(), &context())
            .unwrap()
            .changed
    );
    assert_eq!(f.spawn_calls.load(Ordering::SeqCst), 1);
}

#[test]
fn host_verification_drop_cancels_a_reserved_worker_before_any_git_preparation() {
    let f = Fixture::new();
    f.start();
    let cancelled = f
        .service
        .flight
        .lock()
        .unwrap()
        .as_ref()
        .unwrap()
        .cancelled
        .clone();
    let (work, interrupted) = f.take_job();
    drop(f.service);
    assert!(cancelled.load(Ordering::Acquire));
    drop(interrupted);
    work();
    assert_eq!(f.runner.prepares.load(Ordering::SeqCst), 0);
    assert!(
        f.repository.finish_calls.lock().unwrap().is_empty(),
        "a dropped owner cannot persist a completion"
    );
    assert_eq!(f.wakes.load(Ordering::SeqCst), 1);
}

#[test]
fn host_verification_panic_callback_and_disconnected_worker_finish_interrupted_once() {
    for use_panic_callback in [true, false] {
        let f = Fixture::new();
        let claim = f.start();
        let (work, interrupted) = f.take_job();
        drop(work);
        if use_panic_callback {
            // The existing effect-worker helper invokes this callback after a caught panic.
            interrupted();
        } else {
            drop(interrupted);
        }
        assert!(f.service.poll(Instant::now()));
        assert!(!f.service.poll(Instant::now() + Duration::from_secs(2)));
        let stored = f
            .repository
            .load_host_verification("/repo", &claim.stored.attempt.attempt_id)
            .unwrap()
            .unwrap();
        let receipt = stored.receipt.unwrap();
        assert_eq!(receipt.result, TaskVerificationResult::Interrupted {});
        assert!(receipt.manifest.is_none());
        assert_eq!(f.repository.finish_calls.lock().unwrap().len(), 1);
        assert_eq!(f.repository.outbox.lock().unwrap().len(), 1);
        assert_eq!(f.spawn_calls.load(Ordering::SeqCst), 1);
        assert_eq!(f.runner.prepares.load(Ordering::SeqCst), 0);
    }
}

#[test]
fn host_verification_poll_cancels_when_dispatch_permission_scope_or_lease_changes() {
    for change in ["permit", "scope", "lease"] {
        let f = Fixture::new();
        let claim = f.start();
        let now = Instant::now();
        assert!(!f.service.poll(now));
        let cancelled = f
            .service
            .flight
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .cancelled
            .clone();
        assert!(!cancelled.load(Ordering::Acquire));
        match change {
            "permit" => {
                *f.permit.lock().unwrap() =
                    PlanningWorkPermitDecision::Denied(PlanningWorkPermitBlockReason::Missing)
            }
            "scope" => {
                f.repository.snapshot.lock().unwrap().tasks[0]
                    .scope
                    .scope_id = "f".repeat(64)
            }
            "lease" => {
                f.repository.snapshot.lock().unwrap().tasks[0].leases[0].lease_generation =
                    Some("f".repeat(64))
            }
            _ => unreachable!(),
        }
        assert!(!f.service.poll(now + Duration::from_secs(1)));
        assert!(cancelled.load(Ordering::Acquire));
        let (work, interrupted) = f.take_job();
        drop(interrupted);
        work();
        assert!(f.service.poll(now + Duration::from_secs(2)));
        let receipt = f
            .repository
            .load_host_verification("/repo", &claim.stored.attempt.attempt_id)
            .unwrap()
            .unwrap()
            .receipt
            .unwrap();
        assert_eq!(
            receipt.result,
            TaskVerificationResult::NotRun {
                reason: TaskVerificationNotRunReason::Cancelled
            }
        );
        assert_eq!(f.runner.prepares.load(Ordering::SeqCst), 0);
        assert_eq!(f.spawn_calls.load(Ordering::SeqCst), 1);
    }
}
