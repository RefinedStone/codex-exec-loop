use super::*;
use std::sync::Barrier;

fn fill_retained_jobs(host: &ExtensionService) -> BTreeMap<String, Value> {
    let mut state = host.lock();
    let mut next = state.durable.clone();
    let count = MAX_JOBS - next.jobs.len();
    let now = now_ms();
    let mut retained = BTreeMap::new();
    for index in 0..count {
        let id = format!("retained-{index}");
        let job = ExtensionJob {
            id: id.clone(),
            extension: "akra.test".into(),
            action: "work".into(),
            state: ExtensionJobState::Completed,
            generation: host.generation().into(),
            revision: 3,
            sequence: 1,
            progress: json!({"text":"retained progress"}),
            result: Some(json!({"text":"retained result"})),
            error: None,
            cleanup: ExtensionCleanupState::Pending,
            created_at_ms: now,
            updated_at_ms: now,
            result_expires_at_ms: now + retention::RESULT_RETENTION_MS,
            dedupe_key: None,
        };
        retained.insert(id.clone(), serde_json::to_value(&job).unwrap());
        next.jobs.insert(id, job);
    }
    host.persist(&mut state, next).unwrap();
    retained
}

fn assert_retained(host: &ExtensionService, retained: &BTreeMap<String, Value>) {
    let state = host.lock();
    for (id, expected) in retained {
        assert_eq!(
            serde_json::to_value(state.durable.jobs.get(id).expect("retained result")).unwrap(),
            *expected,
            "{id} was changed by an admission that did not need its result"
        );
    }
}

#[test]
fn full_retention_dedupe_and_active_limit_rejection_preserve_all_results() {
    let handler = Arc::new(TestHandler::default());
    let host = service(Arc::new(MemoryStore::default()), handler.clone());
    enable(&host);
    let mut active = Vec::new();
    for index in 0..MAX_ACTIVE_JOBS {
        active.push(call(&host, "work", json!({"text":format!("work-{index}")}), None).unwrap());
    }
    wait_until(|| handler.entered.load(Ordering::Acquire) == MAX_ACTIVE_JOBS);
    let retained = fill_retained_jobs(&host);
    let barrier = Barrier::new(4);
    std::thread::scope(|scope| {
        let mut retries = Vec::new();
        for index in 0..4 {
            let barrier = &barrier;
            let host = &host;
            retries.push(scope.spawn(move || {
                barrier.wait();
                call(
                    host,
                    "work",
                    json!({"text":"work-0"}),
                    Some(&format!("retry-{index}")),
                )
                .unwrap()
            }));
        }
        for retry in retries {
            assert_eq!(retry.join().unwrap()["id"], active[0]["id"]);
        }
    });
    let error = call(&host, "work", json!({"text":"ninth"}), None).unwrap_err();
    assert!(error.to_string().contains("concurrent job limit"));
    assert_retained(&host, &retained);
    assert_eq!(handler.cleanup_entered.load(Ordering::Acquire), 0);
    assert_eq!(host.lock().durable.jobs.len(), MAX_JOBS);
    handler.release.store(true, Ordering::Release);
    wait_until(|| !host.busy());
}

#[test]
fn full_unexpired_retention_rejects_new_work_without_shortening_result_lifetime() {
    let handler = Arc::new(TestHandler::default());
    let host = service(Arc::new(MemoryStore::default()), handler.clone());
    enable(&host);
    let retained = fill_retained_jobs(&host);
    let error = call(&host, "work", json!({"text":"new"}), Some("new")).unwrap_err();
    assert!(error.to_string().contains("retention limit"));
    assert_retained(&host, &retained);
    assert_eq!(handler.cleanup_entered.load(Ordering::Acquire), 0);
    assert_eq!(handler.entered.load(Ordering::Acquire), 0);
    assert!(host.lock().durable.receipts.is_empty());
}

#[test]
fn cleaned_metadata_retirement_and_new_admission_share_one_durable_commit() {
    let handler = Arc::new(TestHandler::default());
    handler.release.store(true, Ordering::Release);
    let store = Arc::new(MemoryStore::default());
    let host = service(store.clone(), handler.clone());
    enable(&host);
    let original = call(&host, "work", json!({"text":"original"}), Some("original")).unwrap();
    let old_id = original["id"].as_str().unwrap();
    wait_until(|| host.job("akra.test", old_id).unwrap().state == ExtensionJobState::Completed);
    {
        let mut state = host.lock();
        let mut next = state.durable.clone();
        next.jobs.get_mut(old_id).unwrap().result_expires_at_ms = 1;
        host.persist(&mut state, next).unwrap();
    }
    host.reap_expired().unwrap();
    assert_eq!(
        host.job("akra.test", old_id).unwrap().cleanup,
        ExtensionCleanupState::Complete
    );
    let retained = fill_retained_jobs(&host);
    let before = serde_json::to_value(store.load().unwrap()).unwrap();
    store.fail.store(true, Ordering::Release);
    assert!(
        call(
            &host,
            "work",
            json!({"text":"not-persisted"}),
            Some("failed")
        )
        .is_err()
    );
    assert_eq!(serde_json::to_value(store.load().unwrap()).unwrap(), before);
    assert_eq!(host.lock().durable.jobs.len(), MAX_JOBS);
    assert!(host.job("akra.test", old_id).is_ok());
    store.fail.store(false, Ordering::Release);

    let barrier = Barrier::new(2);
    let accepted = std::thread::scope(|scope| {
        let first = scope.spawn(|| {
            barrier.wait();
            call(&host, "work", json!({"text":"first"}), Some("first"))
        });
        let second = scope.spawn(|| {
            barrier.wait();
            call(&host, "work", json!({"text":"second"}), Some("second"))
        });
        let results = [first.join().unwrap(), second.join().unwrap()];
        assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
        results.into_iter().find_map(Result::ok).unwrap()
    });
    wait_until(|| {
        host.job("akra.test", accepted["id"].as_str().unwrap())
            .unwrap()
            .state
            == ExtensionJobState::Completed
    });
    assert_retained(&host, &retained);
    assert_eq!(host.lock().durable.jobs.len(), MAX_JOBS);
    assert!(host.job("akra.test", old_id).is_err());
    assert!(
        host.lock()
            .durable
            .receipts
            .contains_key("akra.test:original")
    );
    let retry = call(&host, "work", json!({"text":"original"}), Some("original")).unwrap_err();
    assert!(retry.to_string().contains("receipt has expired"));
    assert_eq!(handler.entered.load(Ordering::Acquire), 2);
    assert_eq!(handler.cleanup_entered.load(Ordering::Acquire), 1);
}
