use super::*;
use crate::domain::provider_credentials::Provider;
use std::sync::{Arc, Mutex};

#[derive(Default)]
struct BackendState {
    calls: Vec<std::thread::ThreadId>,
    entries: Vec<(KeyringPart, Zeroizing<Vec<u8>>)>,
    oversized: bool,
}
struct FakeBackend(Arc<Mutex<BackendState>>);
impl ProviderKeyStoreBackend for FakeBackend {
    fn read_part(&mut self, part: &KeyringPart) -> CredentialResult<Option<Zeroizing<Vec<u8>>>> {
        let mut state = self.0.lock().unwrap();
        state.calls.push(std::thread::current().id());
        if state.oversized {
            return Ok(Some(Zeroizing::new(vec![0; MAX_CAPSULE_BYTES + 1])));
        }
        Ok(state
            .entries
            .iter()
            .find(|(key, _)| key == part)
            .map(|(_, value)| Zeroizing::new(value.to_vec())))
    }
    fn write_part(&mut self, part: &KeyringPart, capsule: &[u8]) -> CredentialResult<()> {
        let mut state = self.0.lock().unwrap();
        state.calls.push(std::thread::current().id());
        state.entries.retain(|(key, _)| key != part);
        state
            .entries
            .push((part.clone(), Zeroizing::new(capsule.to_vec())));
        Ok(())
    }
    fn delete_part(&mut self, part: &KeyringPart) -> CredentialResult<()> {
        let mut state = self.0.lock().unwrap();
        state.calls.push(std::thread::current().id());
        state.entries.retain(|(key, _)| key != part);
        Ok(())
    }
}
fn part(index: u16) -> KeyringPart {
    KeyringPart {
        profile_id: "a".repeat(64),
        provider: Provider::Linear,
        connection_id: "b".repeat(32),
        index,
    }
}

#[test]
fn constructor_does_no_backend_io_and_concurrent_calls_share_one_worker_thread() {
    let state = Arc::new(Mutex::new(BackendState::default()));
    let store = Arc::new(
        OsProviderKeyStoreAdapter::with_backend(Box::new(FakeBackend(state.clone()))).unwrap(),
    );
    assert!(state.lock().unwrap().calls.is_empty());
    let workers: Vec<_> = (0..8)
        .map(|index| {
            let store = store.clone();
            std::thread::spawn(move || {
                let key = part(index);
                store.write_part(&key, b"synthetic-capsule").unwrap();
                assert_eq!(
                    store.read_part(&key).unwrap().unwrap().as_slice(),
                    b"synthetic-capsule"
                );
                store.delete_part(&key).unwrap();
                assert!(store.read_part(&key).unwrap().is_none());
            })
        })
        .collect();
    for worker in workers {
        worker.join().unwrap();
    }
    let state = state.lock().unwrap();
    assert_eq!(state.calls.len(), 32);
    let thread = state.calls[0];
    assert_ne!(thread, std::thread::current().id());
    assert!(state.calls.iter().all(|current| *current == thread));
    assert!(state.entries.is_empty());
}

#[test]
fn malformed_slot_names_and_oversized_blobs_are_rejected_before_backend_access() {
    let state = Arc::new(Mutex::new(BackendState::default()));
    let store =
        OsProviderKeyStoreAdapter::with_backend(Box::new(FakeBackend(state.clone()))).unwrap();
    let mut invalid = part(0);
    invalid.profile_id = "../another-profile".into();
    assert_eq!(
        store.write_part(&invalid, b"value"),
        Err(ProviderCredentialError::InvalidRequest)
    );
    assert!(store.read_part(&invalid).is_err());
    assert!(store.delete_part(&invalid).is_err());
    assert_eq!(
        store.write_part(&part(0), &vec![0; MAX_CAPSULE_BYTES + 1]),
        Err(ProviderCredentialError::TooLarge)
    );
    assert!(
        store
            .write_part(&part(MAX_SECRET_PARTS as u16), b"value")
            .is_err()
    );
    assert!(state.lock().unwrap().calls.is_empty());
}

#[test]
fn oversized_backend_result_is_not_exposed_to_the_application() {
    let state = Arc::new(Mutex::new(BackendState {
        oversized: true,
        ..BackendState::default()
    }));
    let store = OsProviderKeyStoreAdapter::with_backend(Box::new(FakeBackend(state))).unwrap();
    assert!(matches!(
        store.read_part(&part(0)),
        Err(ProviderCredentialError::Corrupt)
    ));
}

#[test]
fn raw_keyring_errors_never_enter_the_safe_error_contract() {
    let error = keyring_error(keyring::Error::Invalid(
        "field".into(),
        "synthetic-sensitive-message".into(),
    ));
    assert_eq!(error, ProviderCredentialError::Unavailable);
    assert!(!error.to_string().contains("synthetic-sensitive-message"));
    assert_eq!(
        keyring_error(keyring::Error::BadEncoding(b"synthetic-secret".to_vec())),
        ProviderCredentialError::Corrupt
    );
}

struct StalledBackend {
    state: Arc<Mutex<BackendState>>,
    entered: std::sync::mpsc::SyncSender<()>,
    release: std::sync::mpsc::Receiver<()>,
    stalled: bool,
}
impl ProviderKeyStoreBackend for StalledBackend {
    fn read_part(&mut self, part: &KeyringPart) -> CredentialResult<Option<Zeroizing<Vec<u8>>>> {
        FakeBackend(self.state.clone()).read_part(part)
    }
    fn write_part(&mut self, part: &KeyringPart, capsule: &[u8]) -> CredentialResult<()> {
        if !self.stalled {
            self.stalled = true;
            self.entered.send(()).unwrap();
            self.release.recv_timeout(Duration::from_secs(15)).unwrap();
        }
        FakeBackend(self.state.clone()).write_part(part, capsule)
    }
    fn delete_part(&mut self, part: &KeyringPart) -> CredentialResult<()> {
        FakeBackend(self.state.clone()).delete_part(part)
    }
}

#[test]
fn timed_out_os_write_is_unknown_and_expired_or_cancelled_queued_writes_are_skipped() {
    let state = Arc::new(Mutex::new(BackendState::default()));
    let (entered, observed) = mpsc::sync_channel(1);
    let (release, wait) = mpsc::sync_channel(1);
    let store = Arc::new(
        OsProviderKeyStoreAdapter::with_backend_timeout(
            Box::new(StalledBackend {
                state: state.clone(),
                entered,
                release: wait,
                stalled: false,
            }),
            Duration::from_millis(200),
        )
        .unwrap(),
    );
    let first = {
        let store = store.clone();
        std::thread::spawn(move || store.write_part(&part(0), b"late-owned-slot"))
    };
    observed.recv_timeout(Duration::from_secs(5)).unwrap();
    assert_eq!(
        first.join().unwrap(),
        Err(ProviderCredentialError::OutcomeUnknown)
    );

    // This request expires while still queued behind the blocked first OS call.
    assert_eq!(
        store.write_part(&part(1), b"never-submit-expired"),
        Err(ProviderCredentialError::OutcomeUnknown)
    );

    // Put a cancellation-controlled job in the queue deterministically, without a timing sleep.
    let cancelled = Arc::new(AtomicBool::new(false));
    let context = JobContext {
        cancelled: cancelled.clone(),
        abandoned: Arc::new(AtomicBool::new(false)),
        deadline: Instant::now() + Duration::from_secs(5),
    };
    let (reply, answer) = mpsc::sync_channel(1);
    assert!(
        store
            .sender
            .try_send(Command::Write(
                part(2),
                Zeroizing::new(b"never-submit-cancelled".to_vec()),
                context,
                reply
            ))
            .is_ok()
    );
    cancelled.store(true, Ordering::SeqCst);
    release.send(()).unwrap();
    assert_eq!(
        answer.recv_timeout(Duration::from_secs(5)).unwrap(),
        Err(ProviderCredentialError::OutcomeUnknown)
    );
    assert_eq!(
        store.read_part(&part(0)).unwrap().unwrap().as_slice(),
        b"late-owned-slot"
    );
    assert!(store.read_part(&part(1)).unwrap().is_none());
    assert!(store.read_part(&part(2)).unwrap().is_none());
}

#[test]
fn dropping_an_adapter_does_not_join_an_uninterruptible_backend_call() {
    let state = Arc::new(Mutex::new(BackendState::default()));
    let (entered, observed) = mpsc::sync_channel(1);
    let (release, wait) = mpsc::sync_channel(1);
    let store = Arc::new(
        OsProviderKeyStoreAdapter::with_backend_timeout(
            Box::new(StalledBackend {
                state,
                entered,
                release: wait,
                stalled: false,
            }),
            Duration::from_millis(100),
        )
        .unwrap(),
    );
    let caller = {
        let store = store.clone();
        std::thread::spawn(move || store.write_part(&part(0), b"late-owned-slot"))
    };
    observed.recv_timeout(Duration::from_secs(5)).unwrap();
    assert_eq!(
        caller.join().unwrap(),
        Err(ProviderCredentialError::OutcomeUnknown)
    );
    let before = Instant::now();
    drop(store);
    assert!(before.elapsed() < Duration::from_secs(1));
    release.send(()).unwrap();
}

/// #2588: existing provider credentials live under this OS key store service. A product rename must not change it.
#[test]
fn key_store_service_stays_the_same_across_the_product_rename() {
    assert_eq!(SERVICE, "akra-provider-credentials-v1");
}
