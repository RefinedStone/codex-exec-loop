use super::*;

#[test]
fn shutdown_waits_for_admitted_policy_writes_and_rejects_later_ones() {
    let f = Fixture::new();
    let claim = f.start();
    f.deliver_executed(&claim);
    assert!(f.service.poll(Instant::now()));
    let receipt = f
        .repository
        .load_host_verification("/repo", &claim.stored.attempt.attempt_id)
        .unwrap()
        .unwrap()
        .receipt
        .unwrap();
    let entered = Arc::new(Barrier::new(2));
    let release = Arc::new(Barrier::new(2));
    *f.repository.failure_limit_hook.lock().unwrap() = Some({
        let entered = entered.clone();
        let release = release.clone();
        Arc::new(move || {
            entered.wait();
            release.wait();
        })
    });
    std::thread::scope(|threads| {
        let service = f.service.clone();
        let receipt = receipt.clone();
        let lease = claim.stored.lease.clone();
        let policy = threads.spawn(move || {
            service.apply_automatic_failure_limit(
                &receipt,
                &lease,
                &PostTurnContinuationGate::default().capture(),
                Instant::now() + Duration::from_secs(5),
            )
        });
        entered.wait();
        let service = f.service.clone();
        let (send, receive) = mpsc::channel();
        let stop = threads.spawn(move || {
            service.shutdown();
            send.send(()).unwrap();
        });
        wait_until(|| f.service.closed.load(Ordering::Acquire));
        assert!(
            matches!(receive.try_recv(), Err(mpsc::TryRecvError::Empty)),
            "shutdown cannot release ownership while a policy write is admitted"
        );
        release.wait();
        policy.join().unwrap().unwrap();
        stop.join().unwrap();
        receive.recv_timeout(Duration::from_secs(1)).unwrap();
    });
    assert!(
        f.service
            .apply_automatic_failure_limit(
                &receipt,
                &claim.stored.lease,
                &PostTurnContinuationGate::default().capture(),
                Instant::now() + Duration::from_secs(1),
            )
            .is_err()
    );
    assert_eq!(f.repository.failure_limit_calls.load(Ordering::SeqCst), 1);
}
