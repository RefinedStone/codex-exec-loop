use crate::domain::planning::PostTurnContinuationGate;
use std::sync::Barrier;

use super::super::automatic::{ExecutionGuard, automatic_request};
use super::*;

fn guard() -> ExecutionGuard {
    ExecutionGuard {
        source: fixtures::manifest().source,
        continuation: Arc::new(PostTurnContinuationGate::default().capture()),
        deadline: Instant::now() + Duration::from_secs(5),
    }
}

fn wait_until(mut predicate: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(2);
    while !predicate() {
        assert!(
            Instant::now() < deadline,
            "fixture synchronization timed out"
        );
        std::thread::yield_now();
    }
}

#[test]
fn automatic_request_identity_replays_the_same_completion_and_separates_new_work() {
    let (snapshot, stored) = fixtures::snapshot();
    let task = &snapshot.tasks[0];
    let source = fixtures::manifest().source;
    let make = |lease: &_, source: &_, turn: &str| {
        automatic_request(&task.scope, "criterion 1", lease, source, turn).unwrap()
    };
    let first = make(&stored.lease, &source, "turn-1");
    first.validate().unwrap();
    assert_eq!(
        first.request_id,
        make(&stored.lease, &source, "turn-1").request_id
    );
    let mut advanced_state = stored.lease.clone();
    advanced_state.state = crate::domain::parallel_mode::ParallelModeSlotLeaseState::Leased;
    assert_eq!(
        first.request_id,
        make(&advanced_state, &source, "turn-1").request_id
    );
    let mut replacement = stored.lease.clone();
    replacement.lease_generation = Some("5".repeat(64));
    assert_ne!(
        first.request_id,
        make(&replacement, &source, "turn-1").request_id
    );
    let mut changed_source = source.clone();
    changed_source.tree_sha = "e".repeat(40);
    assert_ne!(
        first.request_id,
        make(&stored.lease, &changed_source, "turn-1").request_id
    );
    assert_ne!(
        first.request_id,
        make(&stored.lease, &source, "turn-2").request_id
    );
}

#[test]
fn automatic_wait_requires_durable_finish_and_keeps_the_original_retry_fact() {
    let f = Fixture::new();
    let claim = f.start();
    let stored = claim.stored.clone();
    let service = f.service.clone();
    let (send, receive) = mpsc::channel();
    std::thread::scope(|threads| {
        threads.spawn(move || {
            send.send(service.wait_for_automatic_receipt(
                &stored,
                &guard(),
                Instant::now() + Duration::from_secs(5),
            ))
            .unwrap();
        });
        wait_until(|| f.repository.verification_reads.load(Ordering::SeqCst) > 0);
        let original = f.deliver_executed(&claim);
        assert!(matches!(receive.try_recv(), Err(mpsc::TryRecvError::Empty)));
        f.repository.fail_finish_once.store(true, Ordering::SeqCst);
        let now = Instant::now();
        assert!(!f.service.poll(now));
        assert!(matches!(receive.try_recv(), Err(mpsc::TryRecvError::Empty)));
        assert!(f.service.has_pending());
        assert!(f.service.poll(now + Duration::from_secs(1)));
        let finished = receive
            .recv_timeout(Duration::from_secs(2))
            .unwrap()
            .unwrap();
        let receipt = finished.receipt.unwrap();
        assert_eq!(receipt.finished_at, original.at);
        assert_eq!(receipt.result, original.result);
        assert_eq!(f.repository.finish_calls.lock().unwrap().len(), 2);
        assert_eq!(f.spawn_calls.load(Ordering::SeqCst), 1);
        assert_eq!(
            f.repository.acknowledgements.load(Ordering::SeqCst),
            0,
            "outbox ACK is not a prerequisite for the durable observation"
        );
    });
}

#[test]
fn automatic_cancellation_reaches_the_reserved_worker_before_git_preparation() {
    let f = Fixture::new();
    f.service.initialize_live_owner(AT).unwrap();
    let permit = PostTurnContinuationGate::default().capture();
    let execution_guard = ExecutionGuard {
        source: fixtures::manifest().source,
        continuation: Arc::new(permit.clone()),
        deadline: Instant::now() + Duration::from_secs(5),
    };
    let lease = fixtures::snapshot().1.lease;
    let claim = f
        .service
        .request_guarded(request(), &context(), Some(lease), Some(execution_guard))
        .unwrap()
        .unwrap();
    assert!(permit.invalidate_if_current());
    let (work, interrupted) = f.take_job();
    drop(interrupted);
    work();
    assert!(f.service.poll(Instant::now()));
    let stored = f
        .repository
        .load_host_verification("/repo", &claim.stored.attempt.attempt_id)
        .unwrap()
        .unwrap();
    assert_eq!(
        stored.receipt.unwrap().result,
        TaskVerificationResult::NotRun {
            reason: TaskVerificationNotRunReason::Cancelled
        }
    );
    assert_eq!(f.repository.reads.load(Ordering::SeqCst), 0);
    assert_eq!(f.runner.prepares.load(Ordering::SeqCst), 0);
}

#[test]
fn automatic_wait_has_one_deadline_and_rejects_cancelled_or_replaced_input() {
    let f = Fixture::new();
    let claim = f.start();
    assert!(
        f.service
            .wait_for_automatic_receipt(&claim.stored, &guard(), Instant::now())
            .is_err()
    );
    assert_eq!(f.repository.verification_reads.load(Ordering::SeqCst), 0);
    let mut cancelled = guard();
    let permit = PostTurnContinuationGate::default().capture();
    assert!(permit.invalidate_if_current());
    cancelled.continuation = Arc::new(permit);
    assert!(
        f.service
            .wait_for_automatic_receipt(
                &claim.stored,
                &cancelled,
                Instant::now() + Duration::from_secs(1),
            )
            .is_err()
    );
    assert_eq!(f.repository.verification_reads.load(Ordering::SeqCst), 0);
    f.deliver_executed(&claim);
    assert!(f.service.poll(Instant::now()));
    let mut changed = guard();
    changed.source.tree_sha = "a".repeat(40);
    assert!(
        f.service
            .wait_for_automatic_receipt(
                &claim.stored,
                &changed,
                Instant::now() + Duration::from_secs(1),
            )
            .is_err()
    );
}

#[test]
fn automatic_completion_skips_manual_recipes_unconfigured_optional_and_waived_criteria() {
    for mode in 0..4 {
        let f = Fixture::new();
        f.service.initialize_live_owner(AT).unwrap();
        let lease = {
            let mut snapshot = f.repository.snapshot.lock().unwrap();
            let task = &mut snapshot.tasks[0];
            match mode {
                0 => task.task.acceptance[0].verification_recipe = None,
                1 => task.task.acceptance[0].essential = false,
                3 => task.task.acceptance[0].verification_recipe = Some(
                    crate::domain::planning::verification::TaskVerificationRecipe::RepoGitGrepV1,
                ),
                _ => {
                    let (reason, reason_digest) =
                        normalize_waiver_reason("Approved exception").unwrap();
                    task.waivers.push(TaskCriterionWaiverReceipt {
                        event_id: "waiver-event".into(),
                        evidence_revision: 1,
                        waiver: TaskCriterionWaiver {
                            request_id: "waiver-request".into(),
                            scope: task.scope.clone(),
                            criterion_id: task.task.acceptance[0].id.clone(),
                            reason,
                            reason_digest,
                            confirmation_ref: "human-confirmation".into(),
                            recorded_at: AT.into(),
                        },
                    });
                }
            }
            task.scope.criteria_sha256 = criteria_sha256(&task.task.acceptance).unwrap();
            task.leases[0].clone()
        };
        assert_eq!(
            f.service
                .verify_automatic_completion(
                    "/repo",
                    &lease,
                    &"c".repeat(40),
                    "turn-1",
                    Arc::new(PostTurnContinuationGate::default().capture()),
                )
                .unwrap(),
            AutomaticTaskVerificationOutcome::NotApplicable
        );
        assert_eq!(f.repository.claims.load(Ordering::SeqCst), 0);
        assert_eq!(f.spawn_calls.load(Ordering::SeqCst), 0);
    }
}

#[test]
fn automatic_completion_never_adopts_a_replacement_lease() {
    let f = Fixture::new();
    f.service.initialize_live_owner(AT).unwrap();
    let mut expected = fixtures::snapshot().1.lease;
    expected.lease_generation = Some("7".repeat(64));
    let error = f
        .service
        .verify_automatic_completion(
            "/repo",
            &expected,
            &"c".repeat(40),
            "turn-1",
            Arc::new(PostTurnContinuationGate::default().capture()),
        )
        .unwrap_err();
    assert_eq!(
        error.downcast_ref::<TaskEvidenceError>(),
        Some(&TaskEvidenceError::LeaseChanged)
    );
    assert_eq!(f.repository.claims.load(Ordering::SeqCst), 0);
    assert_eq!(f.spawn_calls.load(Ordering::SeqCst), 0);
}

mod shutdown;
