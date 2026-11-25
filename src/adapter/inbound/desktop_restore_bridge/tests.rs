use super::*;
use crate::adapter::outbound::vault_key_store::FakeVaultKeyStoreAdapter;
use crate::application::port::inbound::vault_restore_session_port::{
    MAX_VIEWING_SESSION as SESSION_LIFETIME, VaultRestoreSessionFailure, VaultRestoreSessionHold,
    VaultRestoreSessionPrepared,
};
mod reconnect;
use crate::application::port::outbound::evidence_vault_port::{
    EvidenceVaultStorePort, VaultKey, VaultPlaintext, VaultRecordLoadRequest,
    VaultRecordSaveRequest, VaultRecordScope, VaultStoreError,
};
use crate::application::port::outbound::issue_snapshot_port::{
    IssueRawFields, IssueSnapshotPort, IssueSnapshotReadRequest, IssueSnapshotResolveRequest,
    IssueSnapshotResponse,
};
use crate::application::port::outbound::vault_key_store_port::{
    VaultKeyStoreError, VaultKeyStorePort,
};
use crate::application::port::outbound::vault_native_confirmation_port::{
    VaultNativeConfirmationOutcome, VaultNativeConfirmationPort, VaultNativeConfirmationPurpose,
    VaultNativeConfirmationRequest,
};
use crate::application::port::outbound::vault_restore_workspace_port::{
    VaultRestoreWorkspaceError, VaultRestoreWorkspacePort,
};
use crate::application::service::desktop_vault_restore::{
    DesktopVaultRestoreDependencies, DesktopVaultRestoreService,
};
use crate::application::service::evidence_vault_restore::{
    EvidenceVaultRestoreService, restorable_fields,
};
use crate::application::service::evidence_vault_unlock::EvidenceVaultUnlockService;
use crate::application::service::issue_masking::{
    IssueSensitiveSpan, IssueTextField, mask_issue_text,
};
use crate::domain::issue_policy::{IssueDisclosurePolicy, IssueSensitiveSpanAction};
use crate::domain::issue_publish_intent::{IssuePublishVaultField, confirmed_version_name};
use crate::domain::issue_work::{
    IssueComparisonBasis, IssueFieldHash, IssueProjectScope, IssueProvider, IssueReference,
};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::{
    Mutex,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};
use std::time::Instant;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader, DuplexStream};
use tokio::sync::Notify;

const TITLE: &str = "ticket synthetic-title-secret";
const BODY: &str = "details synthetic-body-secret";

#[derive(Default)]
struct Gate {
    started: Notify,
    released: AtomicBool,
}
impl Gate {
    fn wait(&self) {
        self.started.notify_one();
        let deadline = Instant::now() + Duration::from_secs(5);
        while !self.released.load(Ordering::SeqCst) && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(2));
        }
    }
    fn release(&self) {
        self.released.store(true, Ordering::SeqCst);
    }
}
#[derive(Default)]
struct Keys {
    loads: AtomicUsize,
    stores: AtomicUsize,
    gate: Mutex<Option<Arc<Gate>>>,
}
impl VaultKeyStorePort for Keys {
    fn load_key(&self) -> Result<Option<VaultKey>, VaultKeyStoreError> {
        self.loads.fetch_add(1, Ordering::SeqCst);
        let gate = self.gate.lock().unwrap().clone();
        if let Some(gate) = gate {
            gate.wait();
        }
        Ok(Some(VaultKey::new(vec![7; 32])))
    }
    fn get_or_create_key(&self) -> Result<VaultKey, VaultKeyStoreError> {
        self.stores.fetch_add(1, Ordering::SeqCst);
        panic!("read-only restore must never create a key");
    }
}
#[derive(Default)]
struct Records {
    values: Mutex<HashMap<VaultRecordScope, VaultPlaintext>>,
    writes: AtomicUsize,
    corrupt: AtomicBool,
    write_gate: Mutex<Option<Arc<Gate>>>,
    lose_commit_reply: AtomicBool,
    commits: AtomicUsize,
}
impl EvidenceVaultStorePort for Records {
    fn begin_binding(
        &self,
        request: VaultRecordLoadRequest,
    ) -> Result<
        Box<
            dyn crate::application::port::outbound::evidence_vault_port::VaultBindingTransaction
                + '_,
        >,
        VaultStoreError,
    > {
        if self.corrupt.load(Ordering::SeqCst) {
            return Err(VaultStoreError::IntegrityFailure);
        }
        crate::test_utils::vault_binding::begin_test_binding(
            self.values.lock().unwrap(),
            request,
            crate::test_utils::vault_binding::TestBindingHooks {
                before_write: Some(Box::new(|_| {
                    let gate = self.write_gate.lock().unwrap().clone();
                    if let Some(gate) = gate {
                        gate.wait();
                    }
                    self.writes.fetch_add(1, Ordering::SeqCst);
                    Ok(())
                })),
                after_pointer: Some(Box::new(|| {
                    self.commits.fetch_add(1, Ordering::SeqCst);
                    if self.lose_commit_reply.swap(false, Ordering::SeqCst) {
                        Err(VaultStoreError::WriteOutcomeUnknown)
                    } else {
                        Ok(())
                    }
                })),
            },
        )
    }

    fn save(&self, request: VaultRecordSaveRequest) -> Result<(), VaultStoreError> {
        self.writes.fetch_add(1, Ordering::SeqCst);
        self.values
            .lock()
            .unwrap()
            .insert(request.scope, request.plaintext);
        Ok(())
    }
    fn load(&self, request: VaultRecordLoadRequest) -> Result<VaultPlaintext, VaultStoreError> {
        if self.corrupt.load(Ordering::SeqCst) {
            return Err(VaultStoreError::IntegrityFailure);
        }
        self.values
            .lock()
            .unwrap()
            .get(&request.scope)
            .cloned()
            .ok_or(VaultStoreError::RecordNotFound)
    }
}
struct Remote {
    fields: Mutex<(String, String)>,
    reference: Mutex<IssueReference>,
    reads: AtomicUsize,
    resolves: AtomicUsize,
    gate: Mutex<Option<Arc<Gate>>>,
}
impl Remote {
    fn snapshot(&self) -> IssueSnapshotResponse {
        let (title, body) = self.fields.lock().unwrap().clone();
        let raw_fields = IssueRawFields::new(title, body);
        IssueSnapshotResponse {
            issue: self.reference.lock().unwrap().clone(),
            comparison_basis: IssueComparisonBasis {
                field_hash: raw_fields.field_hash(),
                provider_updated_at: "2026-11-22T00:00:00Z".into(),
            },
            raw_fields,
        }
    }
}
impl IssueSnapshotPort for Remote {
    fn resolve(&self, _: IssueSnapshotResolveRequest) -> anyhow::Result<IssueSnapshotResponse> {
        self.resolves.fetch_add(1, Ordering::SeqCst);
        Ok(self.snapshot())
    }
    fn read(&self, _: IssueSnapshotReadRequest) -> anyhow::Result<IssueSnapshotResponse> {
        self.reads.fetch_add(1, Ordering::SeqCst);
        let gate = self.gate.lock().unwrap().clone();
        if let Some(gate) = gate {
            gate.wait();
        }
        Ok(self.snapshot())
    }
}
struct Confirmation {
    calls: AtomicUsize,
    outcome: VaultNativeConfirmationOutcome,
    gate: Option<Arc<Gate>>,
    purposes: Mutex<Vec<VaultNativeConfirmationPurpose>>,
    write_outcome: Mutex<Option<VaultNativeConfirmationOutcome>>,
    write_gate: Mutex<Option<Arc<Gate>>>,
}
impl VaultNativeConfirmationPort for Confirmation {
    fn confirm(
        &self,
        request: &VaultNativeConfirmationRequest,
        cancelled: &AtomicBool,
        deadline: Instant,
    ) -> VaultNativeConfirmationOutcome {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.purposes.lock().unwrap().push(request.purpose);
        let write = request.purpose == VaultNativeConfirmationPurpose::RewriteBinding;
        let gate = if write {
            self.write_gate.lock().unwrap().clone()
        } else {
            self.gate.clone()
        };
        if let Some(gate) = &gate {
            gate.started.notify_one();
            while !gate.released.load(Ordering::SeqCst)
                && !cancelled.load(Ordering::SeqCst)
                && Instant::now() < deadline
            {
                std::thread::sleep(Duration::from_millis(2));
            }
        }
        if cancelled.load(Ordering::SeqCst) || Instant::now() >= deadline {
            VaultNativeConfirmationOutcome::Cancelled
        } else if write {
            self.write_outcome.lock().unwrap().unwrap_or(self.outcome)
        } else {
            self.outcome
        }
    }
}
struct Workspace(Option<String>);
impl VaultRestoreWorkspacePort for Workspace {
    fn canonical_workspace(&self, workspace: &str) -> Result<String, VaultRestoreWorkspaceError> {
        Ok(self.0.clone().unwrap_or_else(|| workspace.to_owned()))
    }
}
struct Fixture {
    keys: Arc<Keys>,
    records: Arc<Records>,
    remote: Arc<Remote>,
    confirmation: Arc<Confirmation>,
    unlock: Arc<EvidenceVaultUnlockService>,
    initial_loads: usize,
}
impl Fixture {
    fn new(outcome: VaultNativeConfirmationOutcome, gate: Option<Arc<Gate>>) -> Self {
        let keys = Arc::new(Keys::default());
        let unlock = Arc::new(EvidenceVaultUnlockService::new(keys.clone()));
        let records = Arc::new(Records::default());
        let fields = seed(records.as_ref());
        let initial_loads = keys.loads.load(Ordering::SeqCst);
        Self {
            keys,
            records,
            unlock,
            initial_loads,
            remote: Arc::new(Remote {
                fields: Mutex::new(fields),
                reference: Mutex::new(issue()),
                reads: AtomicUsize::new(0),
                resolves: AtomicUsize::new(0),
                gate: Mutex::new(None),
            }),
            confirmation: Arc::new(Confirmation {
                calls: AtomicUsize::new(0),
                outcome,
                gate,
                purposes: Mutex::new(Vec::new()),
                write_outcome: Mutex::new(None),
                write_gate: Mutex::new(None),
            }),
        }
    }
    fn services(&self) -> DesktopVaultRestoreDependencies {
        self.services_with_store(self.records.clone())
    }
    fn services_with_store(
        &self,
        store: Arc<dyn EvidenceVaultStorePort>,
    ) -> DesktopVaultRestoreDependencies {
        DesktopVaultRestoreDependencies {
            supported: true,
            snapshots: self.remote.clone(),
            unlock: self.unlock.clone(),
            restore: Arc::new(EvidenceVaultRestoreService::new(
                self.remote.clone(),
                store,
                self.unlock.clone(),
            )),
            confirmation: self.confirmation.clone(),
            workspace: Arc::new(Workspace(None)),
        }
    }
    fn assert_no_key_access(&self) {
        assert_eq!(self.keys.loads.load(Ordering::SeqCst), self.initial_loads);
        assert_eq!(self.keys.stores.load(Ordering::SeqCst), 0);
    }
}
fn issue() -> IssueReference {
    IssueReference {
        provider: IssueProvider::GitHub,
        server_id: "github.com".into(),
        repository_id: "R_1".into(),
        issue_id: "I_1".into(),
        project_scope: IssueProjectScope {
            workspace_dir: "/work/project".into(),
        },
    }
}
fn seed(store: &dyn EvidenceVaultStorePort) -> (String, String) {
    let sensitive = |field, text: &str, secret: &str| {
        let start = text.find(secret).unwrap();
        IssueSensitiveSpan {
            field,
            range: start..start + secret.len(),
            class: None,
            user_decision: Some(IssueSensitiveSpanAction::RestorableMask),
        }
    };
    let outcome = mask_issue_text(
        TITLE,
        BODY,
        &[
            sensitive(IssueTextField::Title, TITLE, "synthetic-title-secret"),
            sensitive(IssueTextField::Body, BODY, "synthetic-body-secret"),
        ],
        &IssueDisclosurePolicy::builtin(),
    )
    .unwrap();
    let version = confirmed_version_name(&IssueFieldHash::of_fields(
        &outcome.shared_copy.title,
        &outcome.shared_copy.body,
    ));
    use crate::application::port::outbound::evidence_vault_port::{
        VaultBindingCommitPolicy, VaultBindingCommitRequest, VaultBindingCommitResult,
        VaultBindingFields,
    };
    // 암호문 준비는 별도 fake로 수행한다. 실제 native 경로의 Keys는 초기화를 금지한다.
    let seed_unlock = EvidenceVaultUnlockService::new(Arc::new(
        FakeVaultKeyStoreAdapter::with_key(VaultKey::new(vec![7; 32])),
    ));
    let key = seed_unlock.sealing_key().unwrap();
    let mut fields = VaultBindingFields::default();
    for field in restorable_fields(&outcome.masked_spans) {
        match field.field {
            IssuePublishVaultField::Title => fields.title = Some(field.plaintext),
            IssuePublishVaultField::Body => fields.body = Some(field.plaintext),
        }
    }
    let binding = store
        .begin_binding(VaultRecordLoadRequest {
            scope: VaultRecordScope::current_version(issue()),
            key,
        })
        .unwrap();
    let expected = binding.snapshot().token.clone();
    assert!(matches!(
        binding
            .commit(VaultBindingCommitRequest {
                expected,
                operation_id: "native-restore-synthetic-fixture".to_string(),
                base_version: version,
                approval_digest: [11; 32],
                fields,
                policy: VaultBindingCommitPolicy::InstallIfMissing,
            })
            .unwrap(),
        VaultBindingCommitResult::Committed(_),
    ));
    (outcome.shared_copy.title, outcome.shared_copy.body)
}
fn prepare_request() -> Value {
    json!({
        "version": 2, "id": "test-session", "command": "prepare",
        "issue_url": "https://github.com/acme/project/issues/1",
        "issue": { "provider": "github", "server_id": "github.com", "repository_id": "R_1", "issue_id": "I_1",
            "project_scope": { "workspace_dir": "/work/project" } }
    })
}
fn restore_request() -> Value {
    json!({"version":2,"id":"test-session","command":"restore"})
}
async fn run_test_io<R: AsyncRead + Unpin, W: AsyncWrite + Unpin>(
    input: R,
    output: W,
    dependencies: DesktopVaultRestoreDependencies,
    duration: Duration,
) -> io::Result<()> {
    let mut frames = Frames::new(input);
    let Some((id, request)) = first_request(&mut frames).await else {
        return Ok(());
    };
    let port = Arc::new(DesktopVaultRestoreService::new(dependencies, duration));
    session(frames, output, id, request, port).await
}
fn start(
    services: DesktopVaultRestoreDependencies,
    duration: Duration,
) -> (
    BufReader<DuplexStream>,
    tokio::task::JoinHandle<io::Result<()>>,
) {
    let (client, server) = tokio::io::duplex(128 * 1024);
    let (input, output) = tokio::io::split(server);
    (
        BufReader::new(client),
        tokio::spawn(run_test_io(input, output, services, duration)),
    )
}
async fn send(client: &mut BufReader<DuplexStream>, value: &Value) {
    let mut bytes = serde_json::to_vec(value).unwrap();
    bytes.push(b'\n');
    client.get_mut().write_all(&bytes).await.unwrap();
}
async fn response(client: &mut BufReader<DuplexStream>) -> Value {
    let mut line = String::new();
    tokio::time::timeout(Duration::from_secs(3), client.read_line(&mut line))
        .await
        .unwrap()
        .unwrap();
    serde_json::from_str(&line).unwrap()
}
async fn no_more_output(client: &mut BufReader<DuplexStream>) {
    let mut bytes = Vec::new();
    tokio::time::timeout(Duration::from_secs(3), client.read_to_end(&mut bytes))
        .await
        .unwrap()
        .unwrap();
    assert!(bytes.is_empty());
}

#[tokio::test]
async fn native_prepare_does_not_touch_keys_and_restore_reads_fresh_copy_once_without_writing() {
    let fixture = Fixture::new(VaultNativeConfirmationOutcome::Confirmed, None);
    let initial_writes = fixture.records.writes.load(Ordering::SeqCst);
    let (mut client, task) = start(fixture.services(), SESSION_LIFETIME);
    send(&mut client, &prepare_request()).await;
    assert_eq!(response(&mut client).await["result"]["kind"], "prepared");
    fixture.assert_no_key_access();
    send(&mut client, &restore_request()).await;
    let output = response(&mut client).await;
    assert_eq!(output["result"]["title"], TITLE);
    assert_eq!(output["result"]["body"], BODY);
    task.await.unwrap().unwrap();
    assert_eq!(fixture.remote.resolves.load(Ordering::SeqCst), 1);
    assert_eq!(fixture.remote.reads.load(Ordering::SeqCst), 1);
    assert_eq!(
        fixture.records.writes.load(Ordering::SeqCst),
        initial_writes
    );
    assert_eq!(fixture.keys.stores.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn cancellation_and_unsupported_platform_never_read_keys() {
    for (outcome, supported, reason) in [
        (
            VaultNativeConfirmationOutcome::Cancelled,
            true,
            "confirmation_cancelled",
        ),
        (
            VaultNativeConfirmationOutcome::Confirmed,
            false,
            "platform_unsupported",
        ),
        (
            VaultNativeConfirmationOutcome::Unsupported,
            true,
            "platform_unsupported",
        ),
        (
            VaultNativeConfirmationOutcome::Unavailable,
            true,
            "source_unavailable",
        ),
    ] {
        let fixture = Fixture::new(outcome, None);
        let mut services = fixture.services();
        services.supported = supported;
        let (mut client, task) = start(services, SESSION_LIFETIME);
        send(&mut client, &prepare_request()).await;
        assert_eq!(response(&mut client).await["result"]["reason"], reason);
        task.await.unwrap().unwrap();
        fixture.assert_no_key_access();
        if !supported {
            assert_eq!(fixture.remote.resolves.load(Ordering::SeqCst), 0);
            assert_eq!(fixture.confirmation.calls.load(Ordering::SeqCst), 0);
        }
    }
}

#[tokio::test]
async fn caller_cannot_supply_approval_or_skip_prepare() {
    for command in [restore_request(), {
        let mut command = prepare_request();
        command["approved"] = json!(true);
        command
    }] {
        let fixture = Fixture::new(VaultNativeConfirmationOutcome::Confirmed, None);
        let (mut client, task) = start(fixture.services(), SESSION_LIFETIME);
        send(&mut client, &command).await;
        no_more_output(&mut client).await;
        task.await.unwrap().unwrap();
        fixture.assert_no_key_access();
        assert_eq!(fixture.confirmation.calls.load(Ordering::SeqCst), 0);
    }
}

#[tokio::test]
async fn url_full_identity_and_canonical_workspace_must_match_before_prompt() {
    for case in 0..4 {
        let fixture = Fixture::new(VaultNativeConfirmationOutcome::Confirmed, None);
        let mut request = prepare_request();
        let mut services = fixture.services();
        match case {
            0 => request["issue_url"] = json!("https://github.com/acme/project/pull/1"),
            1 => request["issue"]["repository_id"] = json!("R_other"),
            2 => request["issue"]["issue_id"] = json!("I_other"),
            _ => services.workspace = Arc::new(Workspace(Some("/different/project".into()))),
        }
        let (mut client, task) = start(services, SESSION_LIFETIME);
        send(&mut client, &request).await;
        assert_eq!(response(&mut client).await["result"]["reason"], "not_bound");
        task.await.unwrap().unwrap();
        fixture.assert_no_key_access();
        assert_eq!(fixture.confirmation.calls.load(Ordering::SeqCst), 0);
    }
}

#[tokio::test]
async fn remote_edit_after_native_confirmation_holds_without_reconnecting() {
    let fixture = Fixture::new(VaultNativeConfirmationOutcome::Confirmed, None);
    let writes = fixture.records.writes.load(Ordering::SeqCst);
    let (mut client, task) = start(fixture.services(), SESSION_LIFETIME);
    send(&mut client, &prepare_request()).await;
    response(&mut client).await;
    fixture
        .remote
        .fields
        .lock()
        .unwrap()
        .1
        .push_str(" changed remotely");
    send(&mut client, &restore_request()).await;
    assert_eq!(
        response(&mut client).await["result"],
        json!({"kind":"held","reason":"published_copy_changed","can_reconnect":true})
    );
    client.get_mut().shutdown().await.unwrap();
    no_more_output(&mut client).await;
    task.await.unwrap().unwrap();
    assert_eq!(fixture.records.writes.load(Ordering::SeqCst), writes);
}

#[tokio::test]
async fn changing_session_id_or_target_after_prepared_cannot_unlock() {
    for field in ["id", "issue_url"] {
        let fixture = Fixture::new(VaultNativeConfirmationOutcome::Confirmed, None);
        let (mut client, task) = start(fixture.services(), SESSION_LIFETIME);
        send(&mut client, &prepare_request()).await;
        response(&mut client).await;
        let mut request = restore_request();
        request[field] = json!("different");
        send(&mut client, &request).await;
        no_more_output(&mut client).await;
        task.await.unwrap().unwrap();
        fixture.assert_no_key_access();
    }
}

#[tokio::test]
async fn eof_or_extra_byte_during_native_prompt_cancels_without_key_access() {
    for eof in [true, false] {
        let gate = Arc::new(Gate::default());
        let fixture = Fixture::new(
            VaultNativeConfirmationOutcome::Confirmed,
            Some(gate.clone()),
        );
        let (mut client, task) = start(fixture.services(), SESSION_LIFETIME);
        send(&mut client, &prepare_request()).await;
        gate.started.notified().await;
        if eof {
            client.get_mut().shutdown().await.unwrap();
        } else {
            client.get_mut().write_all(b"{").await.unwrap();
        }
        no_more_output(&mut client).await;
        task.await.unwrap().unwrap();
        fixture.assert_no_key_access();
    }
}

#[tokio::test]
async fn expired_native_prompt_never_accesses_keys_or_emits_prepared() {
    let gate = Arc::new(Gate::default());
    let fixture = Fixture::new(VaultNativeConfirmationOutcome::Confirmed, Some(gate));
    let (mut client, task) = start(fixture.services(), Duration::from_millis(30));
    send(&mut client, &prepare_request()).await;
    no_more_output(&mut client).await;
    task.await.unwrap().unwrap();
    fixture.assert_no_key_access();
}

#[tokio::test]
async fn eof_during_key_lookup_invalidates_grant_and_drops_late_plaintext() {
    let fixture = Fixture::new(VaultNativeConfirmationOutcome::Confirmed, None);
    let (mut client, task) = start(fixture.services(), SESSION_LIFETIME);
    send(&mut client, &prepare_request()).await;
    response(&mut client).await;
    let gate = Arc::new(Gate::default());
    *fixture.keys.gate.lock().unwrap() = Some(gate.clone());
    send(&mut client, &restore_request()).await;
    gate.started.notified().await;
    client.get_mut().shutdown().await.unwrap();
    no_more_output(&mut client).await;
    task.await.unwrap().unwrap();
    gate.release();
    tokio::time::sleep(Duration::from_millis(20)).await;
    assert_eq!(
        fixture.keys.loads.load(Ordering::SeqCst),
        fixture.initial_loads + 1
    );
    assert_eq!(fixture.remote.reads.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn eof_during_latest_read_prevents_the_final_key_lookup_and_output() {
    let fixture = Fixture::new(VaultNativeConfirmationOutcome::Confirmed, None);
    let (mut client, task) = start(fixture.services(), SESSION_LIFETIME);
    send(&mut client, &prepare_request()).await;
    response(&mut client).await;
    let gate = Arc::new(Gate::default());
    *fixture.remote.gate.lock().unwrap() = Some(gate.clone());
    send(&mut client, &restore_request()).await;
    gate.started.notified().await;
    let before_cancel = fixture.keys.loads.load(Ordering::SeqCst);
    client.get_mut().shutdown().await.unwrap();
    no_more_output(&mut client).await;
    task.await.unwrap().unwrap();
    gate.release();
    tokio::time::sleep(Duration::from_millis(20)).await;
    assert_eq!(fixture.keys.loads.load(Ordering::SeqCst), before_cancel);
}

#[tokio::test]
async fn corrupted_record_returns_only_a_fixed_hold() {
    let fixture = Fixture::new(VaultNativeConfirmationOutcome::Confirmed, None);
    fixture.records.corrupt.store(true, Ordering::SeqCst);
    let (mut client, task) = start(fixture.services(), SESSION_LIFETIME);
    send(&mut client, &prepare_request()).await;
    response(&mut client).await;
    send(&mut client, &restore_request()).await;
    let output = response(&mut client).await;
    assert_eq!(
        output["result"],
        json!({"kind":"held","reason":"record_unavailable","can_reconnect":false})
    );
    task.await.unwrap().unwrap();
}

#[test]
fn protected_frame_is_bounded_after_json_escaping() {
    let oversized = "\0".repeat(protocol::MAX_OUTPUT_BYTES / 3);
    let encoded = protocol::OutputBudget::new()
        .encode(
            "test",
            Reply::Restored {
                title: "",
                body: &oversized,
            },
            true,
        )
        .unwrap();
    let value: Value = serde_json::from_slice(&encoded.bytes).unwrap();
    assert_eq!(
        value["result"],
        json!({"kind":"held","reason":"source_unavailable","can_reconnect":false})
    );
    assert!(encoded.bytes.len() < 200);
    assert!(encoded.terminal);
}

#[tokio::test]
async fn oversized_or_malformed_request_has_no_side_effect() {
    for bytes in [
        vec![b'x'; protocol::MAX_INPUT_BYTES + 1],
        b"{invalid}\n".to_vec(),
    ] {
        let fixture = Fixture::new(VaultNativeConfirmationOutcome::Confirmed, None);
        let (mut client, task) = start(fixture.services(), SESSION_LIFETIME);
        client.get_mut().write_all(&bytes).await.unwrap();
        no_more_output(&mut client).await;
        task.await.unwrap().unwrap();
        fixture.assert_no_key_access();
        assert_eq!(fixture.confirmation.calls.load(Ordering::SeqCst), 0);
    }
}

#[cfg(unix)]
#[tokio::test]
async fn actual_encrypted_records_restore_through_the_private_pipe_without_file_mutation() {
    use crate::adapter::outbound::filesystem::evidence_vault_store::FilesystemEvidenceVaultStoreAdapter;
    let fixture = Fixture::new(VaultNativeConfirmationOutcome::Confirmed, None);
    struct Temporary(std::path::PathBuf);
    impl Drop for Temporary {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let temporary = Temporary(crate::test_utils::platform_safe_temp_dir().join(format!(
        "akra-restore-encrypted-{}-{}",
        std::process::id(),
        rand::random::<u64>()
    )));
    crate::private_directory::ensure_private_directory(&temporary.0).unwrap();
    let store = Arc::new(FilesystemEvidenceVaultStoreAdapter::new(
        temporary.0.clone(),
    ));
    seed(store.as_ref());
    fn files(root: &std::path::Path) -> Vec<(std::path::PathBuf, Vec<u8>)> {
        let mut values = Vec::new();
        for entry in std::fs::read_dir(root).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                values.extend(files(&path));
            } else {
                values.push((path.clone(), std::fs::read(path).unwrap()));
            }
        }
        values.sort_by(|left, right| left.0.cmp(&right.0));
        values
    }
    let before = files(&temporary.0);
    for (_, bytes) in &before {
        let text = String::from_utf8_lossy(bytes);
        assert!(!text.contains("synthetic-title-secret"));
        assert!(!text.contains("synthetic-body-secret"));
    }
    let (mut client, task) = start(fixture.services_with_store(store), SESSION_LIFETIME);
    send(&mut client, &prepare_request()).await;
    response(&mut client).await;
    send(&mut client, &restore_request()).await;
    assert_eq!(response(&mut client).await["result"]["body"], BODY);
    task.await.unwrap().unwrap();
    assert_eq!(files(&temporary.0), before);
}

#[tokio::test]
async fn prepared_session_expiry_never_unlocks_without_a_timely_restore() {
    let fixture = Fixture::new(VaultNativeConfirmationOutcome::Confirmed, None);
    let (mut client, task) = start(fixture.services(), Duration::from_millis(80));
    send(&mut client, &prepare_request()).await;
    assert_eq!(response(&mut client).await["result"]["kind"], "prepared");
    no_more_output(&mut client).await;
    task.await.unwrap().unwrap();
    fixture.assert_no_key_access();
}

fn session_request() -> VaultRestoreSessionRequest {
    VaultRestoreSessionRequest {
        issue_url: "https://github.com/acme/project/issues/1".into(),
        issue: issue(),
    }
}
fn session_port(fixture: &Fixture) -> Arc<dyn VaultRestoreSessionPort> {
    Arc::new(DesktopVaultRestoreService::new(
        fixture.services(),
        SESSION_LIFETIME,
    ))
}

#[test]
fn direct_port_cannot_restore_before_native_prepare() {
    let fixture = Fixture::new(VaultNativeConfirmationOutcome::Confirmed, None);
    let port = session_port(&fixture);
    assert!(matches!(
        port.restore(),
        Err(VaultRestoreSessionFailure {
            reason: VaultRestoreSessionHold::Locked,
            can_reconnect: false
        })
    ));
    fixture.assert_no_key_access();
    assert_eq!(fixture.remote.resolves.load(Ordering::SeqCst), 0);
    assert_eq!(fixture.confirmation.calls.load(Ordering::SeqCst), 0);
}

#[test]
fn direct_port_prepares_once_consumes_restore_once_and_remains_live_for_output() {
    let fixture = Fixture::new(VaultNativeConfirmationOutcome::Confirmed, None);
    let port = session_port(&fixture);
    assert_eq!(
        port.prepare(session_request()),
        Ok(VaultRestoreSessionPrepared { reconnect: true })
    );
    let mut other_target = session_request();
    other_target.issue_url = "https://github.com/other/project/issues/2".into();
    other_target.issue.repository_id = "R_other".into();
    other_target.issue.issue_id = "I_other".into();
    assert_eq!(
        port.prepare(other_target),
        Err(VaultRestoreSessionHold::Locked)
    );
    assert_eq!(fixture.remote.resolves.load(Ordering::SeqCst), 1);
    fixture.assert_no_key_access();
    assert_eq!(fixture.confirmation.calls.load(Ordering::SeqCst), 1);
    let result = port.restore().unwrap();
    assert_eq!(result.title.expose(), TITLE);
    assert_eq!(result.body.expose(), BODY);
    assert!(port.is_live());
    let loads = fixture.keys.loads.load(Ordering::SeqCst);
    assert!(matches!(
        port.restore(),
        Err(VaultRestoreSessionFailure {
            reason: VaultRestoreSessionHold::Locked,
            can_reconnect: false
        })
    ));
    assert_eq!(
        port.prepare(session_request()),
        Err(VaultRestoreSessionHold::Locked)
    );
    assert_eq!(fixture.keys.loads.load(Ordering::SeqCst), loads);
    assert_eq!(fixture.keys.stores.load(Ordering::SeqCst), 0);
}

#[test]
fn direct_port_cancel_after_prepare_rejects_every_later_entry() {
    let fixture = Fixture::new(VaultNativeConfirmationOutcome::Confirmed, None);
    let port = session_port(&fixture);
    assert_eq!(
        port.prepare(session_request()),
        Ok(VaultRestoreSessionPrepared { reconnect: true })
    );
    port.cancel();
    assert!(!port.is_live());
    assert_eq!(
        port.prepare(session_request()),
        Err(VaultRestoreSessionHold::Locked)
    );
    assert!(matches!(
        port.restore(),
        Err(VaultRestoreSessionFailure {
            reason: VaultRestoreSessionHold::Locked,
            can_reconnect: false
        })
    ));
    fixture.assert_no_key_access();
    assert_eq!(fixture.confirmation.calls.load(Ordering::SeqCst), 1);
}

#[test]
fn direct_port_validates_target_without_trusting_the_protocol_adapter() {
    for case in 0..8 {
        let fixture = Fixture::new(VaultNativeConfirmationOutcome::Confirmed, None);
        let mut request = session_request();
        match case {
            0 => request.issue_url = "https://github.com/acme/project/issues/01".into(),
            1 => request.issue.server_id = "untrusted.invalid".into(),
            2 => request.issue.repository_id = "R_other".into(),
            3 => request.issue.issue_id = "I_other".into(),
            4 => request.issue.project_scope.workspace_dir = "relative/project".into(),
            5 => request.issue.repository_id = "a".repeat(201),
            6 => request.issue.issue_id = "I_bad\n".into(),
            _ => {
                request.issue_url = "https://github.com/acme/project/issues/1?approved=true".into()
            }
        }
        let port = session_port(&fixture);
        assert_eq!(
            port.prepare(request),
            Err(VaultRestoreSessionHold::NotBound)
        );
        assert!(matches!(
            port.restore(),
            Err(VaultRestoreSessionFailure {
                reason: VaultRestoreSessionHold::Locked,
                can_reconnect: false
            })
        ));
        fixture.assert_no_key_access();
        assert_eq!(fixture.confirmation.calls.load(Ordering::SeqCst), 0);
    }
}

#[test]
fn direct_port_caps_the_deadline_and_rejects_expired_requests() {
    let fixture = Fixture::new(VaultNativeConfirmationOutcome::Confirmed, None);
    let capped = DesktopVaultRestoreService::new(fixture.services(), Duration::from_secs(3600));
    assert!(capped.deadline() <= Instant::now() + SESSION_LIFETIME);
    let expired = DesktopVaultRestoreService::new(fixture.services(), Duration::ZERO);
    assert_eq!(
        expired.prepare(session_request()),
        Err(VaultRestoreSessionHold::Locked)
    );
    assert!(matches!(
        expired.restore(),
        Err(VaultRestoreSessionFailure {
            reason: VaultRestoreSessionHold::Locked,
            can_reconnect: false
        })
    ));
    fixture.assert_no_key_access();
    assert_eq!(fixture.confirmation.calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn direct_port_cancellation_does_not_wait_for_prompt_or_revive_late_approval() {
    struct LateConfirmation(Arc<Gate>);
    impl VaultNativeConfirmationPort for LateConfirmation {
        fn confirm(
            &self,
            _: &VaultNativeConfirmationRequest,
            _: &AtomicBool,
            _: Instant,
        ) -> VaultNativeConfirmationOutcome {
            // 취소를 무시하는 느린 의존성도 서비스 상태를 되살릴 수 없어야 한다.
            self.0.wait();
            VaultNativeConfirmationOutcome::Confirmed
        }
    }
    let fixture = Fixture::new(VaultNativeConfirmationOutcome::Confirmed, None);
    let gate = Arc::new(Gate::default());
    let mut dependencies = fixture.services();
    dependencies.confirmation = Arc::new(LateConfirmation(gate.clone()));
    let port: Arc<dyn VaultRestoreSessionPort> = Arc::new(DesktopVaultRestoreService::new(
        dependencies,
        SESSION_LIFETIME,
    ));
    let preparing_port = port.clone();
    let preparing = tokio::task::spawn_blocking(move || preparing_port.prepare(session_request()));
    tokio::time::timeout(Duration::from_secs(2), gate.started.notified())
        .await
        .unwrap();

    let duplicate_port = port.clone();
    let duplicate = tokio::task::spawn_blocking(move || duplicate_port.prepare(session_request()));
    let duplicate_result = tokio::time::timeout(Duration::from_secs(2), duplicate).await;
    if duplicate_result.is_err() {
        gate.release();
    }
    assert_eq!(
        duplicate_result.unwrap().unwrap(),
        Err(VaultRestoreSessionHold::Locked)
    );

    let cancelling_port = port.clone();
    let cancelling = tokio::task::spawn_blocking(move || cancelling_port.cancel());
    let cancelled = tokio::time::timeout(Duration::from_secs(2), cancelling).await;
    gate.release();
    cancelled.unwrap().unwrap();
    assert_eq!(
        preparing.await.unwrap(),
        Err(VaultRestoreSessionHold::Locked)
    );
    assert!(!port.is_live());
    assert_eq!(
        port.prepare(session_request()),
        Err(VaultRestoreSessionHold::Locked)
    );
    assert!(matches!(
        port.restore(),
        Err(VaultRestoreSessionFailure {
            reason: VaultRestoreSessionHold::Locked,
            can_reconnect: false
        })
    ));
    fixture.assert_no_key_access();
}

#[tokio::test]
async fn closing_one_private_window_during_key_lookup_does_not_revive_it_or_cancel_another() {
    let fixture = Fixture::new(VaultNativeConfirmationOutcome::Confirmed, None);
    let (mut a, task_a) = start(fixture.services(), SESSION_LIFETIME);
    let (mut b, task_b) = start(fixture.services(), SESSION_LIFETIME);
    send(&mut a, &prepare_request()).await;
    assert_eq!(response(&mut a).await["result"]["kind"], "prepared");
    send(&mut b, &prepare_request()).await;
    assert_eq!(response(&mut b).await["result"]["kind"], "prepared");
    fixture.assert_no_key_access();
    let gate = Arc::new(Gate::default());
    *fixture.keys.gate.lock().unwrap() = Some(gate.clone());
    send(&mut a, &restore_request()).await;
    tokio::time::timeout(Duration::from_secs(3), gate.started.notified())
        .await
        .unwrap();
    a.get_mut().shutdown().await.unwrap();
    no_more_output(&mut a).await;
    task_a.await.unwrap().unwrap();
    *fixture.keys.gate.lock().unwrap() = None;
    gate.release();
    send(&mut b, &restore_request()).await;
    let restored = response(&mut b).await;
    assert_eq!(restored["result"]["title"], TITLE);
    assert_eq!(restored["result"]["body"], BODY);
    no_more_output(&mut b).await;
    task_b.await.unwrap().unwrap();
    // The cancelled A cannot read remote data; B has its own live grant.
    assert_eq!(fixture.remote.reads.load(Ordering::SeqCst), 1);
    assert_eq!(fixture.keys.stores.load(Ordering::SeqCst), 0);
}
