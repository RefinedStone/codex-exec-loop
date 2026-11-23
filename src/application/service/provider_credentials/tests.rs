use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, SyncSender};
use std::time::{Duration, Instant};

use zeroize::Zeroizing;

use super::*;

const PROFILE: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

struct Gate {
    entered: SyncSender<()>,
    release: Mutex<Receiver<()>>,
}
impl Gate {
    fn pair() -> (Arc<Self>, Receiver<()>, SyncSender<()>) {
        let (entered, observed) = mpsc::sync_channel(1);
        let (release, wait) = mpsc::sync_channel(1);
        (
            Arc::new(Self {
                entered,
                release: Mutex::new(wait),
            }),
            observed,
            release,
        )
    }
    fn wait(&self) {
        self.entered.send(()).unwrap();
        self.release
            .lock()
            .unwrap()
            .recv_timeout(Duration::from_secs(15))
            .unwrap();
    }
}

#[derive(Default)]
struct MemoryLedger {
    rows: Mutex<HashMap<Provider, CredentialLedger>>,
    before_publish: Mutex<Option<Arc<Gate>>>,
    before_begin: Mutex<Option<Arc<Gate>>>,
}
impl ProviderCredentialLedgerPort for MemoryLedger {
    fn load(&self, provider: Provider) -> CredentialResult<CredentialLedger> {
        Ok(self
            .rows
            .lock()
            .unwrap()
            .get(&provider)
            .cloned()
            .unwrap_or_else(|| CredentialLedger::empty(PROFILE.into(), provider)))
    }
    fn compare_exchange_guarded(
        &self,
        expected: &CredentialLedger,
        next: &CredentialLedger,
        allowed: &dyn Fn() -> bool,
    ) -> CredentialResult<bool> {
        if next.operations.len() > expected.operations.len()
            && next
                .operations
                .last()
                .is_some_and(|operation| operation.receipt.kind == CredentialOperationKind::Connect)
        {
            let gate = self.before_begin.lock().unwrap().take();
            if let Some(gate) = gate {
                gate.wait();
            }
        }
        if next.active.is_some() && next.active != expected.active {
            let gate = self.before_publish.lock().unwrap().take();
            if let Some(gate) = gate {
                gate.wait();
            }
        }
        let mut rows = self.rows.lock().unwrap();
        let current = rows
            .get(&expected.provider)
            .cloned()
            .unwrap_or_else(|| CredentialLedger::empty(PROFILE.into(), expected.provider));
        if &current != expected {
            return Ok(false);
        }
        if !allowed() {
            return Err(ProviderCredentialError::Cancelled);
        }
        next.validate()?;
        rows.insert(next.provider, next.clone());
        Ok(true)
    }
}

#[derive(Default)]
struct FakeStore {
    values: Mutex<Vec<(KeyringPart, Zeroizing<Vec<u8>>)>>,
    reads: AtomicUsize,
    writes: AtomicUsize,
    deletes: AtomicUsize,
    fail_write: Mutex<Option<(u16, bool)>>,
    fail_delete: AtomicBool,
    fail_read: AtomicBool,
    write_gate: Mutex<Option<Arc<Gate>>>,
    delete_gate: Mutex<Option<Arc<Gate>>>,
}
impl ProviderKeyStorePort for FakeStore {
    fn read_part(&self, part: &KeyringPart) -> CredentialResult<Option<Zeroizing<Vec<u8>>>> {
        self.reads.fetch_add(1, Ordering::SeqCst);
        if self.fail_read.load(Ordering::SeqCst) {
            return Err(ProviderCredentialError::Locked);
        }
        Ok(self
            .values
            .lock()
            .unwrap()
            .iter()
            .find(|(key, _)| key == part)
            .map(|(_, value)| Zeroizing::new(value.to_vec())))
    }
    fn write_part(&self, part: &KeyringPart, capsule: &[u8]) -> CredentialResult<()> {
        self.writes.fetch_add(1, Ordering::SeqCst);
        let gate = self.write_gate.lock().unwrap().take();
        if let Some(gate) = gate {
            gate.wait();
        }
        let failure = *self.fail_write.lock().unwrap();
        if failure == Some((part.index, false)) {
            return Err(ProviderCredentialError::Unavailable);
        }
        let mut values = self.values.lock().unwrap();
        values.retain(|(key, _)| key != part);
        values.push((part.clone(), Zeroizing::new(capsule.to_vec())));
        if failure == Some((part.index, true)) {
            return Err(ProviderCredentialError::OutcomeUnknown);
        }
        Ok(())
    }
    fn delete_part(&self, part: &KeyringPart) -> CredentialResult<()> {
        self.deletes.fetch_add(1, Ordering::SeqCst);
        let gate = self.delete_gate.lock().unwrap().take();
        if let Some(gate) = gate {
            gate.wait();
        }
        if self.fail_delete.load(Ordering::SeqCst) {
            return Err(ProviderCredentialError::Unavailable);
        }
        self.values.lock().unwrap().retain(|(key, _)| key != part);
        Ok(())
    }
}

struct FakePrompt {
    token: Mutex<Vec<u8>>,
    calls: AtomicUsize,
    confirms: AtomicUsize,
    consent: AtomicBool,
}
impl FakePrompt {
    fn new() -> Self {
        Self {
            token: Mutex::new(b"synthetic-provider-token".to_vec()),
            calls: AtomicUsize::new(0),
            confirms: AtomicUsize::new(0),
            consent: AtomicBool::new(true),
        }
    }
}
impl NativePromptPort for FakePrompt {
    fn prompt(
        &self,
        request: &ProviderCredentialPromptRequest,
        _: &AtomicBool,
        _: Instant,
    ) -> CredentialResult<SecretToken> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        SecretToken::from_bytes(request.provider, self.token.lock().unwrap().clone())
    }
    fn confirm_migration(
        &self,
        _: &LegacyImportOffer,
        _: &AtomicBool,
        _: Instant,
    ) -> CredentialResult<bool> {
        self.confirms.fetch_add(1, Ordering::SeqCst);
        Ok(self.consent.load(Ordering::SeqCst))
    }
}
#[derive(Default)]
struct FakeVerifier {
    calls: AtomicUsize,
    reject: AtomicBool,
}
impl ProviderAccountVerifierPort for FakeVerifier {
    fn verify(
        &self,
        request: &ConnectRequest,
        _: &SecretToken,
        _: &AtomicBool,
        _: Instant,
    ) -> CredentialResult<ConnectionMetadata> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if self.reject.load(Ordering::SeqCst) {
            return Err(ProviderCredentialError::Unauthorized);
        }
        Ok(ConnectionMetadata {
            configuration: request.configuration.clone(),
            account_id: "account-1".into(),
            account_name: "Test Account".into(),
        })
    }
}

struct Harness {
    service: Arc<ProviderCredentialService>,
    ledger: Arc<MemoryLedger>,
    store: Arc<FakeStore>,
    prompt: Arc<FakePrompt>,
    verifier: Arc<FakeVerifier>,
}
impl Harness {
    fn new() -> Self {
        let ledger = Arc::new(MemoryLedger::default());
        let store = Arc::new(FakeStore::default());
        let prompt = Arc::new(FakePrompt::new());
        let verifier = Arc::new(FakeVerifier::default());
        let service = Arc::new(
            ProviderCredentialService::new(
                PROFILE.into(),
                ledger.clone(),
                store.clone(),
                prompt.clone(),
                verifier.clone(),
            )
            .unwrap(),
        );
        Self {
            service,
            ledger,
            store,
            prompt,
            verifier,
        }
    }
    fn connect(&self, id: &str, revision: u64) -> OperationReceipt {
        self.service
            .start_connect(request(id, revision), owner(id))
            .unwrap()
    }
    fn active(&self) -> ConnectionRef {
        self.service
            .state(Provider::Linear)
            .unwrap()
            .active_connection
            .unwrap()
            .reference
    }
}
fn request(id: &str, revision: u64) -> ConnectRequest {
    ConnectRequest {
        operation_id: id.into(),
        profile_id: PROFILE.into(),
        provider: Provider::Linear,
        expected_revision: revision,
        configuration: ConnectionConfiguration::default(),
    }
}
fn operation(id: &str) -> CredentialOperationRequest {
    CredentialOperationRequest {
        operation_id: id.into(),
        profile_id: PROFILE.into(),
        provider: Provider::Linear,
    }
}
fn owner(id: &str) -> NativeOperationOwner {
    NativeOperationOwner::new(
        format!("owner-{id}"),
        Instant::now() + Duration::from_secs(30),
    )
    .unwrap()
}
fn disconnect(id: &str, revision: u64) -> DisconnectRequest {
    DisconnectRequest {
        operation_id: id.into(),
        profile_id: PROFILE.into(),
        provider: Provider::Linear,
        expected_revision: revision,
    }
}
fn offer(id: &str) -> LegacyImportOffer {
    LegacyImportOffer {
        operation_id: id.into(),
        profile_id: PROFILE.into(),
        provider: Provider::Linear,
        expected_revision: 0,
        configuration: ConnectionConfiguration::default(),
        snapshot_id: "b".repeat(64),
    }
}
fn token() -> SecretToken {
    SecretToken::from_bytes(Provider::Linear, b"synthetic-provider-token".to_vec())
        .unwrap_or_else(|_| panic!("fixture token"))
}

#[test]
fn successful_connect_and_same_id_replay_prompt_and_store_only_once() {
    let harness = Harness::new();
    let first = harness.connect("connect", 0);
    assert_eq!(first.status, CredentialOperationStatus::Committed);
    assert_eq!(first.connection, Some(harness.active()));
    let second = harness.connect("connect", 0);
    assert_eq!(first, second);
    assert_eq!(harness.prompt.calls.load(Ordering::SeqCst), 1);
    assert_eq!(harness.verifier.calls.load(Ordering::SeqCst), 1);
    assert_eq!(harness.store.writes.load(Ordering::SeqCst), 1);
    let lease = harness
        .service
        .acquire(&harness.active())
        .unwrap_or_else(|_| panic!("current credentials"));
    assert_eq!(lease.token.expose(), b"synthetic-provider-token");
}

#[test]
fn state_and_operation_reads_are_ledger_only_and_cas_events_are_secret_free() {
    let harness = Harness::new();
    let mut changes = harness.service.subscribe();
    harness.connect("connect", 0);
    let reads = harness.store.reads.load(Ordering::SeqCst);
    let _ = harness.service.state(Provider::Linear).unwrap();
    let _ = harness.service.operation(operation("connect")).unwrap();
    assert_eq!(harness.store.reads.load(Ordering::SeqCst), reads);
    let mut count = 0;
    while let Ok(event) = changes.try_recv() {
        let value = serde_json::to_value(&event).unwrap();
        assert_eq!(value.as_object().unwrap().len(), 3);
        assert_eq!(event.profile_id, PROFILE);
        count += 1;
    }
    assert!(count >= 5);
}

#[test]
fn rejected_replacement_and_stale_intent_preserve_the_previous_active_connection() {
    let harness = Harness::new();
    harness.connect("first", 0);
    let original = harness.active();
    harness.verifier.reject.store(true, Ordering::SeqCst);
    let failed = harness.connect("replacement", 1);
    assert_eq!(failed.status, CredentialOperationStatus::Failed);
    assert_eq!(failed.error, Some(ProviderCredentialError::Unauthorized));
    assert_eq!(harness.active(), original);
    assert!(matches!(
        harness
            .service
            .start_connect(request("stale", 0), owner("stale")),
        Err(ProviderCredentialError::Conflict)
    ));
    assert_eq!(harness.store.writes.load(Ordering::SeqCst), 1);
}

#[test]
fn cancel_during_an_os_write_fences_late_completion_and_keeps_the_previous_connection() {
    let harness = Harness::new();
    harness.connect("first", 0);
    let original = harness.active();
    let (gate, entered, release) = Gate::pair();
    *harness.store.write_gate.lock().unwrap() = Some(gate);
    let service = harness.service.clone();
    let worker =
        std::thread::spawn(move || service.start_connect(request("second", 1), owner("second")));
    entered.recv_timeout(Duration::from_secs(10)).unwrap();
    let cancelled = harness.service.cancel(operation("second")).unwrap();
    assert_eq!(cancelled.status, CredentialOperationStatus::Cancelled);
    assert!(cancelled.cleanup_pending);
    assert_eq!(harness.active(), original);
    release.send(()).unwrap();
    let result = worker.join().unwrap().unwrap();
    assert_eq!(result.status, CredentialOperationStatus::Cancelled);
    assert!(!result.cleanup_pending);
    assert_eq!(harness.active(), original);
    assert!(
        harness
            .store
            .values
            .lock()
            .unwrap()
            .iter()
            .all(|(key, _)| key.connection_id == original.connection_id)
    );
}

#[test]
fn owner_ending_while_waiting_for_publication_rolls_back_the_active_change() {
    let harness = Harness::new();
    harness.connect("first", 0);
    let original = harness.active();
    let (gate, entered, release) = Gate::pair();
    *harness.ledger.before_publish.lock().unwrap() = Some(gate);
    let operation_owner = owner("replacement");
    let worker_owner = operation_owner.clone();
    let service = harness.service.clone();
    let worker =
        std::thread::spawn(move || service.start_connect(request("replacement", 1), worker_owner));
    entered.recv_timeout(Duration::from_secs(10)).unwrap();
    operation_owner.cancel();
    release.send(()).unwrap();
    let receipt = worker.join().unwrap().unwrap();
    assert_eq!(receipt.status, CredentialOperationStatus::Cancelled);
    assert_eq!(harness.active(), original);
}

#[test]
fn disconnect_cannot_be_undone_by_a_late_write_or_old_operation_replay() {
    let harness = Harness::new();
    let (gate, entered, release) = Gate::pair();
    *harness.store.write_gate.lock().unwrap() = Some(gate);
    let service = harness.service.clone();
    let worker =
        std::thread::spawn(move || service.start_connect(request("late", 0), owner("late")));
    entered.recv_timeout(Duration::from_secs(10)).unwrap();
    let removed = harness.service.disconnect(disconnect("remove", 0)).unwrap();
    assert!(removed.cleanup_pending);
    assert!(
        harness
            .service
            .state(Provider::Linear)
            .unwrap()
            .active_connection
            .is_none()
    );
    let newer = harness.connect("new", 1);
    assert_eq!(newer.status, CredentialOperationStatus::Committed);
    let current = harness.active();
    release.send(()).unwrap();
    let late = worker.join().unwrap().unwrap();
    assert_eq!(late.status, CredentialOperationStatus::Superseded);
    assert_eq!(harness.active(), current);
    let replay = harness.service.disconnect(disconnect("remove", 0)).unwrap();
    assert_eq!(replay.status, CredentialOperationStatus::Committed);
    assert_eq!(harness.active(), current);
    assert!(
        harness
            .store
            .values
            .lock()
            .unwrap()
            .iter()
            .all(|(key, _)| key.connection_id == current.connection_id)
    );
}

#[test]
fn deleting_old_slots_can_finish_after_a_new_connection_without_deleting_it() {
    let harness = Harness::new();
    harness.connect("old", 0);
    let old = harness.active();
    let (gate, entered, release) = Gate::pair();
    *harness.store.delete_gate.lock().unwrap() = Some(gate);
    let service = harness.service.clone();
    let worker = std::thread::spawn(move || service.disconnect(disconnect("remove", 1)));
    entered.recv_timeout(Duration::from_secs(10)).unwrap();
    assert!(
        harness
            .service
            .state(Provider::Linear)
            .unwrap()
            .active_connection
            .is_none()
    );
    let connected = harness.connect("new", 2);
    assert_eq!(connected.status, CredentialOperationStatus::Committed);
    let current = harness.active();
    assert_ne!(old.connection_id, current.connection_id);
    release.send(()).unwrap();
    worker.join().unwrap().unwrap();
    assert!(harness.service.acquire(&current).is_ok());
    assert!(
        harness
            .store
            .values
            .lock()
            .unwrap()
            .iter()
            .all(|(key, _)| key.connection_id == current.connection_id)
    );
}

#[test]
fn partial_multislot_write_failure_never_publishes_and_cleans_observed_parts() {
    let harness = Harness::new();
    *harness.prompt.token.lock().unwrap() = "界".repeat(4096).into_bytes();
    *harness.store.fail_write.lock().unwrap() = Some((2, true));
    let result = harness.connect("partial", 0);
    assert_eq!(result.status, CredentialOperationStatus::OutcomeUnknown);
    assert!(
        harness
            .service
            .state(Provider::Linear)
            .unwrap()
            .active_connection
            .is_none()
    );
    assert!(harness.store.values.lock().unwrap().is_empty());
    assert!(!result.cleanup_pending);
    assert_eq!(harness.store.writes.load(Ordering::SeqCst), 3);
}

#[test]
fn an_unobserved_submitted_write_remains_unknown_across_restart_without_os_reads() {
    let harness = Harness::new();
    *harness.store.fail_write.lock().unwrap() = Some((0, false));
    let receipt = harness.connect("unknown", 0);
    assert_eq!(receipt.status, CredentialOperationStatus::OutcomeUnknown);
    assert!(receipt.cleanup_pending);
    let reads = harness.store.reads.load(Ordering::SeqCst);
    let restarted = ProviderCredentialService::new(
        PROFILE.into(),
        harness.ledger.clone(),
        harness.store.clone(),
        harness.prompt.clone(),
        harness.verifier.clone(),
    )
    .unwrap();
    restarted.recover().unwrap();
    assert_eq!(harness.store.reads.load(Ordering::SeqCst), reads);
    assert!(
        restarted
            .state(Provider::Linear)
            .unwrap()
            .active_connection
            .is_none()
    );
    let replay = restarted
        .start_connect(request("unknown", 0), owner("replay"))
        .unwrap();
    assert_eq!(replay.status, CredentialOperationStatus::OutcomeUnknown);
    assert!(replay.cleanup_pending);
    assert_eq!(harness.store.writes.load(Ordering::SeqCst), 1);
}

#[test]
fn failed_delete_preserves_the_tombstone_and_reports_cleanup_pending() {
    let harness = Harness::new();
    harness.connect("first", 0);
    let old = harness.active();
    harness.store.fail_delete.store(true, Ordering::SeqCst);
    let removed = harness.service.disconnect(disconnect("remove", 1)).unwrap();
    assert_eq!(removed.status, CredentialOperationStatus::Committed);
    assert!(removed.cleanup_pending);
    assert!(
        harness
            .service
            .state(Provider::Linear)
            .unwrap()
            .active_connection
            .is_none()
    );
    assert_eq!(
        harness.service.validate_current(&old),
        Err(ProviderCredentialError::Conflict)
    );
    assert!(!harness.store.values.lock().unwrap().is_empty());
}

#[test]
fn key_store_lock_does_not_erase_metadata_or_replace_a_token_with_an_empty_value() {
    let harness = Harness::new();
    harness.connect("first", 0);
    let original = harness.active();
    harness.store.fail_read.store(true, Ordering::SeqCst);
    assert!(matches!(
        harness.service.acquire(&original),
        Err(ProviderCredentialError::Locked)
    ));
    assert_eq!(harness.active(), original);
    assert_eq!(harness.store.writes.load(Ordering::SeqCst), 1);
}

#[test]
fn legacy_import_requires_native_consent_snapshot_binding_and_a_terminal_ack() {
    let harness = Harness::new();
    let offer = offer("migration");
    let operation_owner = owner("migration");
    let prepared = harness
        .service
        .prepare_import(offer.clone(), operation_owner.clone())
        .unwrap();
    assert_eq!(prepared.status, CredentialOperationStatus::Prepared);
    assert!(!prepared.legacy_cleanup_allowed);
    assert_eq!(harness.prompt.confirms.load(Ordering::SeqCst), 1);
    let mut altered = offer.clone();
    altered.snapshot_id = "c".repeat(64);
    assert!(matches!(
        harness
            .service
            .import_secret(altered, token(), operation_owner.clone()),
        Err(ProviderCredentialError::Conflict)
    ));
    let committed = harness
        .service
        .import_secret(offer.clone(), token(), operation_owner.clone())
        .unwrap();
    assert!(committed.legacy_cleanup_allowed);
    assert!(committed.cleanup_pending);
    let replay = harness
        .service
        .import_secret(offer.clone(), token(), operation_owner)
        .unwrap();
    assert_eq!(committed, replay);
    assert_eq!(harness.store.writes.load(Ordering::SeqCst), 1);
    let ack = LegacyCleanupRequest {
        operation_id: offer.operation_id,
        profile_id: offer.profile_id,
        provider: offer.provider,
        snapshot_id: offer.snapshot_id,
    };
    let cleaned = harness
        .service
        .acknowledge_legacy_cleanup(ack.clone())
        .unwrap();
    assert!(!cleaned.cleanup_pending);
    assert!(!cleaned.legacy_cleanup_allowed);
    assert_eq!(
        harness.service.acknowledge_legacy_cleanup(ack).unwrap(),
        cleaned
    );
    assert_eq!(
        harness.service.operation(operation("migration")).unwrap(),
        cleaned
    );
}

#[test]
fn cancelled_migration_and_disconnect_never_allow_legacy_cleanup_or_automatic_reimport() {
    let harness = Harness::new();
    let offer = offer("migration");
    let operation_owner = owner("migration");
    harness
        .service
        .prepare_import(offer.clone(), operation_owner.clone())
        .unwrap();
    harness.service.cancel_owner(&operation_owner).unwrap();
    let cancelled = harness.service.operation(operation("migration")).unwrap();
    assert_eq!(cancelled.status, CredentialOperationStatus::Cancelled);
    assert!(!cancelled.legacy_cleanup_allowed);
    assert!(
        harness
            .service
            .import_secret(offer, token(), operation_owner)
            .is_err()
    );
    harness.service.disconnect(disconnect("remove", 0)).unwrap();
    assert!(matches!(
        harness
            .service
            .prepare_import(self::offer("other"), owner("other")),
        Err(ProviderCredentialError::Conflict)
    ));
    assert_eq!(harness.store.writes.load(Ordering::SeqCst), 0);
}

#[test]
fn restart_cancels_a_prepared_import_and_keeps_active_credentials_and_receipts() {
    let harness = Harness::new();
    harness
        .service
        .prepare_import(offer("prepared"), owner("prepared"))
        .unwrap();
    let restarted = ProviderCredentialService::new(
        PROFILE.into(),
        harness.ledger.clone(),
        harness.store.clone(),
        harness.prompt.clone(),
        harness.verifier.clone(),
    )
    .unwrap();
    restarted.recover().unwrap();
    assert_eq!(
        restarted.operation(operation("prepared")).unwrap().status,
        CredentialOperationStatus::Cancelled
    );
    assert_eq!(harness.store.reads.load(Ordering::SeqCst), 0);
    let committed = restarted
        .start_connect(request("fresh", 0), owner("fresh"))
        .unwrap();
    assert_eq!(committed.status, CredentialOperationStatus::Committed);
    let current = restarted
        .state(Provider::Linear)
        .unwrap()
        .active_connection
        .unwrap()
        .reference;
    let again = ProviderCredentialService::new(
        PROFILE.into(),
        harness.ledger.clone(),
        harness.store.clone(),
        harness.prompt.clone(),
        harness.verifier.clone(),
    )
    .unwrap();
    again.recover().unwrap();
    assert!(again.acquire(&current).is_ok());
}

#[test]
fn shutdown_cancels_pending_owners_without_waiting_for_the_key_store_and_blocks_new_input() {
    let harness = Harness::new();
    let (gate, entered, release) = Gate::pair();
    *harness.store.write_gate.lock().unwrap() = Some(gate);
    let service = harness.service.clone();
    let worker =
        std::thread::spawn(move || service.start_connect(request("pending", 0), owner("pending")));
    entered.recv_timeout(Duration::from_secs(10)).unwrap();
    let before = harness.store.reads.load(Ordering::SeqCst);
    harness.service.shutdown().unwrap();
    assert_eq!(harness.store.reads.load(Ordering::SeqCst), before);
    assert_eq!(
        harness
            .service
            .operation(operation("pending"))
            .unwrap()
            .status,
        CredentialOperationStatus::Cancelled
    );
    assert!(matches!(
        harness
            .service
            .start_connect(request("new", 0), owner("new")),
        Err(ProviderCredentialError::Cancelled)
    ));
    release.send(()).unwrap();
    worker.join().unwrap().unwrap();
    assert!(
        harness
            .service
            .state(Provider::Linear)
            .unwrap()
            .active_connection
            .is_none()
    );
}

#[test]
fn real_sqlite_metadata_and_public_receipts_contain_neither_token_nor_fingerprint() {
    use crate::adapter::outbound::db::sqlite_provider_credentials::SqliteProviderCredentialLedgerAdapter;
    let root = crate::test_utils::platform_safe_temp_dir().join(format!(
        "akra-provider-secret-free-{}-{:016x}",
        std::process::id(),
        rand::random::<u64>()
    ));
    crate::private_directory::ensure_private_directory(&root).unwrap();
    let runtime = root.join("runtime");
    let ledger = Arc::new(SqliteProviderCredentialLedgerAdapter::with_runtime(
        PROFILE.into(),
        runtime.clone(),
    ));
    let store = Arc::new(FakeStore::default());
    let service = ProviderCredentialService::new(
        PROFILE.into(),
        ledger,
        store,
        Arc::new(FakePrompt::new()),
        Arc::new(FakeVerifier::default()),
    )
    .unwrap();
    let receipt = service
        .start_connect(request("connect", 0), owner("connect"))
        .unwrap();
    let public = serde_json::to_string(&receipt).unwrap();
    let database = std::fs::read(runtime.join("provider-credentials.db")).unwrap();
    assert!(
        !database
            .windows(b"synthetic-provider-token".len())
            .any(|window| window == b"synthetic-provider-token")
    );
    assert!(!public.contains("synthetic-provider-token"));
    assert!(!public.to_lowercase().contains("fingerprint"));
    assert!(
        !database
            .windows(b"fingerprint".len())
            .any(|window| window == b"fingerprint")
    );
    drop(service);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn explicit_disconnect_replay_retries_old_cleanup_without_changing_revision_or_new_slots() {
    let harness = Harness::new();
    harness.connect("old", 0);
    harness.store.fail_delete.store(true, Ordering::SeqCst);
    let first = harness.service.disconnect(disconnect("remove", 1)).unwrap();
    assert!(first.cleanup_pending);
    harness.connect("new", 2);
    let current = harness.active();
    assert!(
        harness
            .service
            .operation(operation("remove"))
            .unwrap()
            .cleanup_pending
    );
    harness.store.fail_delete.store(false, Ordering::SeqCst);
    let replay = harness.service.disconnect(disconnect("remove", 1)).unwrap();
    assert!(!replay.cleanup_pending);
    assert_eq!(harness.active(), current);
    assert_eq!(harness.service.state(Provider::Linear).unwrap().revision, 3);
    assert!(
        harness
            .store
            .values
            .lock()
            .unwrap()
            .iter()
            .all(|(key, _)| key.connection_id == current.connection_id)
    );
}

#[test]
fn shutdown_before_begin_cas_prevents_a_late_prompt_and_pending_intent() {
    let harness = Harness::new();
    let (gate, entered, release) = Gate::pair();
    *harness.ledger.before_begin.lock().unwrap() = Some(gate);
    let service = harness.service.clone();
    let worker = std::thread::spawn(move || {
        service.start_connect(request("late-begin", 0), owner("late-begin"))
    });
    entered.recv_timeout(Duration::from_secs(10)).unwrap();
    harness.service.shutdown().unwrap();
    release.send(()).unwrap();
    assert!(matches!(
        worker.join().unwrap(),
        Err(ProviderCredentialError::Cancelled)
    ));
    assert_eq!(harness.prompt.calls.load(Ordering::SeqCst), 0);
    assert!(
        harness
            .service
            .state(Provider::Linear)
            .unwrap()
            .operation
            .is_none()
    );
}

struct StoreBackend(Arc<FakeStore>);
impl ProviderKeyStoreBackend for StoreBackend {
    fn read_part(&mut self, part: &KeyringPart) -> CredentialResult<Option<Zeroizing<Vec<u8>>>> {
        self.0.read_part(part)
    }
    fn write_part(&mut self, part: &KeyringPart, capsule: &[u8]) -> CredentialResult<()> {
        self.0.write_part(part, capsule)
    }
    fn delete_part(&mut self, part: &KeyringPart) -> CredentialResult<()> {
        self.0.delete_part(part)
    }
}

#[test]
fn real_worker_shutdown_returns_before_a_stalled_write_and_never_publishes_its_late_result() {
    use crate::adapter::outbound::provider_key_store::OsProviderKeyStoreAdapter;
    let ledger = Arc::new(MemoryLedger::default());
    let fake = Arc::new(FakeStore::default());
    let (gate, entered, release) = Gate::pair();
    *fake.write_gate.lock().unwrap() = Some(gate);
    let store =
        Arc::new(OsProviderKeyStoreAdapter::with_backend(Box::new(StoreBackend(fake))).unwrap());
    let service = Arc::new(
        ProviderCredentialService::new(
            PROFILE.into(),
            ledger.clone(),
            store.clone(),
            Arc::new(FakePrompt::new()),
            Arc::new(FakeVerifier::default()),
        )
        .unwrap(),
    );
    let (done, answer) = mpsc::sync_channel(1);
    let worker = {
        let service = service.clone();
        std::thread::spawn(move || {
            done.send(service.start_connect(request("stalled", 0), owner("stalled")))
                .unwrap();
        })
    };
    entered.recv_timeout(Duration::from_secs(10)).unwrap();
    service.shutdown().unwrap();
    let receipt = answer
        .recv_timeout(Duration::from_secs(2))
        .unwrap()
        .unwrap();
    assert_eq!(receipt.status, CredentialOperationStatus::Cancelled);
    assert!(receipt.cleanup_pending);
    assert!(
        service
            .state(Provider::Linear)
            .unwrap()
            .active_connection
            .is_none()
    );
    worker.join().unwrap();
    let candidate = ledger.load(Provider::Linear).unwrap().operations[0]
        .candidate
        .clone()
        .unwrap();
    release.send(()).unwrap();
    // Observe the late fake OS completion through the same worker; no real key store is used.
    assert!(store.read_part(&part_key(&candidate, 0)).unwrap().is_some());
    assert!(
        service
            .state(Provider::Linear)
            .unwrap()
            .active_connection
            .is_none()
    );
    assert_eq!(
        service.operation(operation("stalled")).unwrap().status,
        CredentialOperationStatus::Cancelled
    );
}

#[test]
fn cleanup_stops_at_the_first_store_error_instead_of_retrying_every_retained_candidate() {
    let harness = Harness::new();
    *harness.store.fail_write.lock().unwrap() = Some((0, false));
    assert!(harness.connect("unknown-1", 0).cleanup_pending);
    assert!(harness.connect("unknown-2", 0).cleanup_pending);
    harness.store.fail_read.store(true, Ordering::SeqCst);
    let reads = harness.store.reads.load(Ordering::SeqCst);
    let receipt = harness.service.disconnect(disconnect("remove", 0)).unwrap();
    assert!(receipt.cleanup_pending);
    assert_eq!(harness.store.reads.load(Ordering::SeqCst) - reads, 1);
}

#[test]
fn prepared_migration_replay_requires_the_original_live_owner_but_terminal_receipts_do_not() {
    let harness = Harness::new();
    let migration = offer("owner-bound");
    let first_owner = owner("first-session");
    let prepared = harness
        .service
        .prepare_import(migration.clone(), first_owner.clone())
        .unwrap();
    assert_eq!(prepared.status, CredentialOperationStatus::Prepared);
    assert_eq!(
        harness
            .service
            .prepare_import(migration.clone(), first_owner.clone())
            .unwrap(),
        prepared
    );
    assert_eq!(harness.prompt.confirms.load(Ordering::SeqCst), 1);
    let other_owner = owner("other-session");
    assert!(matches!(
        harness
            .service
            .prepare_import(migration.clone(), other_owner.clone()),
        Err(ProviderCredentialError::Conflict)
    ));
    assert!(
        harness
            .service
            .import_secret(migration.clone(), token(), other_owner)
            .is_err()
    );
    assert_eq!(harness.store.writes.load(Ordering::SeqCst), 0);

    let committed = harness
        .service
        .import_secret(migration.clone(), token(), first_owner)
        .unwrap();
    assert_eq!(committed.status, CredentialOperationStatus::Committed);
    assert_eq!(
        harness
            .service
            .prepare_import(migration, owner("receipt-session"))
            .unwrap(),
        committed
    );
    assert_eq!(harness.prompt.confirms.load(Ordering::SeqCst), 1);
    assert_eq!(harness.store.writes.load(Ordering::SeqCst), 1);
}

#[test]
fn prepared_migration_cannot_be_reused_in_the_gap_before_eof_durable_cancellation() {
    let harness = Harness::new();
    let migration = offer("owner-ending");
    let first_owner = owner("ending-session");
    harness
        .service
        .prepare_import(migration.clone(), first_owner.clone())
        .unwrap();
    first_owner.cancel(); // The transport has observed EOF; its durable cancellation worker has not run yet.
    assert_eq!(
        harness
            .service
            .operation(operation("owner-ending"))
            .unwrap()
            .status,
        CredentialOperationStatus::Prepared
    );
    assert!(matches!(
        harness
            .service
            .prepare_import(migration.clone(), owner("new-session")),
        Err(ProviderCredentialError::Conflict)
    ));
    assert!(matches!(
        harness
            .service
            .prepare_import(migration, first_owner.clone()),
        Err(ProviderCredentialError::Cancelled)
    ));
    assert_eq!(harness.prompt.confirms.load(Ordering::SeqCst), 1);
    assert_eq!(harness.store.writes.load(Ordering::SeqCst), 0);
    harness.service.cancel_owner(&first_owner).unwrap();
    assert_eq!(
        harness
            .service
            .operation(operation("owner-ending"))
            .unwrap()
            .status,
        CredentialOperationStatus::Cancelled
    );
}
