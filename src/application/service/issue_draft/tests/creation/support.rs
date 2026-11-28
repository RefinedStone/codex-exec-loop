//! Real facade orchestration with the existing policy/template fixture and memory boundaries.
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

use anyhow::{Result, bail};

use super::super::{
    FakeJournal, FakeProjectFiles, FakeRemoteIssue, FakeVaultKeyStoreAdapter, IssueDraftService,
    fixture as update_fixture,
};
use crate::application::port::inbound::issue_draft_port::IssueCreationTarget;
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
use crate::application::port::outbound::issue_policy_source_port::IssuePolicySourcePort;
use crate::application::port::outbound::issue_snapshot_port::{
    IssueSnapshotPort, IssueSnapshotReadRequest, IssueSnapshotResolveRequest, IssueSnapshotResponse,
};
use crate::application::port::outbound::issue_template_source_port::IssueTemplateSourcePort;
use crate::application::port::outbound::issue_update_port::{
    IssueUpdatePort, IssueUpdateRequest, IssueUpdateResponse,
};
use crate::application::service::evidence_vault_unlock::EvidenceVaultUnlockService;
use crate::application::service::issue_creation::IssueCreationService;
use crate::application::service::issue_policy::IssuePolicyService;
use crate::application::service::issue_publish_journal::IssueJournaledPublishService;
use crate::application::service::issue_template::IssueTemplateService;
use crate::domain::issue_creation::{
    IssueCreatedLocation, IssueCreationIntentRecord, IssueCreationIntentState,
    IssueRepositoryReference,
};
use crate::domain::issue_work::{IssueLocator, IssueProvider, IssueReference};

pub(super) fn target() -> IssueCreationTarget {
    let existing = super::super::target();
    let crate::application::port::inbound::issue_draft_port::IssueDraftLocation::GitHub {
        owner,
        repository,
        ..
    } = existing.location
    else {
        panic!("creation fixture requires GitHub")
    };
    IssueCreationTarget {
        cwd: existing.cwd,
        owner,
        repository,
    }
}

pub(super) struct CreationRemote {
    existing: Arc<FakeRemoteIssue>,
    pub(super) creates: AtomicUsize,
    pub(super) posts: Mutex<Vec<IssueCreateRequest>>,
    pub(super) resolves: Mutex<Vec<IssueSnapshotResolveRequest>>,
    pub(super) reads: Mutex<Vec<IssueReference>>,
    pub(super) updates: Mutex<Vec<IssueUpdateRequest>>,
}

impl IssueCreatePort for CreationRemote {
    fn resolve_repository(
        &self,
        request: IssueRepositoryResolveRequest,
    ) -> Result<IssueRepositoryReference> {
        Ok(IssueRepositoryReference {
            provider: IssueProvider::GitHub,
            server_id: "github.com".to_string(),
            repository_id: if request.owner == "sample" && request.repository == "widgets" {
                "R_1"
            } else {
                "R_OTHER"
            }
            .to_string(),
            project_scope: request.project_scope,
        })
    }

    fn create_issue(&self, request: IssueCreateRequest) -> Result<IssueCreateResponse> {
        self.creates.fetch_add(1, Ordering::SeqCst);
        self.posts.lock().unwrap().push(request.clone());
        request.ensure_authorized()?;
        *self.existing.fields.lock().unwrap() = request.shared_copy.clone();
        if self.existing.lose_answer {
            bail!("the create response was lost after the synthetic remote write")
        }
        Ok(IssueCreateResponse {
            created: IssueCreatedLocation {
                issue: IssueReference {
                    provider: request.repository.provider,
                    server_id: request.repository.server_id,
                    repository_id: request.repository.repository_id,
                    project_scope: request.repository.project_scope,
                    issue_id: "I_7".to_string(),
                },
                locator: IssueLocator {
                    owner: "sample".to_string(),
                    repository: "widgets".to_string(),
                    number: 7,
                },
                url: "https://github.com/sample/widgets/issues/7".to_string(),
            },
            published_copy: request.shared_copy,
        })
    }
}

impl IssueSnapshotPort for CreationRemote {
    fn resolve(&self, request: IssueSnapshotResolveRequest) -> Result<IssueSnapshotResponse> {
        self.resolves.lock().unwrap().push(request.clone());
        self.existing.resolve(request)
    }

    fn read(&self, request: IssueSnapshotReadRequest) -> Result<IssueSnapshotResponse> {
        self.reads.lock().unwrap().push(request.issue.clone());
        self.existing.read(request)
    }
}

impl IssueUpdatePort for CreationRemote {
    fn update_title_and_body(&self, request: IssueUpdateRequest) -> Result<IssueUpdateResponse> {
        self.updates.lock().unwrap().push(request.clone());
        self.existing.update_title_and_body(request)
    }
}

pub(super) struct ScopeSources {
    files: Arc<FakeProjectFiles>,
    pub(super) policy_workspaces: Mutex<Vec<String>>,
    pub(super) template_workspaces: Mutex<Vec<String>>,
}

impl IssuePolicySourcePort for ScopeSources {
    fn load_user_policy_toml(&self) -> Result<Option<String>> {
        self.files.load_user_policy_toml()
    }

    fn load_project_policy_toml(&self, workspace: &str) -> Result<Option<String>> {
        self.policy_workspaces
            .lock()
            .unwrap()
            .push(workspace.to_string());
        self.files.load_project_policy_toml(workspace)
    }
}

impl IssueTemplateSourcePort for ScopeSources {
    fn load_template(&self, workspace: &str, path: &str) -> Result<Option<String>> {
        self.template_workspaces
            .lock()
            .unwrap()
            .push(workspace.to_string());
        self.files.load_template(workspace, path)
    }
}

#[derive(Default)]
pub(super) struct CreationJournal {
    pub(super) records: Mutex<Vec<IssueCreationIntentRecord>>,
    pub(super) fail_before: Mutex<Option<IssueCreationIntentState>>,
}

struct Transaction<'a> {
    journal: &'a CreationJournal,
    records: MutexGuard<'a, Vec<IssueCreationIntentRecord>>,
}

impl IssueCreationJournalTransaction for Transaction<'_> {
    fn records(&mut self) -> Result<Vec<IssueCreationIntentRecord>> {
        Ok(self.records.clone())
    }

    fn save(&mut self, record: &IssueCreationIntentRecord) -> Result<()> {
        let mut failure = self.journal.fail_before.lock().unwrap();
        if *failure == Some(record.state) {
            *failure = None;
            bail!("the creation journal failed before saving this state")
        }
        if let Some(existing) = self
            .records
            .iter_mut()
            .find(|old| old.intent_id == record.intent_id)
        {
            *existing = record.clone();
        } else {
            self.records.push(record.clone());
        }
        Ok(())
    }
}

impl IssueCreationJournalPort for CreationJournal {
    fn canonical_workspace(&self, workspace_dir: &str) -> Result<String> {
        let workspace = workspace_dir.trim_end_matches("/.");
        Ok(match workspace {
            "/work/project-linked" | "/work/project/sub" => "/work/project",
            other => other,
        }
        .to_string())
    }

    fn begin(&self, _workspace_dir: &str) -> Result<Box<dyn IssueCreationJournalTransaction + '_>> {
        Ok(Box::new(Transaction {
            journal: self,
            records: self.records.lock().unwrap(),
        }))
    }
}

type StoredValue = (VaultRecordKey, VaultPlaintext);
pub(super) type StagedRecords = HashMap<VaultCreationRecordScope, StoredValue>;

/// The adapter's cipher is tested separately; this fake checks scope and key ownership.
#[derive(Default)]
pub(super) struct CreationVault {
    pub(super) staged: Mutex<StagedRecords>,
    records: Mutex<HashMap<VaultRecordScope, StoredValue>>,
    pub(super) stage_attempts: AtomicUsize,
    pub(super) lose_stage_acknowledgement: AtomicBool,
}

impl EvidenceVaultStorePort for CreationVault {
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
        crate::test_utils::vault_binding::begin_test_binding(
            self.records.lock().unwrap(),
            request,
            crate::test_utils::vault_binding::TestBindingHooks::default(),
        )
    }

    fn save_creation(&self, request: VaultCreationSaveRequest) -> Result<(), VaultStoreError> {
        self.stage_attempts.fetch_add(1, Ordering::SeqCst);
        self.staged
            .lock()
            .unwrap()
            .insert(request.scope, (request.key, request.plaintext));
        if self.lose_stage_acknowledgement.load(Ordering::SeqCst) {
            return Err(VaultStoreError::WriteOutcomeUnknown);
        }
        Ok(())
    }

    fn load_creation(
        &self,
        request: VaultCreationLoadRequest,
    ) -> Result<VaultPlaintext, VaultStoreError> {
        let (key, plaintext) = self
            .staged
            .lock()
            .unwrap()
            .get(&request.scope)
            .cloned()
            .ok_or(VaultStoreError::RecordNotFound)?;
        if key != request.key {
            return Err(VaultStoreError::IntegrityFailure);
        }
        Ok(plaintext)
    }

    fn save(&self, request: VaultRecordSaveRequest) -> Result<(), VaultStoreError> {
        self.records
            .lock()
            .unwrap()
            .insert(request.scope, (request.key, request.plaintext));
        Ok(())
    }

    fn load(&self, request: VaultRecordLoadRequest) -> Result<VaultPlaintext, VaultStoreError> {
        let (key, plaintext) = self
            .records
            .lock()
            .unwrap()
            .get(&request.scope)
            .cloned()
            .ok_or(VaultStoreError::RecordNotFound)?;
        if key != request.key {
            return Err(VaultStoreError::IntegrityFailure);
        }
        Ok(plaintext)
    }
}

pub(super) struct CreationFixture {
    pub(super) files: Arc<FakeProjectFiles>,
    pub(super) remote: Arc<CreationRemote>,
    pub(super) journal: Arc<CreationJournal>,
    pub(super) vault: Arc<CreationVault>,
    pub(super) service: IssueDraftService,
    pub(super) sources: Arc<ScopeSources>,
}

impl CreationFixture {
    pub(super) fn new(lose_answer: bool) -> Self {
        let existing = update_fixture(lose_answer);
        let remote = Arc::new(CreationRemote {
            existing: existing.remote.clone(),
            creates: AtomicUsize::new(0),
            posts: Mutex::new(Vec::new()),
            resolves: Mutex::new(Vec::new()),
            reads: Mutex::new(Vec::new()),
            updates: Mutex::new(Vec::new()),
        });
        let journal = Arc::new(CreationJournal::default());
        let vault = Arc::new(CreationVault::default());
        let publisher = Arc::new(IssueJournaledPublishService::new(
            remote.clone(),
            remote.clone(),
            Arc::new(FakeJournal::default()),
            vault.clone(),
        ));
        let creator = Arc::new(IssueCreationService::new(
            remote.clone(),
            remote.clone(),
            journal.clone(),
            vault.clone(),
            publisher.clone(),
        ));
        let sources = Arc::new(ScopeSources {
            files: existing.files.clone(),
            policy_workspaces: Mutex::new(Vec::new()),
            template_workspaces: Mutex::new(Vec::new()),
        });
        let mut service = existing.service;
        service.snapshot = remote.clone();
        service.policy = IssuePolicyService::new(sources.clone());
        service.template = IssueTemplateService::new(sources.clone());
        service.publisher = publisher;
        service.vault_unlock = Arc::new(EvidenceVaultUnlockService::new(Arc::new(
            FakeVaultKeyStoreAdapter::with_key(VaultKey::new(vec![4; 32])),
        )));
        Self {
            files: existing.files,
            remote,
            journal,
            vault,
            service: service.with_creation(creator),
            sources,
        }
    }

    pub(super) fn creates(&self) -> usize {
        self.remote.creates.load(Ordering::SeqCst)
    }

    pub(super) fn regular_scopes(&self) -> Vec<VaultRecordScope> {
        self.vault.records.lock().unwrap().keys().cloned().collect()
    }

    pub(super) fn staged(&self) -> StagedRecords {
        self.vault.staged.lock().unwrap().clone()
    }
}
