use super::*;

struct ReleaseHandler(Arc<TestHandler>);
impl Drop for ReleaseHandler {
    fn drop(&mut self) {
        self.0.release.store(true, Ordering::Release);
        self.0.hold_cleanup.store(false, Ordering::Release);
    }
}

#[test]
fn worker_panic_becomes_failed_and_does_not_stop_other_extension_calls() {
    let handler = Arc::new(TestHandler::default());
    handler.panic_worker.store(true, Ordering::Release);
    let host = service(Arc::new(MemoryStore::default()), handler.clone());
    enable(&host);
    let answer = call(&host, "work", json!({"text":"panic"}), Some("panic")).unwrap();
    let id = answer["id"].as_str().unwrap();
    wait_until(|| host.job("akra.test", id).unwrap().cleanup == ExtensionCleanupState::Complete);
    let job = host.job("akra.test", id).unwrap();
    assert_eq!(job.state, ExtensionJobState::Failed);
    assert_eq!(job.error.as_deref(), Some("extension worker panicked"));
    assert_eq!(handler.cleanups.lock().unwrap().len(), 1);
    assert_eq!(
        call(&host, "echo", json!({"text":"still available"}), None).unwrap()["text"],
        "still available"
    );
}

#[test]
fn duplicate_cancel_waits_for_worker_and_terminal_cancel_is_idempotent() {
    let handler = Arc::new(TestHandler::default());
    handler.hold_cancelled_worker.store(true, Ordering::Release);
    let _release = ReleaseHandler(handler.clone());
    let host = service(Arc::new(MemoryStore::default()), handler.clone());
    enable(&host);
    let started = call(&host, "work", json!({"text":"last result"}), Some("cancel")).unwrap();
    let id = started["id"].as_str().unwrap();
    wait_until(|| handler.entered.load(Ordering::Acquire) == 1);
    for _ in 0..2 {
        let cancelled = call(&host, "cancel", json!({"job_id":id}), None).unwrap();
        assert_eq!(cancelled["state"], "cancelling");
    }
    assert_eq!(handler.cleanup_entered.load(Ordering::Acquire), 0);
    handler.release.store(true, Ordering::Release);
    wait_until(|| host.job("akra.test", id).unwrap().cleanup == ExtensionCleanupState::Complete);
    let job = host.job("akra.test", id).unwrap();
    assert_eq!(job.state, ExtensionJobState::Cancelled);
    assert!(
        job.result.is_none(),
        "late successful output must not survive cancellation"
    );
    let repeated = call(&host, "cancel", json!({"job_id":id}), None).unwrap();
    assert_eq!(repeated["state"], "cancelled");
    assert_eq!(repeated["revision"], job.revision);
    assert_eq!(handler.cleanups.lock().unwrap().len(), 1);

    let completed = call(&host, "work", json!({"text":"completed"}), Some("complete")).unwrap();
    let completed_id = completed["id"].as_str().unwrap();
    wait_until(|| {
        host.job("akra.test", completed_id).unwrap().state == ExtensionJobState::Completed
    });
    let completed = host.job("akra.test", completed_id).unwrap();
    let repeated = call(&host, "cancel", json!({"job_id":completed_id}), None).unwrap();
    assert_eq!(repeated["state"], "completed");
    assert_eq!(repeated["revision"], completed.revision);
    assert_eq!(repeated["result"]["text"], "completed");
}

#[test]
fn disable_cannot_enable_until_old_worker_and_cleanup_have_finished() {
    let handler = Arc::new(TestHandler::default());
    handler.hold_cancelled_worker.store(true, Ordering::Release);
    handler.hold_cleanup.store(true, Ordering::Release);
    let _release = ReleaseHandler(handler.clone());
    let host = service(Arc::new(MemoryStore::default()), handler.clone());
    enable(&host);
    let started = call(&host, "work", json!({"text":"old"}), Some("old")).unwrap();
    let id = started["id"].as_str().unwrap();
    wait_until(|| handler.entered.load(Ordering::Acquire) == 1);
    let revision = host.catalog()[0].revision;
    call(
        &host,
        "configure",
        json!({"enabled":false,"expected_revision":revision}),
        None,
    )
    .unwrap();
    assert_eq!(host.catalog()[0].lifecycle, ExtensionLifecycle::Disabling);
    assert_eq!(
        host.job("akra.test", id).unwrap().state,
        ExtensionJobState::Cancelling
    );
    assert!(
        call(
            &host,
            "configure",
            json!({"enabled":true,"expected_revision":revision+1}),
            None
        )
        .is_err()
    );
    assert!(call(&host, "work", json!({"text":"too early"}), None).is_err());
    handler.release.store(true, Ordering::Release);
    wait_until(|| handler.cleanup_entered.load(Ordering::Acquire) == 1);
    assert_eq!(host.catalog()[0].lifecycle, ExtensionLifecycle::Disabling);
    assert!(
        call(
            &host,
            "configure",
            json!({"enabled":true,"expected_revision":revision+1}),
            None
        )
        .is_err()
    );
    let hidden = call(&host, "job", json!({"job_id":id}), None).unwrap();
    assert!(hidden["result"].is_null() && hidden["progress"].is_null());
    handler.hold_cleanup.store(false, Ordering::Release);
    wait_until(|| host.catalog()[0].lifecycle == ExtensionLifecycle::Disabled);
    let old = host.job("akra.test", id).unwrap();
    enable(&host);
    let next = call(&host, "work", json!({"text":"new"}), Some("new")).unwrap();
    let next_id = next["id"].as_str().unwrap();
    wait_until(|| host.job("akra.test", next_id).unwrap().state == ExtensionJobState::Completed);
    host.cleanup_job(&context(&host), &old).unwrap();
    assert_eq!(
        handler.cleanups.lock().unwrap().as_slice(),
        &[(id.to_owned(), old.generation)]
    );
    assert_eq!(
        host.job("akra.test", next_id).unwrap().result.unwrap()["text"],
        "new"
    );
}

#[test]
fn cleanup_failure_stays_visible_and_disable_can_retry_before_reenable() {
    let handler = Arc::new(TestHandler::default());
    handler.release.store(true, Ordering::Release);
    handler.fail_cleanup.store(true, Ordering::Release);
    let host = service(Arc::new(MemoryStore::default()), handler.clone());
    enable(&host);
    let started = call(&host, "work", json!({"text":"cleanup"}), Some("cleanup")).unwrap();
    let id = started["id"].as_str().unwrap();
    wait_until(|| host.job("akra.test", id).unwrap().state == ExtensionJobState::Completed);
    let revision = host.catalog()[0].revision;
    call(
        &host,
        "configure",
        json!({"enabled":false,"expected_revision":revision}),
        None,
    )
    .unwrap();
    wait_until(|| host.catalog()[0].lifecycle == ExtensionLifecycle::Error);
    let revision = host.catalog()[0].revision;
    assert!(
        call(
            &host,
            "configure",
            json!({"enabled":true,"expected_revision":revision}),
            None
        )
        .is_err()
    );
    assert_eq!(
        host.job("akra.test", id).unwrap().cleanup,
        ExtensionCleanupState::Failed
    );
    handler.fail_cleanup.store(false, Ordering::Release);
    call(
        &host,
        "configure",
        json!({"enabled":false,"expected_revision":revision}),
        None,
    )
    .unwrap();
    wait_until(|| host.catalog()[0].lifecycle == ExtensionLifecycle::Disabled);
    assert_eq!(handler.cleanup_entered.load(Ordering::Acquire), 2);
    enable(&host);
    assert!(host.catalog()[0].enabled);
}

#[test]
fn retention_cleanup_racing_disable_finishes_once_and_allows_reenable() {
    let handler = Arc::new(TestHandler::default());
    handler.release.store(true, Ordering::Release);
    let store = Arc::new(MemoryStore::default());
    let host = service(store.clone(), handler.clone());
    enable(&host);
    let answer = call(&host, "work", json!({"text":"retained"}), Some("retained")).unwrap();
    let id = answer["id"].as_str().unwrap();
    wait_until(|| {
        host.job("akra.test", id).unwrap().state == ExtensionJobState::Completed
            && Arc::strong_count(&host) == 1
    });
    {
        let mut state = host.lock();
        let mut next = state.durable.clone();
        next.jobs.get_mut(id).unwrap().result_expires_at_ms = 1;
        host.persist(&mut state, next).unwrap();
    }
    let entered = Arc::new(std::sync::Barrier::new(2));
    let release = Arc::new(std::sync::Barrier::new(2));
    *handler.cleanup_barriers.lock().unwrap() = Some((entered.clone(), release.clone()));
    let sweeping = host.clone();
    let sweep = std::thread::spawn(move || sweeping.reap_expired());
    wait_until(|| handler.cleanup_entered.load(Ordering::Acquire) == 1);
    entered.wait();

    struct ReleaseCleanup(Arc<std::sync::Barrier>);
    impl Drop for ReleaseCleanup {
        fn drop(&mut self) {
            self.0.wait();
        }
    }
    let release_cleanup = ReleaseCleanup(release);
    let revision = host.catalog()[0].revision;
    call(
        &host,
        "configure",
        json!({"enabled":false,"expected_revision":revision}),
        None,
    )
    .unwrap();
    // The main thread and blocked sweep are the only owners once the disable
    // worker has observed the existing cleanup claim and returned. It must not
    // finish later and hide the missing sweep finalization in this regression.
    wait_until(|| Arc::strong_count(&host) == 2);
    assert_eq!(host.catalog()[0].lifecycle, ExtensionLifecycle::Disabling);
    assert_eq!(handler.cleanup_entered.load(Ordering::Acquire), 1);
    assert!(
        call(
            &host,
            "configure",
            json!({"enabled":true,"expected_revision":revision+1}),
            None,
        )
        .is_err()
    );
    drop(release_cleanup);
    sweep.join().unwrap().unwrap();

    assert_eq!(host.catalog()[0].lifecycle, ExtensionLifecycle::Disabled);
    assert_eq!(
        store.load().unwrap().activations["akra.test"].lifecycle,
        ExtensionLifecycle::Disabled
    );
    assert_eq!(
        host.job("akra.test", id).unwrap().cleanup,
        ExtensionCleanupState::Complete
    );
    assert!(host.lock().cleanup_claims.is_empty());
    host.reap_expired().unwrap();
    assert_eq!(handler.cleanup_entered.load(Ordering::Acquire), 1);
    assert_eq!(handler.cleanups.lock().unwrap().len(), 1);
    enable(&host);
    assert_eq!(
        call(&host, "echo", json!({"text":"enabled again"}), None).unwrap()["text"],
        "enabled again"
    );
}
