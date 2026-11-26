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
    let (actual, key_id) = match harness.next() {
        Notice::StoreWaiting { worker, key_id } => (worker, key_id),
        other => harness.unexpected("a paused credential store", &other),
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
            other => harness.unexpected("a credential store or worker result", &other),
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

    // Pending 때문에 보류된 호출도 별도 프로세스의 재조회로 이미 있는 키만 얻는다.
    harness.spawn(3, WorkerMode::Seal);
    let recovered = harness.result(3);
    assert_success(
        &mut harness,
        &recovered,
        snapshot.key_id.as_deref().unwrap(),
    );
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
    let worker = match harness.next() {
        Notice::BeforeStoreWaiting { worker } => worker,
        other => harness.unexpected(
            "the committed-intent checkpoint before credential STORE",
            &other,
        ),
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
