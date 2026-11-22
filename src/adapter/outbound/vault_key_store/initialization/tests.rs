use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use super::super::{KEY_STORE_ACCOUNT, KEY_STORE_SERVICE, UNAVAILABLE};
use super::*;

static NEXT_FIXTURE: AtomicUsize = AtomicUsize::new(1);

struct Fixture {
    base: PathBuf,
    runtime: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let base = std::env::temp_dir().join(format!(
            "akra-key-initialization-{}-{stamp}-{}",
            std::process::id(),
            NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed),
        ));
        crate::private_directory::ensure_private_directory(&base).unwrap();
        let base = std::fs::canonicalize(base).unwrap();
        Self {
            runtime: base.join("nested").join("runtime"),
            base,
        }
    }

    fn run(&self, credential: &dyn CredentialBackend) -> Result<VaultKey, VaultKeyStoreError> {
        super::super::test_support::get_or_create_with_store(&self.runtime, credential)
    }

    fn journal(&self) -> Journal {
        Journal::open(
            &self.runtime,
            scope_hash(KEY_STORE_SERVICE, KEY_STORE_ACCOUNT),
        )
        .unwrap()
    }

    fn state(&self) -> Option<State> {
        self.journal().snapshot().unwrap()
    }

    fn pending(&self, key: &VaultKey) -> State {
        let mut journal = self.journal();
        let transaction = journal.begin().unwrap();
        let state = transaction
            .pending(
                None,
                fingerprint(&scope_hash(KEY_STORE_SERVICE, KEY_STORE_ACCOUNT), key),
            )
            .unwrap();
        transaction.commit().unwrap();
        state
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.base);
    }
}

#[derive(Default)]
struct MemoryCredential {
    key: Mutex<Option<VaultKey>>,
    loads: AtomicUsize,
    stores: AtomicUsize,
    store_mode: AtomicUsize,
    load_failure: AtomicBool,
}

impl MemoryCredential {
    fn with_key(key: VaultKey) -> Self {
        Self {
            key: Mutex::new(Some(key)),
            ..Self::default()
        }
    }
}

impl CredentialBackend for MemoryCredential {
    fn load(&self) -> Result<Option<VaultKey>, VaultKeyStoreError> {
        self.loads.fetch_add(1, Ordering::SeqCst);
        if self.load_failure.load(Ordering::SeqCst) {
            return Err(VaultKeyStoreError::Unavailable(
                "synthetic-secret-error".to_string(),
            ));
        }
        Ok(self.key.lock().unwrap().clone())
    }

    fn store(&self, key: &VaultKey) -> Result<(), VaultKeyStoreError> {
        self.stores.fetch_add(1, Ordering::SeqCst);
        let mode = self.store_mode.load(Ordering::SeqCst);
        if mode != 2 {
            *self.key.lock().unwrap() = Some(key.clone());
        }
        if mode == 3 {
            self.load_failure.store(true, Ordering::SeqCst);
        }
        if mode == 1 {
            return Err(VaultKeyStoreError::Unavailable(
                "synthetic-secret-error".to_string(),
            ));
        }
        Ok(())
    }
}

fn assert_unavailable(result: Result<VaultKey, VaultKeyStoreError>) {
    assert_eq!(
        result.unwrap_err(),
        VaultKeyStoreError::Unavailable(UNAVAILABLE.to_string())
    );
}

#[test]
fn initialization_reuses_the_durable_key_without_persisting_key_bytes() {
    let fixture = Fixture::new();
    let credential = MemoryCredential::default();
    let first = fixture.run(&credential).unwrap();
    assert_eq!(first, fixture.run(&credential).unwrap());
    assert_eq!(credential.stores.load(Ordering::SeqCst), 1);
    let state = fixture.state().unwrap();
    assert_eq!(state.phase, Phase::Settled);
    assert_eq!(state.generation, 1);
    assert_eq!(
        state.fingerprint,
        fingerprint(&scope_hash(KEY_STORE_SERVICE, KEY_STORE_ACCOUNT), &first,)
    );
    assert_ne!(state.fingerprint.as_slice(), first.expose());
    for entry in std::fs::read_dir(&fixture.runtime).unwrap() {
        let bytes = std::fs::read(entry.unwrap().path()).unwrap();
        assert!(
            !bytes
                .windows(first.expose().len())
                .any(|window| window == first.expose())
        );
    }
}

#[test]
fn pending_missing_mismatched_and_failed_reads_never_send_another_key() {
    let fixture = Fixture::new();
    let pending = fixture.pending(&VaultKey::new(vec![7; 32]));
    let credential = MemoryCredential::default();
    assert_unavailable(fixture.run(&credential));
    *credential.key.lock().unwrap() = Some(VaultKey::new(vec![9; 32]));
    assert_unavailable(fixture.run(&credential));
    credential.load_failure.store(true, Ordering::SeqCst);
    assert_unavailable(fixture.run(&credential));
    assert_eq!(credential.stores.load(Ordering::SeqCst), 0);
    assert_eq!(fixture.state(), Some(pending));
}

#[test]
fn matching_readback_settles_pending_without_replaying_store() {
    let fixture = Fixture::new();
    let key = VaultKey::new(vec![7; 32]);
    let pending = fixture.pending(&key);
    let credential = MemoryCredential::with_key(key.clone());
    assert_eq!(fixture.run(&credential).unwrap(), key);
    assert_eq!(credential.stores.load(Ordering::SeqCst), 0);
    let state = fixture.state().unwrap();
    assert_eq!(state.generation, pending.generation);
    assert_eq!(state.phase, Phase::Settled);
}

#[test]
fn store_error_keeps_pending_and_retry_recovers_only_by_readback() {
    let fixture = Fixture::new();
    let credential = MemoryCredential::default();
    credential.store_mode.store(1, Ordering::SeqCst);
    assert_unavailable(fixture.run(&credential));
    assert_eq!(fixture.state().unwrap().phase, Phase::Pending);
    let durable = credential.key.lock().unwrap().clone().unwrap();
    assert_eq!(fixture.run(&credential).unwrap(), durable);
    assert_eq!(credential.stores.load(Ordering::SeqCst), 1);
    assert_eq!(fixture.state().unwrap().phase, Phase::Settled);
}

#[test]
fn missing_or_failed_readback_does_not_return_the_candidate() {
    for mode in [2, 3] {
        let fixture = Fixture::new();
        let credential = MemoryCredential::default();
        credential.store_mode.store(mode, Ordering::SeqCst);
        assert_unavailable(fixture.run(&credential));
        assert_unavailable(fixture.run(&credential));
        assert_eq!(credential.stores.load(Ordering::SeqCst), 1);
        assert_eq!(fixture.state().unwrap().phase, Phase::Pending);
    }
}

#[test]
fn settled_key_loss_uses_a_new_generation() {
    let fixture = Fixture::new();
    let credential = MemoryCredential::default();
    let before = fixture.run(&credential).unwrap();
    *credential.key.lock().unwrap() = None;
    let after = fixture.run(&credential).unwrap();
    assert_ne!(before, after);
    assert_eq!(credential.stores.load(Ordering::SeqCst), 2);
    assert_eq!(fixture.state().unwrap().generation, 2);
    assert_eq!(fixture.state().unwrap().phase, Phase::Settled);
}

#[test]
fn a_stale_none_is_discarded_after_another_initializer_finishes() {
    struct StaleReader<'a> {
        fixture: &'a Fixture,
        credential: MemoryCredential,
        first: AtomicBool,
    }
    impl CredentialBackend for StaleReader<'_> {
        fn load(&self) -> Result<Option<VaultKey>, VaultKeyStoreError> {
            if self.first.swap(false, Ordering::SeqCst) {
                self.fixture.run(&self.credential)?;
                return Ok(None);
            }
            self.credential.load()
        }
        fn store(&self, key: &VaultKey) -> Result<(), VaultKeyStoreError> {
            self.credential.store(key)
        }
    }
    let fixture = Fixture::new();
    let credential = StaleReader {
        fixture: &fixture,
        credential: MemoryCredential::default(),
        first: AtomicBool::new(true),
    };
    let key = fixture.run(&credential).unwrap();
    assert_eq!(credential.credential.stores.load(Ordering::SeqCst), 1);
    assert_eq!(Some(key), credential.credential.key.lock().unwrap().clone());
}

#[test]
fn repeated_generation_changes_stop_after_a_bounded_number_of_reads() {
    struct ChangingReader<'a> {
        fixture: &'a Fixture,
        loads: AtomicUsize,
    }
    impl CredentialBackend for ChangingReader<'_> {
        fn load(&self) -> Result<Option<VaultKey>, VaultKeyStoreError> {
            self.loads.fetch_add(1, Ordering::SeqCst);
            let mut journal = self.fixture.journal();
            let transaction = journal.begin()?;
            let previous = transaction.snapshot()?;
            let pending = transaction.pending(previous.as_ref(), [1; 32])?;
            transaction.settle(&pending)?;
            transaction.commit()?;
            Ok(None)
        }
        fn store(&self, _: &VaultKey) -> Result<(), VaultKeyStoreError> {
            panic!("a stale absence must never reach storage");
        }
    }
    let fixture = Fixture::new();
    let credential = ChangingReader {
        fixture: &fixture,
        loads: AtomicUsize::new(0),
    };
    assert_unavailable(fixture.run(&credential));
    assert_eq!(credential.loads.load(Ordering::SeqCst), MAX_STATE_RETRIES);
}

#[test]
fn unknown_schema_and_extra_objects_are_rejected_before_os_lookup() {
    for modification in [
        "PRAGMA user_version = 99;",
        "CREATE TABLE unexpected(value TEXT);",
    ] {
        let fixture = Fixture::new();
        drop(fixture.journal());
        let connection =
            rusqlite::Connection::open(fixture.runtime.join("key-initialization.db")).unwrap();
        connection.execute_batch(modification).unwrap();
        drop(connection);
        let credential = MemoryCredential::default();
        assert_unavailable(fixture.run(&credential));
        assert_eq!(credential.loads.load(Ordering::SeqCst), 0);
        assert_eq!(credential.stores.load(Ordering::SeqCst), 0);
    }
}

#[test]
fn an_invalid_existing_key_is_not_replaced() {
    let fixture = Fixture::new();
    let credential = MemoryCredential::with_key(VaultKey::new(vec![4; 31]));
    assert_unavailable(fixture.run(&credential));
    assert_eq!(credential.stores.load(Ordering::SeqCst), 0);
    assert_eq!(
        credential
            .key
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .expose()
            .len(),
        31
    );
}

#[test]
fn another_account_does_not_bypass_or_inherit_pending() {
    let fixture = Fixture::new();
    fixture.pending(&VaultKey::new(vec![7; 32]));
    let credential = MemoryCredential::default();
    let key = get_or_create(
        &fixture.runtime,
        KEY_STORE_SERVICE,
        "another-account",
        &credential,
    )
    .unwrap();
    assert_eq!(credential.stores.load(Ordering::SeqCst), 1);
    assert_unavailable(fixture.run(&credential));
    assert_eq!(credential.key.lock().unwrap().as_ref(), Some(&key));
    assert_ne!(scope_hash("ab", "c"), scope_hash("a", "bc"));
}
