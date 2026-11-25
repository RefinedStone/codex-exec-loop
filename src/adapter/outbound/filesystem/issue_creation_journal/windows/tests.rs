use super::super::IssueCreationJournalAdapter;
use super::*;
use crate::adapter::outbound::filesystem::evidence_vault_store::FilesystemEvidenceVaultStoreAdapter;
use crate::adapter::outbound::filesystem::issue_publish_intent_journal::IssuePublishIntentJournalAdapter;
use crate::application::port::inbound::issue_draft_port::IssueCreationTarget;
use crate::application::port::outbound::evidence_vault_port::{
    EvidenceVaultStorePort, VaultKey, VaultPlaintext, VaultRecordBinding, VaultRecordField,
    VaultRecordKey, VaultRecordLoadRequest, VaultRecordScope,
};
use crate::application::port::outbound::issue_create_port::{
    IssueCreatePort, IssueCreateRequest, IssueCreateResponse, IssueRepositoryResolveRequest,
};
use crate::application::port::outbound::issue_creation_journal_port::IssueCreationJournalPort;
use crate::application::port::outbound::issue_creation_provenance_port::IssueCreationProvenancePort;
use crate::application::port::outbound::issue_publish_intent_journal_port::IssuePublishIntentJournalPort;
use crate::application::port::outbound::issue_snapshot_port::{
    IssueRawFields, IssueSnapshotPort, IssueSnapshotReadRequest, IssueSnapshotResolveRequest,
    IssueSnapshotResponse,
};
use crate::application::port::outbound::issue_update_port::{
    IssueUpdatePort, IssueUpdateRequest, IssueUpdateResponse,
};
use crate::application::service::issue_creation::{
    IssueCreationApprovedRequest, IssueCreationOutcome, IssueCreationService,
};
use crate::application::service::issue_publish_journal::{
    IssueJournaledPublishService, IssueRestorableField,
};
use crate::domain::issue_automation::source::IssueSourceRepository;
use crate::domain::issue_creation::{
    IssueCreatedLocation, IssueCreationApproval, IssueCreationApprovedCopy,
    IssueCreationIntentState, IssueRepositoryReference,
};
use crate::domain::issue_publish_intent::IssuePublishVaultField;
use crate::domain::issue_validation::{IssueRequirements, validate_issue};
use crate::domain::issue_work::{
    IssueComparisonBasis, IssueFieldHash, IssueLocator, IssueProjectScope, IssueProvider,
    IssueReference, IssueSharedCopy,
};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let root = crate::test_utils::platform_safe_temp_dir().join(format!(
            "akra-create-win-{}-{:016x}",
            std::process::id(),
            rand::random::<u64>()
        ));
        for dir in [
            &root,
            &root.join("workspace"),
            &root.join("runtime"),
            &root.join("vault"),
        ] {
            crate::private_directory::ensure_private_directory(dir).unwrap();
        }
        Self(fs::canonicalize(root).unwrap())
    }
    fn workspace(&self) -> String {
        self.0.join("workspace").to_str().unwrap().to_string()
    }
    fn runtime(&self) -> PathBuf {
        self.0.join("runtime")
    }
    fn adapter(&self) -> IssueCreationJournalAdapter {
        let runtime = self.runtime();
        IssueCreationJournalAdapter::new(move |_| Ok(runtime.clone()))
    }
    fn repository(&self) -> IssueRepositoryReference {
        IssueRepositoryReference {
            provider: IssueProvider::GitHub,
            server_id: "github.com".into(),
            repository_id: "R_WIDGET".into(),
            project_scope: IssueProjectScope {
                workspace_dir: self.workspace(),
            },
        }
    }
    fn source(&self) -> IssueSourceRepository {
        let r = self.repository();
        IssueSourceRepository {
            provider: r.provider,
            server_id: r.server_id,
            repository_id: r.repository_id,
            project_scope: r.project_scope,
        }
    }
    fn sent(&self) -> IssueCreationIntentRecord {
        let repository = self.repository();
        let copy = copy();
        IssueCreationIntentRecord {
            intent_id: "intent-1".into(),
            repository: repository.clone(),
            state: IssueCreationIntentState::Sent,
            approved: Some(IssueCreationApprovedCopy {
                approval: IssueCreationApproval {
                    repository,
                    intent_id: "intent-1".into(),
                    shared_copy_hash: IssueFieldHash::of_fields(&copy.title, &copy.body),
                    policy_version: "policy-1".into(),
                    template_version: None,
                },
                shared_copy: copy,
            }),
            pending_vault_fields: Vec::new(),
            created: None,
            observed_hash: None,
            write_response_mismatch: false,
            restore_link_finalized: false,
            shared_copy_only: false,
            predecessor_intent_id: None,
        }
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn copy() -> IssueSharedCopy {
    IssueSharedCopy {
        title: "Synthetic issue".into(),
        body: "Details: [private value removed]".into(),
    }
}

#[test]
fn absent_provenance_is_read_only_and_sent_is_visible_under_reservation() {
    let fixture = Fixture::new();
    let adapter = fixture.adapter();
    let empty = adapter.load(&fixture.source()).unwrap();
    assert!(!empty.unresolved && empty.created_issue_ids.is_empty());
    assert!(!fixture.runtime().join(JOURNAL_NAME).exists());
    let mut tx = adapter.begin(&fixture.workspace()).unwrap();
    let sent = fixture.sent();
    tx.save(&sent).unwrap();
    assert!(adapter.load(&fixture.source()).unwrap().unresolved);
    assert!(Transaction::open(fixture.runtime(), fixture.workspace(), Duration::ZERO).is_err());
    drop(tx);
    assert_eq!(
        fixture
            .adapter()
            .begin(&fixture.workspace())
            .unwrap()
            .records()
            .unwrap(),
        vec![sent]
    );
}

#[test]
fn corrupt_payload_schema_legacy_and_hardlink_fail_closed() {
    for sql in [
        "UPDATE issue_creation_journal SET payload = X'FF'",
        "DELETE FROM issue_creation_journal",
        "UPDATE issue_creation_journal SET payload = zeroblob(8388609)",
        "PRAGMA user_version = 2",
        "CREATE TABLE unexpected (v TEXT)",
    ] {
        let fixture = Fixture::new();
        let adapter = fixture.adapter();
        adapter
            .begin(&fixture.workspace())
            .unwrap()
            .save(&fixture.sent())
            .unwrap();
        Connection::open(fixture.runtime().join(JOURNAL_NAME))
            .unwrap()
            .execute_batch(sql)
            .unwrap();
        assert!(adapter.load(&fixture.source()).is_err(), "{sql}");
        assert!(adapter.begin(&fixture.workspace()).is_err(), "{sql}");
    }
    let fixture = Fixture::new();
    let path = fixture.runtime().join("issue-create-intents.jsonl");
    fs::write(&path, b"retain legacy").unwrap();
    assert!(fixture.adapter().load(&fixture.source()).is_err());
    assert!(fixture.adapter().begin(&fixture.workspace()).is_err());
    assert_eq!(fs::read(&path).unwrap(), b"retain legacy");
    fs::remove_file(path).unwrap();
    fixture
        .adapter()
        .begin(&fixture.workspace())
        .unwrap()
        .save(&fixture.sent())
        .unwrap();
    let alias = fixture.runtime().join("alias.db");
    fs::hard_link(fixture.runtime().join(JOURNAL_NAME), &alias).unwrap();
    assert!(fixture.adapter().load(&fixture.source()).is_err());
    assert!(fixture.adapter().begin(&fixture.workspace()).is_err());
}

#[test]
fn changed_approval_cannot_replace_durable_intent() {
    let fixture = Fixture::new();
    let adapter = fixture.adapter();
    let mut tx = adapter.begin(&fixture.workspace()).unwrap();
    let sent = fixture.sent();
    tx.save(&sent).unwrap();
    let mut changed = sent.clone();
    let approved = changed.approved.as_mut().unwrap();
    approved.shared_copy.title = "different".into();
    approved.approval.shared_copy_hash =
        IssueFieldHash::of_fields(&approved.shared_copy.title, &approved.shared_copy.body);
    assert!(tx.save(&changed).is_err());
    assert_eq!(tx.records().unwrap(), vec![sent]);
}

const CHILD_ROOT: &str = "AKRA_TEST_CREATE_JOURNAL_CHILD_ROOT";
#[test]
#[ignore = "child process fixture"]
fn child_holds_sent_creation_lease() {
    let root = std::env::var_os(CHILD_ROOT).expect("fixture root");
    let fixture = std::mem::ManuallyDrop::new(Fixture(PathBuf::from(root)));
    let adapter = fixture.adapter();
    let mut tx = adapter.begin(&fixture.workspace()).unwrap();
    tx.save(&fixture.sent()).unwrap();
    fs::write(fixture.0.join("ready"), b"sent").unwrap();
    loop {
        std::thread::sleep(Duration::from_secs(1));
    }
}

#[test]
fn forced_process_exit_releases_lease_but_preserves_sent() {
    let fixture = Fixture::new();
    let mut child=Command::new(std::env::current_exe().unwrap())
        .args(["--ignored","--exact","adapter::outbound::filesystem::issue_creation_journal::windows::tests::child_holds_sent_creation_lease"])
        .env(CHILD_ROOT,&fixture.0).stdout(Stdio::null()).stderr(Stdio::null()).spawn().unwrap();
    let start = Instant::now();
    while !fixture.0.join("ready").exists() && start.elapsed() < Duration::from_secs(20) {
        if child.try_wait().unwrap().is_some() {
            break;
        }
        std::thread::sleep(Duration::from_millis(30));
    }
    let ready = fixture.0.join("ready").exists();
    let held =
        ready && Transaction::open(fixture.runtime(), fixture.workspace(), Duration::ZERO).is_err();
    let _ = child.kill();
    child.wait().unwrap();
    assert!(ready && held);
    assert!(
        fixture
            .adapter()
            .load(&fixture.source())
            .unwrap()
            .unresolved
    );
    assert_eq!(
        fixture
            .adapter()
            .begin(&fixture.workspace())
            .unwrap()
            .records()
            .unwrap(),
        vec![fixture.sent()]
    );
}

struct Remote {
    journal: IssueCreationJournalAdapter,
    source: IssueSourceRepository,
    snapshot: Mutex<Option<IssueSnapshotResponse>>,
    creates: AtomicUsize,
}
impl IssueCreatePort for Remote {
    fn resolve_repository(
        &self,
        request: IssueRepositoryResolveRequest,
    ) -> Result<IssueRepositoryReference> {
        Ok(IssueRepositoryReference {
            provider: IssueProvider::GitHub,
            server_id: "github.com".into(),
            repository_id: "R_WIDGET".into(),
            project_scope: request.project_scope,
        })
    }
    fn create_issue(&self, request: IssueCreateRequest) -> Result<IssueCreateResponse> {
        request.ensure_authorized()?;
        assert!(self.journal.load(&self.source)?.unresolved);
        assert_eq!(request.shared_copy, copy());
        self.creates.fetch_add(1, Ordering::SeqCst);
        let issue = IssueReference {
            provider: request.repository.provider,
            server_id: request.repository.server_id,
            repository_id: request.repository.repository_id,
            project_scope: request.repository.project_scope,
            issue_id: "I_CREATED".into(),
        };
        let raw = IssueRawFields::new(request.shared_copy.title, request.shared_copy.body);
        *self.snapshot.lock().unwrap() = Some(IssueSnapshotResponse {
            issue,
            comparison_basis: IssueComparisonBasis {
                field_hash: raw.field_hash(),
                provider_updated_at: "2026-11-25T00:00:00Z".into(),
            },
            raw_fields: raw,
        });
        bail!("synthetic response lost after create")
    }
}
impl IssueSnapshotPort for Remote {
    fn resolve(&self, _: IssueSnapshotResolveRequest) -> Result<IssueSnapshotResponse> {
        self.snapshot
            .lock()
            .unwrap()
            .clone()
            .context("not yet created")
    }
    fn read(&self, _: IssueSnapshotReadRequest) -> Result<IssueSnapshotResponse> {
        self.snapshot
            .lock()
            .unwrap()
            .clone()
            .context("not yet created")
    }
}
impl IssueUpdatePort for Remote {
    fn update_title_and_body(&self, _: IssueUpdateRequest) -> Result<IssueUpdateResponse> {
        panic!("creation recovery must not update remote fields")
    }
}

#[test]
fn lost_response_recovers_by_stable_id_with_real_windows_journal_and_vault() {
    let fixture = Fixture::new();
    let remote = Arc::new(Remote {
        journal: fixture.adapter(),
        source: fixture.source(),
        snapshot: Mutex::new(None),
        creates: AtomicUsize::new(0),
    });
    let vault = Arc::new(FilesystemEvidenceVaultStoreAdapter::new(
        fixture.0.join("vault"),
    ));
    let runtime = fixture.runtime();
    let publication = Arc::new(IssuePublishIntentJournalAdapter::new(move |_| {
        Ok(runtime.clone())
    }));
    let service = || {
        IssueCreationService::new(
            remote.clone(),
            remote.clone(),
            Arc::new(fixture.adapter()),
            vault.clone(),
            Arc::new(IssueJournaledPublishService::new(
                remote.clone(),
                remote.clone(),
                publication.clone(),
                vault.clone(),
            )),
        )
    };
    let target = IssueCreationTarget {
        cwd: fixture.workspace(),
        owner: "acme".into(),
        repository: "widget".into(),
    };
    let opened = service().open(&target).unwrap();
    let key = VaultRecordKey {
        key_id: "synthetic-test-key".into(),
        key: VaultKey::new(vec![7; 32]),
    };
    let raw = "synthetic secret must never appear in journal or shared copy";
    let copy = copy();
    let request = || IssueCreationApprovedRequest {
        target: target.clone(),
        intent_id: opened.intent_id.clone(),
        validated: validate_issue(&copy, &IssueRequirements::default()).unwrap(),
        approval: IssueCreationApproval {
            repository: opened.repository.clone(),
            intent_id: opened.intent_id.clone(),
            shared_copy_hash: IssueFieldHash::of_fields(&copy.title, &copy.body),
            policy_version: "p1".into(),
            template_version: None,
        },
        policy_version: "p1".into(),
        template_version: None,
        restorable: vec![IssueRestorableField {
            field: IssuePublishVaultField::Body,
            plaintext: VaultPlaintext::new(raw.into()),
        }],
        vault_key: Some(key.clone()),
        shared_copy_only: false,
    };
    assert!(
        matches!(service().publish(request()).unwrap(),IssueCreationOutcome::Intent(record) if record.state==IssueCreationIntentState::NeedsConfirmation)
    );
    assert!(
        service()
            .recheck(&target, &opened.intent_id, Some(key.clone()))
            .is_ok()
    );
    let _ = service().publish(request()).unwrap();
    assert_eq!(remote.creates.load(Ordering::SeqCst), 1);
    let confirmed = service()
        .confirm(
            &target,
            &opened.intent_id,
            "https://github.com/acme/widget/issues/7",
            Some(key.clone()),
        )
        .unwrap();
    assert_eq!(confirmed.state, IssueCreationIntentState::Confirmed);
    assert!(confirmed.restore_link_finalized);
    assert_eq!(remote.creates.load(Ordering::SeqCst), 1);
    let issue = confirmed.created.unwrap().issue;
    let provenance = fixture.adapter().load(&fixture.source()).unwrap();
    assert!(provenance.created_issue_ids.contains("I_CREATED") && !provenance.unresolved);
    let version = vault
        .load(VaultRecordLoadRequest {
            scope: VaultRecordScope::current_version(issue.clone()),
            key: key.clone(),
        })
        .unwrap();
    let body = vault
        .load(VaultRecordLoadRequest {
            scope: VaultRecordScope {
                issue,
                field: VaultRecordField::Body,
                binding: VaultRecordBinding::ConfirmedVersion(version.expose().to_string()),
            },
            key,
        })
        .unwrap();
    assert_eq!(body.expose(), raw);
    let records = read_validated_records(&fixture.runtime()).unwrap();
    assert!(!serde_json::to_string(&records).unwrap().contains(raw));
    assert_eq!(publication.load(&fixture.workspace()).unwrap().len(), 1);
}
