use crate::domain::planning::PostTurnContinuationGate;

use super::super::automatic::ExecutionGuard;
use super::*;

const WAIT: Duration = Duration::from_secs(5);

fn finished_receipt(fixture: &Fixture) -> (TaskVerificationClaimResult, TaskVerificationReceipt) {
    let claim = fixture.start();
    fixture.deliver_executed(&claim);
    assert!(fixture.service.poll(Instant::now()));
    let receipt = fixture
        .repository
        .load_host_verification("/repo", &claim.stored.attempt.attempt_id)
        .unwrap()
        .unwrap()
        .receipt
        .unwrap();
    (claim, receipt)
}

#[test]
fn automatic_policy_admitted_before_cancellation_finishes_before_invalidation_returns() {
    let fixture = Fixture::new();
    let (claim, receipt) = finished_receipt(&fixture);
    let gate = PostTurnContinuationGate::default();
    let permit = gate.capture();
    let continuation: Arc<dyn TaskVerificationContinuation> = Arc::new(permit.clone());
    let (entered_tx, entered_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let release_rx = Mutex::new(release_rx);
    *fixture.repository.failure_limit_hook.lock().unwrap() = Some(Arc::new(move || {
        entered_tx.send(()).unwrap();
        release_rx
            .lock()
            .unwrap()
            .recv_timeout(WAIT)
            .expect("the policy commit must be released by its test owner");
    }));

    std::thread::scope(|threads| {
        let policy = threads.spawn(|| {
            fixture.service.apply_automatic_failure_limit(
                &receipt,
                &claim.stored.lease,
                continuation.as_ref(),
                Instant::now() + WAIT,
            )
        });
        let entered = entered_rx.recv_timeout(WAIT);
        if entered.is_err() {
            let _ = release_tx.send(());
        }
        entered.expect("the admitted policy must reach its repository commit");

        let (attempted_tx, attempted_rx) = mpsc::channel();
        let (cancelled_tx, cancelled_rx) = mpsc::channel();
        let cancelling_permit = permit.clone();
        let cancellation = threads.spawn(move || {
            attempted_tx.send(()).unwrap();
            cancelled_tx
                .send(cancelling_permit.invalidate_if_current())
                .unwrap();
        });
        let attempted = attempted_rx.recv_timeout(WAIT);
        let early_cancellation = cancelled_rx.recv_timeout(Duration::from_millis(100));
        // Release the repository hook before asserting, including on a broken
        // implementation, so a failing regression never strands its worker.
        release_tx.send(()).unwrap();
        let policy_result = policy.join().unwrap();
        cancellation.join().unwrap();

        attempted.expect("cancellation must be attempted while policy is admitted");
        assert!(
            matches!(early_cancellation, Err(mpsc::RecvTimeoutError::Timeout)),
            "Pause must wait for an already admitted bounded policy commit"
        );
        assert!(cancelled_rx.recv_timeout(WAIT).unwrap());
        policy_result.unwrap();
    });
    assert!(!permit.is_current());
    assert_eq!(
        fixture
            .repository
            .failure_limit_calls
            .load(Ordering::SeqCst),
        1
    );
    assert_eq!(fixture.repository.finish_calls.lock().unwrap().len(), 1);
    assert_eq!(fixture.repository.outbox.lock().unwrap().len(), 1);
}

#[test]
fn automatic_policy_cancelled_before_admission_never_calls_the_repository() {
    let fixture = Fixture::new();
    let (claim, receipt) = finished_receipt(&fixture);
    let gate = PostTurnContinuationGate::default();
    let permit = gate.capture();
    let continuation: Arc<dyn TaskVerificationContinuation> = Arc::new(permit.clone());
    assert!(permit.invalidate_if_current());

    let error = fixture
        .service
        .apply_automatic_failure_limit(
            &receipt,
            &claim.stored.lease,
            continuation.as_ref(),
            Instant::now() + WAIT,
        )
        .unwrap_err();
    assert_eq!(
        error.downcast_ref::<TaskEvidenceError>(),
        Some(&TaskEvidenceError::Unavailable)
    );
    assert_eq!(
        fixture
            .repository
            .failure_limit_calls
            .load(Ordering::SeqCst),
        0
    );
    assert_eq!(fixture.repository.finish_calls.lock().unwrap().len(), 1);
    assert_eq!(fixture.repository.outbox.lock().unwrap().len(), 1);
}

#[test]
fn automatic_claim_waiting_for_admission_cannot_write_after_cancellation() {
    let fixture = Fixture::new();
    fixture.service.initialize_live_owner(AT).unwrap();
    let gate = PostTurnContinuationGate::default();
    let permit = gate.capture();
    let continuation: Arc<dyn TaskVerificationContinuation> = Arc::new(permit.clone());
    let (_, stored) = fixtures::snapshot();
    let guard = ExecutionGuard {
        source: fixtures::manifest().source,
        continuation,
        deadline: Instant::now() + WAIT,
    };
    let admission = fixture.service.flight.lock().unwrap();
    let (attempted_tx, attempted_rx) = mpsc::channel();
    std::thread::scope(|threads| {
        let request = threads.spawn(|| {
            attempted_tx.send(()).unwrap();
            fixture
                .service
                .request_guarded(request(), &context(), Some(stored.lease), Some(guard))
        });
        let attempted = attempted_rx.recv_timeout(WAIT);
        let cancelled = permit.invalidate_if_current();
        drop(admission);
        let error = request.join().unwrap().unwrap_err();
        attempted.expect("the request must be submitted before the admission lock is released");
        assert!(cancelled);
        assert_eq!(
            error.downcast_ref::<TaskEvidenceError>(),
            Some(&TaskEvidenceError::Unavailable)
        );
    });
    assert_eq!(fixture.repository.claims.load(Ordering::SeqCst), 0);
    assert_eq!(fixture.spawn_calls.load(Ordering::SeqCst), 0);
    assert!(fixture.jobs.lock().unwrap().is_empty());
    assert_eq!(fixture.runner.prepares.load(Ordering::SeqCst), 0);
}

#[test]
fn automatic_claim_admitted_before_cancellation_finishes_before_invalidation_returns() {
    let fixture = Fixture::new();
    fixture.service.initialize_live_owner(AT).unwrap();
    let gate = PostTurnContinuationGate::default();
    let permit = gate.capture();
    let continuation: Arc<dyn TaskVerificationContinuation> = Arc::new(permit.clone());
    let (_, stored) = fixtures::snapshot();
    let guard = ExecutionGuard {
        source: fixtures::manifest().source,
        continuation,
        deadline: Instant::now() + WAIT,
    };
    let (entered_tx, entered_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let release_rx = Mutex::new(release_rx);
    *fixture.repository.claim_hook.lock().unwrap() = Some(Arc::new(move || {
        entered_tx.send(()).unwrap();
        release_rx
            .lock()
            .unwrap()
            .recv_timeout(WAIT)
            .expect("the claim commit must be released by its test owner");
    }));

    std::thread::scope(|threads| {
        let claim = threads.spawn(|| {
            fixture
                .service
                .request_guarded(request(), &context(), Some(stored.lease), Some(guard))
        });
        let entered = entered_rx.recv_timeout(WAIT);
        if entered.is_err() {
            let _ = release_tx.send(());
        }
        entered.expect("the admitted claim must reach its repository commit");

        let (attempted_tx, attempted_rx) = mpsc::channel();
        let (cancelled_tx, cancelled_rx) = mpsc::channel();
        let cancelling_permit = permit.clone();
        let cancellation = threads.spawn(move || {
            attempted_tx.send(()).unwrap();
            cancelled_tx
                .send(cancelling_permit.invalidate_if_current())
                .unwrap();
        });
        let attempted = attempted_rx.recv_timeout(WAIT);
        let early_cancellation = cancelled_rx.recv_timeout(Duration::from_millis(100));
        release_tx.send(()).unwrap();
        let claim_result = claim.join().unwrap();
        cancellation.join().unwrap();

        attempted.expect("cancellation must be attempted while the claim is admitted");
        assert!(
            matches!(early_cancellation, Err(mpsc::RecvTimeoutError::Timeout)),
            "Pause must wait for an already admitted bounded claim commit"
        );
        assert!(cancelled_rx.recv_timeout(WAIT).unwrap());
        assert!(claim_result.unwrap().unwrap().changed);
    });
    assert!(!permit.is_current());
    assert_eq!(fixture.repository.claims.load(Ordering::SeqCst), 1);
    assert_eq!(fixture.spawn_calls.load(Ordering::SeqCst), 1);
    assert_eq!(fixture.jobs.lock().unwrap().len(), 1);
    assert_eq!(fixture.runner.prepares.load(Ordering::SeqCst), 0);
}
