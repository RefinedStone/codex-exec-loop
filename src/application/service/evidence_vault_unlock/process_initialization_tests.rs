//! 여러 프로세스가 같은 credential과 durable 초기화 intent를 사용한다.
//! 실제 사용자 keyring/모델/네트워크 provider는 호출하지 않는다.
mod fixture;

use fixture::{Harness, Notice, WorkerMode, WorkerResult};

#[test]
fn credential_server_process() {
    fixture::run_server();
}

#[test]
fn sealing_worker_process() {
    fixture::run_worker();
}

fn expect_store(harness: &mut Harness, worker: u8) -> String {
    let Notice::StoreWaiting {
        worker: actual,
        key_id,
    } = harness.next()
    else {
        panic!("expected a paused credential store")
    };
    assert_eq!(actual, worker);
    key_id
}

fn assert_unavailable(result: &WorkerResult) {
    assert_eq!(result.failure.as_deref(), Some("KeyStoreUnavailable"));
    assert!(result.key_id.is_none());
    assert!(result.ciphertext.is_none());
}

fn assert_success(harness: &mut Harness, result: &WorkerResult, key_id: &str) {
    assert!(result.failure.is_none());
    assert_eq!(result.key_id.as_deref(), Some(key_id));
    assert!(
        harness.opens(result),
        "current credential must open the successful ciphertext"
    );
}

#[test]
fn two_processes_share_one_durable_initialization_key() {
    let mut harness = Harness::new(2);
    harness.spawn(1, WorkerMode::Seal);
    harness.spawn(2, WorkerMode::Seal);
    harness.release_loads(&[1, 2]);
    let mut results = Vec::new();
    while results.len() < 2 {
        match harness.next() {
            Notice::StoreWaiting { .. } => harness.apply_next_store(true),
            Notice::Finished(result) => results.push(result),
            notice => panic!("unexpected initialization notice: {}", notice.summary()),
        }
    }
    harness.wait_worker(1);
    harness.wait_worker(2);
    let snapshot = harness.snapshot();
    let returned_ids: Vec<_> = results
        .iter()
        .filter_map(|result| result.key_id.as_deref())
        .collect();
    let mut restored = Vec::new();
    for result in &results {
        if result.key_id.is_some() {
            restored.push(harness.opens(result));
        } else {
            assert_unavailable(result);
        }
    }
    println!(
        "synthetic durable snapshot={snapshot:?}; returned key_ids={returned_ids:?}; ciphertext opens={restored:?}"
    );
    assert_eq!(
        snapshot.store_requests, 1,
        "only one initializer may ask the credential server to write"
    );
    assert_eq!(
        snapshot.applied_stores, 1,
        "concurrent first sealing must commit only one device key"
    );
    assert_eq!(snapshot.pending_stores, 0);
    assert!(
        !returned_ids.is_empty(),
        "at least one initializer must finish"
    );
    assert!(
        returned_ids
            .iter()
            .all(|id| Some(*id) == snapshot.key_id.as_deref())
    );
    assert!(restored.iter().all(|matches| *matches));

    // 이미 초기화된 journal도 독립 프로세스 둘이 동시에 재조회한다.
    harness.spawn(3, WorkerMode::Seal);
    harness.spawn(4, WorkerMode::Seal);
    let mut recovered = Vec::new();
    while recovered.len() < 2 {
        match harness.next() {
            Notice::Finished(result) => recovered.push(result),
            notice => panic!(
                "warm initialization unexpectedly wrote: {}",
                notice.summary()
            ),
        }
    }
    harness.wait_worker(3);
    harness.wait_worker(4);
    let mut workers: Vec<_> = recovered.iter().map(|result| result.worker).collect();
    workers.sort_unstable();
    assert_eq!(workers, vec![3, 4]);
    for result in &recovered {
        assert_success(&mut harness, result, snapshot.key_id.as_deref().unwrap());
    }
    let after = harness.snapshot();
    assert_eq!(after.store_requests, 1);
    assert_eq!(after.applied_stores, 1);
}

#[test]
fn a_delayed_store_survives_worker_death_without_allowing_another_store() {
    let mut harness = Harness::new(1);
    harness.spawn(1, WorkerMode::Seal);
    harness.release_loads(&[1]);
    let expected = expect_store(&mut harness, 1);
    let queued = harness.snapshot();
    assert_eq!(
        (
            queued.store_requests,
            queued.applied_stores,
            queued.pending_stores
        ),
        (1, 0, 1)
    );
    assert!(queued.key_id.is_none());

    harness.spawn(2, WorkerMode::Seal);
    assert_unavailable(&harness.result(2));
    harness.kill_worker(1);
    // kill과 wait가 끝난 뒤에도 독립 서버에 원래 요청과 stream이 남아 있다.
    let killed = harness.snapshot();
    assert_eq!(
        (
            killed.store_requests,
            killed.applied_stores,
            killed.pending_stores
        ),
        (1, 0, 1)
    );
    assert!(killed.key_id.is_none());
    harness.spawn(3, WorkerMode::Seal);
    assert_unavailable(&harness.result(3));
    let still_queued = harness.snapshot();
    assert_eq!(
        (
            still_queued.store_requests,
            still_queued.applied_stores,
            still_queued.pending_stores
        ),
        (1, 0, 1)
    );

    harness.apply_next_store(true);
    let applied = harness.snapshot();
    assert_eq!(
        (
            applied.store_requests,
            applied.applied_stores,
            applied.pending_stores
        ),
        (1, 1, 0)
    );
    assert_eq!(applied.key_id.as_deref(), Some(expected.as_str()));
    harness.spawn(4, WorkerMode::Seal);
    let recovered = harness.result(4);
    assert_success(&mut harness, &recovered, &expected);
    let after = harness.snapshot();
    assert_eq!(after.store_requests, 1);
    assert_eq!(after.applied_stores, 1);
}

#[test]
fn a_lost_store_ack_is_recovered_by_reading_without_repeating_the_write() {
    let mut harness = Harness::new(1);
    harness.spawn(1, WorkerMode::Seal);
    harness.release_loads(&[1]);
    let expected = expect_store(&mut harness, 1);
    harness.apply_next_store(false);
    let original = harness.result(1);
    if original.key_id.is_some() {
        assert_success(&mut harness, &original, &expected);
    } else {
        assert_unavailable(&original);
    }
    let snapshot = harness.snapshot();
    assert_eq!(
        (
            snapshot.store_requests,
            snapshot.applied_stores,
            snapshot.pending_stores
        ),
        (1, 1, 0)
    );
    harness.spawn(2, WorkerMode::Seal);
    let recovered = harness.result(2);
    assert_success(&mut harness, &recovered, &expected);
    let after = harness.snapshot();
    assert_eq!(after.store_requests, 1);
    assert_eq!(after.applied_stores, 1);
}

#[test]
fn death_after_durable_intent_but_before_store_rpc_holds_with_zero_store_requests() {
    let mut harness = Harness::new(1);
    harness.spawn(1, WorkerMode::PauseBeforeStore);
    harness.release_loads(&[1]);
    let Notice::BeforeStoreWaiting { worker } = harness.next() else {
        panic!("expected the committed-intent checkpoint before credential STORE")
    };
    assert_eq!(worker, 1);
    harness.kill_worker(1);
    let killed = harness.snapshot();
    assert_eq!(
        (
            killed.store_requests,
            killed.applied_stores,
            killed.pending_stores
        ),
        (0, 0, 0)
    );
    assert_eq!(killed.before_store_waiters, 1);
    assert!(killed.key_id.is_none());

    harness.spawn(2, WorkerMode::Seal);
    assert_unavailable(&harness.result(2));
    let after = harness.snapshot();
    assert_eq!(
        (
            after.store_requests,
            after.applied_stores,
            after.pending_stores
        ),
        (0, 0, 0)
    );
    assert!(after.key_id.is_none());
}

#[test]
fn construction_and_native_key_reads_do_not_create_coordination_state() {
    let mut harness = Harness::new(0);
    harness.assert_runtime_empty();
    harness.spawn(1, WorkerMode::ConstructOnly);
    let constructed = harness.result(1);
    assert!(constructed.failure.is_none());
    harness.assert_runtime_empty();
    let before = harness.snapshot();
    assert_eq!(
        (before.loads, before.store_requests, before.applied_stores),
        (0, 0, 0)
    );

    harness.spawn(2, WorkerMode::ReadOnly);
    let read = harness.result(2);
    assert!(read.failure.is_none());
    assert!(read.key_id.is_none());
    harness.assert_runtime_empty();
    let after = harness.snapshot();
    assert_eq!(
        (after.loads, after.store_requests, after.applied_stores),
        (1, 0, 0)
    );
}

#[test]
fn early_worker_failure_identifies_the_initial_load_barrier_and_journal_stage() {
    let mut harness = Harness::new(1);
    harness.block_runtime_with_file();
    harness.spawn(1, WorkerMode::Seal);
    let error = harness.try_release_loads(&[1]).unwrap_err();
    assert!(error.contains("initial_load_barrier"), "{error}");
    assert!(error.contains("Finished worker=1"), "{error}");
    assert!(error.contains("KeyStoreUnavailable"), "{error}");
    assert!(error.contains("journal_open"), "{error}");
    assert!(error.contains("failed"), "{error}");
    assert!(!error.contains("synthetic-runtime-blocker"));
    harness.wait_worker(1);
    let snapshot = harness.snapshot();
    assert_eq!(
        (
            snapshot.loads,
            snapshot.store_requests,
            snapshot.applied_stores
        ),
        (0, 0, 0)
    );
}

#[test]
fn unexpected_notice_diagnostics_exclude_key_identifiers_and_ciphertext() {
    let notice = Notice::Finished(WorkerResult {
        worker: 9,
        key_id: Some("synthetic-secret-key-id".into()),
        ciphertext: Some(b"synthetic-secret-ciphertext".to_vec()),
        failure: Some("synthetic-secret-provider-message".into()),
        diagnostics: Vec::new(),
    });
    let summary = notice.summary();
    assert!(summary.contains("Finished worker=9"));
    assert!(summary.contains("unexpected_failure"));
    assert!(!summary.contains("synthetic-secret"));
}
