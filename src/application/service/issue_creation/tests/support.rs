use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, TryLockError};

use anyhow::{Result, anyhow, bail};

use super::super::{IssueCreationApprovedRequest, IssueCreationOutcome, IssueCreationService};
use crate::application::port::inbound::issue_creation_draft::IssueCreationTarget;
use crate::application::port::outbound::evidence_vault_port::{
    EvidenceVaultStorePort, VaultCreationLoadRequest, VaultCreationRecordScope,
    VaultCreationSaveRequest, VaultKey, VaultPlaintext, VaultRecordKey, VaultRecordLoadRequest,
    VaultRecordSaveRequest, VaultRecordScope, VaultStoreError,
};
use crate::application::port::outbound::issue_create_port::{
    IssueCreatePort, IssueCreateRequest, IssueCreateResponse, IssueRepositoryResolveRequest,
};
use crate::application::port::outbound::issue_creation_journal_port::{
    IssueCreationJournalPort, IssueCreationJournalTransaction,
};
use crate::application::port::outbound::issue_publish_intent_journal_port::{
    IssuePublicationLease, IssuePublishIntentJournalPort,
};
use crate::application::port::outbound::issue_snapshot_port::{
    IssueRawFields, IssueSnapshotPort, IssueSnapshotReadRequest, IssueSnapshotResolveRequest,
    IssueSnapshotResponse,
};
use crate::application::port::outbound::issue_update_port::{
    IssueUpdatePort, IssueUpdateRequest, IssueUpdateResponse,
};
use crate::application::service::issue_publish_journal::{
    IssueJournaledPublishService, IssueRestorableField,
};
use crate::domain::issue_creation::{
    IssueCreatedLocation, IssueCreationApproval, IssueCreationIntentRecord,
    IssueCreationIntentState, IssueRepositoryReference,
};
use crate::domain::issue_publish_intent::{IssuePublishIntentRecord, IssuePublishVaultField};
use crate::domain::issue_validation::{IssueRequirements, validate_issue};
use crate::domain::issue_work::{
    IssueComparisonBasis, IssueFieldHash, IssueLocator, IssueProjectScope, IssueProvider,
    IssueReference, IssueSharedCopy,
};

type UnitHook = Arc<dyn Fn() + Send + Sync>;
type VaultSaveHook = Arc<dyn Fn(&VaultRecordSaveRequest) + Send + Sync>;

pub(super) const WORKSPACE: &str = "/work/project";
pub(super) const CREATED_URL: &str = "https://github.com/acme/widget/issues/7";
pub(super) const RAW_FIRST: &str = "original-private-sentinel-one";
pub(super) const RAW_SECOND: &str = "different-private-sentinel-two";

pub(super) fn target() -> IssueCreationTarget {
    IssueCreationTarget {
        cwd: WORKSPACE.to_string(),
        owner: "acme".to_string(),
        repository: "widget".to_string(),
    }
}

pub(super) fn repository() -> IssueRepositoryReference {
    IssueRepositoryReference {
        provider: IssueProvider::GitHub,
        server_id: "github.com".to_string(),
        repository_id: "R_WIDGET".to_string(),
        project_scope: IssueProjectScope {
            workspace_dir: WORKSPACE.to_string(),
        },
    }
}

pub(super) fn issue() -> IssueReference {
    let repository = repository();
    IssueReference {
        provider: repository.provider,
        server_id: repository.server_id,
        repository_id: repository.repository_id,
        project_scope: repository.project_scope,
        issue_id: "I_CREATED".to_string(),
    }
}

pub(super) fn shared_copy() -> IssueSharedCopy {
    IssueSharedCopy {
        title: "Masked issue".to_string(),
        body: "Details: [private value removed]".to_string(),
    }
}

pub(super) fn key() -> VaultRecordKey {
    VaultRecordKey {
        key_id: "test-key".to_string(),
        key: VaultKey::new(vec![7; 32]),
    }
}

pub(super) fn location() -> IssueCreatedLocation {
    IssueCreatedLocation {
        issue: issue(),
        locator: IssueLocator {
            owner: "acme".to_string(),
            repository: "widget".to_string(),
            number: 7,
        },
        url: CREATED_URL.to_string(),
    }
}

pub(super) fn snapshot(issue: IssueReference, copy: IssueSharedCopy) -> IssueSnapshotResponse {
    let raw_fields = IssueRawFields::new(copy.title, copy.body);
    IssueSnapshotResponse {
        issue,
        comparison_basis: IssueComparisonBasis {
            field_hash: raw_fields.field_hash(),
            provider_updated_at: "2026-11-20T00:00:00Z".to_string(),
        },
        raw_fields,
    }
}

pub(super) fn request(intent_id: &str, raw: Option<&str>) -> IssueCreationApprovedRequest {
    let copy = shared_copy();
    IssueCreationApprovedRequest {
        target: target(),
        intent_id: intent_id.to_string(),
        validated: validate_issue(&copy, &IssueRequirements::default()).unwrap(),
        approval: IssueCreationApproval {
            repository: repository(),
            intent_id: intent_id.to_string(),
            shared_copy_hash: IssueFieldHash::of_fields(&copy.title, &copy.body),
            policy_version: "policy-v1".to_string(),
            template_version: Some("template-v1".to_string()),
        },
        policy_version: "policy-v1".to_string(),
        template_version: Some("template-v1".to_string()),
        restorable: raw
            .map(|raw| IssueRestorableField {
                field: IssuePublishVaultField::Body,
                plaintext: VaultPlaintext::new(raw.to_string()),
            })
            .into_iter()
            .collect(),
        vault_key: raw.map(|_| key()),
        shared_copy_only: false,
    }
}

pub(super) fn intent(outcome: IssueCreationOutcome) -> IssueCreationIntentRecord {
    match outcome {
        IssueCreationOutcome::Intent(record) => *record,
        IssueCreationOutcome::Refused(reason) => panic!("unexpected refusal: {reason}"),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum CreateBehavior {
    Answer,
    ApplyAndLoseAnswer,
    FailWithoutApply,
    WrongRepositoryAnswer,
    WrongCopyAnswer,
}

pub(super) struct FakeRemote {
    behavior: CreateBehavior,
    pub(super) creates: AtomicUsize,
    pub(super) updates: AtomicUsize,
    pub(super) reads: AtomicUsize,
    pub(super) resolves: AtomicUsize,
    pub(super) fail_reads: AtomicBool,
    pub(super) allow_updates: AtomicBool,
    pub(super) snapshots: Mutex<HashMap<u64, IssueSnapshotResponse>>,
    pub(super) posts: Mutex<Vec<IssueCreateRequest>>,
    pub(super) before_create: Mutex<Option<UnitHook>>,
}

impl FakeRemote {
    fn new(behavior: CreateBehavior) -> Self {
        Self {
            behavior,
            creates: AtomicUsize::new(0),
            updates: AtomicUsize::new(0),
            reads: AtomicUsize::new(0),
            resolves: AtomicUsize::new(0),
            fail_reads: AtomicBool::new(false),
            allow_updates: AtomicBool::new(false),
            snapshots: Mutex::new(HashMap::new()),
            posts: Mutex::new(Vec::new()),
            before_create: Mutex::new(None),
        }
    }
}

impl IssueCreatePort for FakeRemote {
    fn resolve_repository(
        &self,
        request: IssueRepositoryResolveRequest,
    ) -> Result<IssueRepositoryReference> {
        let mut resolved = repository();
        resolved.project_scope = request.project_scope;
        if request.owner != "acme" || request.repository != "widget" {
            resolved.repository_id = "R_OTHER".to_string();
        }
        Ok(resolved)
    }

    fn create_issue(&self, request: IssueCreateRequest) -> Result<IssueCreateResponse> {
        self.creates.fetch_add(1, Ordering::SeqCst);
        self.posts.lock().unwrap().push(request.clone());
        request.ensure_authorized()?;
        if let Some(before_create) = self.before_create.lock().unwrap().clone() {
            before_create();
        }
        if self.behavior == CreateBehavior::FailWithoutApply {
            bail!("remote connection failed without a usable response")
        }
        self.snapshots
            .lock()
            .unwrap()
            .insert(7, snapshot(issue(), request.shared_copy.clone()));
        if self.behavior == CreateBehavior::ApplyAndLoseAnswer {
            bail!("remote issue was created but the response was lost")
        }
        let mut created = location();
        let mut published_copy = request.shared_copy;
        if self.behavior == CreateBehavior::WrongRepositoryAnswer {
            created.issue.repository_id = "R_OTHER".to_string();
        }
        if self.behavior == CreateBehavior::WrongCopyAnswer {
            published_copy.body = "Contradictory response".to_string();
        }
        Ok(IssueCreateResponse {
            created,
            published_copy,
        })
    }
}

impl IssueSnapshotPort for FakeRemote {
    fn resolve(&self, request: IssueSnapshotResolveRequest) -> Result<IssueSnapshotResponse> {
        self.resolves.fetch_add(1, Ordering::SeqCst);
        if self.fail_reads.load(Ordering::SeqCst) {
            bail!("remote lookup is unavailable")
        }
        let mut response = self
            .snapshots
            .lock()
            .unwrap()
            .get(&request.locator.number)
            .cloned()
            .ok_or_else(|| anyhow!("the requested issue was not found"))?;
        response.issue.project_scope = request.project_scope;
        if request.locator.owner != "acme" || request.locator.repository != "widget" {
            response.issue.repository_id = "R_OTHER".to_string();
        }
        Ok(response)
    }

    fn read(&self, request: IssueSnapshotReadRequest) -> Result<IssueSnapshotResponse> {
        self.reads.fetch_add(1, Ordering::SeqCst);
        if self.fail_reads.load(Ordering::SeqCst) {
            bail!("remote read is unavailable")
        }
        self.snapshots
            .lock()
            .unwrap()
            .values()
            .find(|snapshot| snapshot.issue.issue_id == request.issue.issue_id)
            .cloned()
            .ok_or_else(|| anyhow!("the requested stable issue was not found"))
    }
}

impl IssueUpdatePort for FakeRemote {
    fn update_title_and_body(&self, request: IssueUpdateRequest) -> Result<IssueUpdateResponse> {
        self.updates.fetch_add(1, Ordering::SeqCst);
        if !self.allow_updates.load(Ordering::SeqCst) {
            bail!("creation and adoption must not update an existing remote issue")
        }
        request.ensure_authorized()?;
        self.snapshots.lock().unwrap().insert(
            7,
            snapshot(request.issue.clone(), request.shared_copy.clone()),
        );
        Ok(IssueUpdateResponse {
            issue: request.issue,
            published_copy: request.shared_copy,
        })
    }
}

#[derive(Default)]
pub(super) struct MemoryCreationJournal {
    pub(super) records: Mutex<Vec<IssueCreationIntentRecord>>,
    pub(super) history: Mutex<Vec<IssueCreationIntentState>>,
    pub(super) fail_before: Mutex<Option<IssueCreationIntentState>>,
    pub(super) fail_after: Mutex<Option<IssueCreationIntentState>>,
}

struct MemoryTransaction<'a> {
    journal: &'a MemoryCreationJournal,
    records: MutexGuard<'a, Vec<IssueCreationIntentRecord>>,
}

impl IssueCreationJournalTransaction for MemoryTransaction<'_> {
    fn records(&mut self) -> Result<Vec<IssueCreationIntentRecord>> {
        Ok(self.records.clone())
    }

    fn save(&mut self, record: &IssueCreationIntentRecord) -> Result<()> {
        let mut failure = self.journal.fail_before.lock().unwrap();
        if *failure == Some(record.state) {
            *failure = None;
            bail!("journal write failed before committing the state")
        }
        drop(failure);
        if let Some(existing) = self
            .records
            .iter_mut()
            .find(|old| old.intent_id == record.intent_id)
        {
            *existing = record.clone();
        } else {
            self.records.push(record.clone());
        }
        self.journal.history.lock().unwrap().push(record.state);
        let mut failure = self.journal.fail_after.lock().unwrap();
        if *failure == Some(record.state) {
            *failure = None;
            bail!("journal write committed but its acknowledgement was lost")
        }
        Ok(())
    }
}

impl IssueCreationJournalPort for MemoryCreationJournal {
    fn canonical_workspace(&self, workspace_dir: &str) -> Result<String> {
        Ok(workspace_dir.trim_end_matches("/.").to_string())
    }

    fn begin(&self, _workspace_dir: &str) -> Result<Box<dyn IssueCreationJournalTransaction + '_>> {
        Ok(Box::new(MemoryTransaction {
            journal: self,
            records: self.records.lock().unwrap(),
        }))
    }
}

#[derive(Default)]
pub(super) struct MemoryPublishJournal {
    pub(super) records: Mutex<Vec<IssuePublishIntentRecord>>,
    pub(super) fail_save: AtomicBool,
    publication: Mutex<()>,
    pub(super) on_lease_contended: Mutex<Option<UnitHook>>,
}

struct MemoryPublicationLease<'a> {
    _guard: MutexGuard<'a, ()>,
}

impl IssuePublicationLease for MemoryPublicationLease<'_> {}

impl IssuePublishIntentJournalPort for MemoryPublishJournal {
    fn acquire_publication_lease(
        &self,
        _workspace_dir: &str,
    ) -> Result<Box<dyn IssuePublicationLease + '_>> {
        let guard = match self.publication.try_lock() {
            Ok(guard) => guard,
            Err(TryLockError::WouldBlock) => {
                if let Some(hook) = self.on_lease_contended.lock().unwrap().clone() {
                    hook();
                }
                self.publication.lock().unwrap()
            }
            Err(TryLockError::Poisoned(_)) => bail!("publication lease was poisoned"),
        };
        Ok(Box::new(MemoryPublicationLease { _guard: guard }))
    }

    fn save(&self, _workspace_dir: &str, record: &IssuePublishIntentRecord) -> Result<()> {
        if self.fail_save.load(Ordering::SeqCst) {
            bail!("publish adoption journal is unavailable")
        }
        let mut records = self.records.lock().unwrap();
        records.retain(|old| old.intent_id != record.intent_id);
        records.push(record.clone());
        Ok(())
    }

    fn load(&self, _workspace_dir: &str) -> Result<Vec<IssuePublishIntentRecord>> {
        Ok(self.records.lock().unwrap().clone())
    }
}

/// This fake checks authenticated scope/key ownership, not the production cipher.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct SealedRecord {
    pub(super) key: VaultRecordKey,
    pub(super) plaintext: VaultPlaintext,
}

impl SealedRecord {
    fn open(&self, key: &VaultRecordKey) -> Result<VaultPlaintext, VaultStoreError> {
        if key.key_id != self.key.key_id {
            return Err(VaultStoreError::KeyMismatch);
        }
        if key.key != self.key.key {
            return Err(VaultStoreError::IntegrityFailure);
        }
        Ok(self.plaintext.clone())
    }
}

#[derive(Default)]
pub(super) struct FakeVault {
    pub(super) creation: Mutex<HashMap<VaultCreationRecordScope, SealedRecord>>,
    pub(super) records: Mutex<HashMap<VaultRecordScope, SealedRecord>>,
    pub(super) creation_attempts: AtomicUsize,
    pub(super) fail_creation: AtomicBool,
    pub(super) fail_regular: AtomicBool,
    pub(super) before_save: Mutex<Option<VaultSaveHook>>,
}

impl EvidenceVaultStorePort for FakeVault {
    fn save_creation(&self, request: VaultCreationSaveRequest) -> Result<(), VaultStoreError> {
        self.creation_attempts.fetch_add(1, Ordering::SeqCst);
        if self.fail_creation.load(Ordering::SeqCst) {
            return Err(VaultStoreError::Storage(
                "creation staging is unavailable".to_string(),
            ));
        }
        self.creation.lock().unwrap().insert(
            request.scope,
            SealedRecord {
                key: request.key,
                plaintext: request.plaintext,
            },
        );
        Ok(())
    }

    fn load_creation(
        &self,
        request: VaultCreationLoadRequest,
    ) -> Result<VaultPlaintext, VaultStoreError> {
        self.creation
            .lock()
            .unwrap()
            .get(&request.scope)
            .ok_or(VaultStoreError::RecordNotFound)?
            .open(&request.key)
    }

    fn save(&self, request: VaultRecordSaveRequest) -> Result<(), VaultStoreError> {
        let hook = self.before_save.lock().unwrap().clone();
        if let Some(hook) = hook {
            hook(&request);
        }
        if self.fail_regular.load(Ordering::SeqCst) {
            return Err(VaultStoreError::Storage(
                "adopted vault storage is unavailable".to_string(),
            ));
        }
        self.records.lock().unwrap().insert(
            request.scope,
            SealedRecord {
                key: request.key,
                plaintext: request.plaintext,
            },
        );
        Ok(())
    }

    fn load(&self, request: VaultRecordLoadRequest) -> Result<VaultPlaintext, VaultStoreError> {
        self.records
            .lock()
            .unwrap()
            .get(&request.scope)
            .ok_or(VaultStoreError::RecordNotFound)?
            .open(&request.key)
    }
}

pub(super) struct Fixture {
    pub(super) remote: Arc<FakeRemote>,
    pub(super) journal: Arc<MemoryCreationJournal>,
    pub(super) publish_journal: Arc<MemoryPublishJournal>,
    pub(super) vault: Arc<FakeVault>,
}

impl Fixture {
    pub(super) fn new(behavior: CreateBehavior) -> Self {
        Self {
            remote: Arc::new(FakeRemote::new(behavior)),
            journal: Arc::new(MemoryCreationJournal::default()),
            publish_journal: Arc::new(MemoryPublishJournal::default()),
            vault: Arc::new(FakeVault::default()),
        }
    }

    pub(super) fn service(&self) -> IssueCreationService {
        IssueCreationService::new(
            self.remote.clone(),
            self.remote.clone(),
            self.journal.clone(),
            self.vault.clone(),
            Arc::new(self.publisher()),
        )
    }

    pub(super) fn publisher(&self) -> IssueJournaledPublishService {
        IssueJournaledPublishService::new(
            self.remote.clone(),
            self.remote.clone(),
            self.publish_journal.clone(),
            self.vault.clone(),
        )
    }

    pub(super) fn creates(&self) -> usize {
        self.remote.creates.load(Ordering::SeqCst)
    }

    pub(super) fn staged(&self) -> HashMap<VaultCreationRecordScope, SealedRecord> {
        self.vault.creation.lock().unwrap().clone()
    }

    pub(super) fn current(&self) -> Option<String> {
        self.vault
            .records
            .lock()
            .unwrap()
            .get(&VaultRecordScope::current_version(issue()))
            .map(|record| record.plaintext.expose().to_string())
    }
}
