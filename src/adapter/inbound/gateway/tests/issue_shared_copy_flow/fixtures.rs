use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use anyhow::{Result, bail};
use serde_json::{Value, json};
use tokio::io::DuplexStream;
use tokio::task::JoinHandle;
use tokio_tungstenite::WebSocketStream;

use super::super::super::hosts::GatewayHosts;
use super::super::super::session::{GatewayServices, run_session, websocket_config};
use super::super::{DUPLEX_BUFFER_BYTES, EchoUpstream, kernel_request};
use super::isolation;
use crate::adapter::outbound::db::sqlite_planning_authority_adapter::SqlitePlanningAuthorityAdapter;
use crate::adapter::outbound::filesystem::issue_automation_policy::FilesystemIssueAutomationPolicySourceAdapter;
use crate::adapter::outbound::filesystem::issue_policy::FilesystemIssuePolicySourceAdapter;
use crate::adapter::outbound::filesystem::issue_template::FilesystemIssueTemplateSourceAdapter;
use crate::adapter::outbound::issue_code_validator::SandboxedIssueCodeValidatorRunnerAdapter;
use crate::application::port::outbound::evidence_vault_port::{
    EvidenceVaultStorePort, VaultBindingTransaction, VaultCreationLoadRequest,
    VaultCreationSaveRequest, VaultKey, VaultPlaintext, VaultRecordLoadRequest,
    VaultRecordSaveRequest, VaultStoreError,
};
use crate::application::port::outbound::issue_snapshot_port::{
    IssueRawFields, IssueSnapshotPort, IssueSnapshotReadRequest, IssueSnapshotResolveRequest,
    IssueSnapshotResponse,
};
use crate::application::port::outbound::issue_update_port::{
    IssueUpdatePort, IssueUpdateRequest, IssueUpdateResponse,
};
use crate::application::port::outbound::vault_key_store_port::{
    VaultKeyStoreError, VaultKeyStorePort,
};
use crate::application::service::evidence_vault_unlock::EvidenceVaultUnlockService;
use crate::application::service::issue_automation_policy::IssueAutomationPolicyService;
use crate::application::service::issue_draft::IssueDraftService;
use crate::application::service::issue_policy::IssuePolicyService;
use crate::application::service::issue_publish_journal::IssueJournaledPublishService;
use crate::application::service::issue_template::IssueTemplateService;
use crate::composition::production::build_issue_publish_journal;
use crate::domain::issue_work::{
    IssueComparisonBasis, IssueProjectScope, IssueProvider, IssueReference, IssueSharedCopy,
};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct ApiCalls {
    pub key_load: usize,
    pub key_get_or_create: usize,
    pub vault_binding: usize,
    pub vault_save: usize,
    pub vault_load: usize,
    pub vault_creation_save: usize,
    pub vault_creation_load: usize,
}

#[derive(Default)]
struct SensitiveApis {
    key_load: AtomicUsize,
    key_get_or_create: AtomicUsize,
    vault_binding: AtomicUsize,
    vault_save: AtomicUsize,
    vault_load: AtomicUsize,
    vault_creation_save: AtomicUsize,
    vault_creation_load: AtomicUsize,
}

impl SensitiveApis {
    fn counts(&self) -> ApiCalls {
        ApiCalls {
            key_load: self.key_load.load(Ordering::SeqCst),
            key_get_or_create: self.key_get_or_create.load(Ordering::SeqCst),
            vault_binding: self.vault_binding.load(Ordering::SeqCst),
            vault_save: self.vault_save.load(Ordering::SeqCst),
            vault_load: self.vault_load.load(Ordering::SeqCst),
            vault_creation_save: self.vault_creation_save.load(Ordering::SeqCst),
            vault_creation_load: self.vault_creation_load.load(Ordering::SeqCst),
        }
    }

    fn unavailable() -> VaultStoreError {
        VaultStoreError::Storage("synthetic Vault is unavailable".to_string())
    }
}

impl VaultKeyStorePort for SensitiveApis {
    fn load_key(&self) -> Result<Option<VaultKey>, VaultKeyStoreError> {
        self.key_load.fetch_add(1, Ordering::SeqCst);
        Err(VaultKeyStoreError::Unsupported)
    }

    fn get_or_create_key(&self) -> Result<VaultKey, VaultKeyStoreError> {
        self.key_get_or_create.fetch_add(1, Ordering::SeqCst);
        Err(VaultKeyStoreError::Unsupported)
    }
}

impl EvidenceVaultStorePort for SensitiveApis {
    fn begin_binding(
        &self,
        _: VaultRecordLoadRequest,
    ) -> Result<Box<dyn VaultBindingTransaction + '_>, VaultStoreError> {
        self.vault_binding.fetch_add(1, Ordering::SeqCst);
        Err(Self::unavailable())
    }

    fn save(&self, _: VaultRecordSaveRequest) -> Result<(), VaultStoreError> {
        self.vault_save.fetch_add(1, Ordering::SeqCst);
        Err(Self::unavailable())
    }

    fn load(&self, _: VaultRecordLoadRequest) -> Result<VaultPlaintext, VaultStoreError> {
        self.vault_load.fetch_add(1, Ordering::SeqCst);
        Err(Self::unavailable())
    }

    fn save_creation(&self, _: VaultCreationSaveRequest) -> Result<(), VaultStoreError> {
        self.vault_creation_save.fetch_add(1, Ordering::SeqCst);
        Err(Self::unavailable())
    }

    fn load_creation(
        &self,
        _: VaultCreationLoadRequest,
    ) -> Result<VaultPlaintext, VaultStoreError> {
        self.vault_creation_load.fetch_add(1, Ordering::SeqCst);
        Err(Self::unavailable())
    }
}

#[derive(Clone, Copy)]
pub(super) enum WriteBehavior {
    Apply,
    LostResponse { applied: bool, fail_reads: bool },
}

struct GithubState {
    copy: IssueSharedCopy,
    writes: usize,
    behavior: WriteBehavior,
    fail_reads: bool,
    pending: Option<IssueSharedCopy>,
}

struct GithubDouble(Mutex<GithubState>);

impl GithubDouble {
    fn new() -> Self {
        Self(Mutex::new(GithubState {
            copy: IssueSharedCopy {
                title: "Old synthetic title".to_string(),
                body: "Old synthetic body".to_string(),
            },
            writes: 0,
            behavior: WriteBehavior::Apply,
            fail_reads: false,
            pending: None,
        }))
    }

    fn snapshot(&self, issue: IssueReference) -> Result<IssueSnapshotResponse> {
        let state = self.0.lock().unwrap();
        if state.fail_reads {
            bail!("synthetic provider lookup failed");
        }
        let raw_fields = IssueRawFields::new(state.copy.title.clone(), state.copy.body.clone());
        Ok(IssueSnapshotResponse {
            issue,
            comparison_basis: IssueComparisonBasis {
                field_hash: raw_fields.field_hash(),
                provider_updated_at: "2026-11-23T00:00:00Z".to_string(),
            },
            raw_fields,
        })
    }
}

impl IssueSnapshotPort for GithubDouble {
    fn resolve(&self, request: IssueSnapshotResolveRequest) -> Result<IssueSnapshotResponse> {
        assert_eq!(request.locator.number, 7);
        self.snapshot(reference(request.project_scope))
    }

    fn read(&self, request: IssueSnapshotReadRequest) -> Result<IssueSnapshotResponse> {
        self.snapshot(request.issue)
    }
}

impl IssueUpdatePort for GithubDouble {
    fn update_title_and_body(&self, request: IssueUpdateRequest) -> Result<IssueUpdateResponse> {
        request.ensure_authorized()?;
        if request.issue.issue_id != "I_7" {
            bail!("synthetic provider only contains issue 7");
        }
        let mut state = self.0.lock().unwrap();
        state.writes += 1;
        match state.behavior {
            WriteBehavior::Apply => state.copy = request.shared_copy.clone(),
            WriteBehavior::LostResponse {
                applied,
                fail_reads,
            } => {
                if applied {
                    state.copy = request.shared_copy.clone();
                } else {
                    state.pending = Some(request.shared_copy.clone());
                }
                state.fail_reads = fail_reads;
                bail!("synthetic response was lost");
            }
        }
        Ok(IssueUpdateResponse {
            issue: request.issue,
            published_copy: request.shared_copy,
        })
    }
}

fn reference(project_scope: IssueProjectScope) -> IssueReference {
    IssueReference {
        provider: IssueProvider::GitHub,
        server_id: "github.com".to_string(),
        repository_id: "R_synthetic".to_string(),
        issue_id: "I_7".to_string(),
        project_scope,
    }
}

pub(super) struct World {
    pub workspace: PathBuf,
    github: Arc<GithubDouble>,
    sensitive: Arc<SensitiveApis>,
}

impl World {
    pub fn new() -> Self {
        let root = isolation::root();
        isolation::prepare_directory(&root.join("user"));
        let workspace = root.join("repository");
        isolation::prepare_directory(&workspace);
        let output = crate::test_utils::git_command()
            .args(["init", "--quiet"])
            .arg(&workspace)
            .output()
            .expect("initialize synthetic repository");
        assert!(
            output.status.success(),
            "synthetic repository init failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        Self {
            workspace,
            github: Arc::new(GithubDouble::new()),
            sensitive: Arc::new(SensitiveApis::default()),
        }
    }

    pub fn target(&self) -> Value {
        json!({
            "cwd": self.workspace.to_str().unwrap(),
            "owner": "synthetic",
            "repository": "shared-copy-fixture",
            "number": 7,
        })
    }

    pub fn counts(&self) -> ApiCalls {
        self.sensitive.counts()
    }

    pub fn writes(&self) -> usize {
        self.github.0.lock().unwrap().writes
    }

    pub fn remote_copy(&self) -> IssueSharedCopy {
        self.github.0.lock().unwrap().copy.clone()
    }

    pub fn runtime(&self) -> PathBuf {
        let location = SqlitePlanningAuthorityAdapter::prepare_private_runtime_directory(
            self.workspace.to_str().unwrap(),
        )
        .expect("production private runtime resolver");
        let path = PathBuf::from(location.runtime_dir);
        assert!(path.starts_with(isolation::root()));
        assert!(!path.starts_with(&self.workspace));
        path
    }

    pub fn set_behavior(&self, behavior: WriteBehavior) {
        self.github.0.lock().unwrap().behavior = behavior;
    }

    pub fn recover_reads(&self, apply_pending: bool) {
        let mut state = self.github.0.lock().unwrap();
        state.fail_reads = false;
        if apply_pending && let Some(copy) = state.pending.take() {
            state.copy = copy;
        }
    }

    pub fn policy(&self, body: &str) {
        let directory = self.workspace.join(".akra");
        isolation::prepare_directory(&directory);
        std::fs::write(directory.join("issue-policy.toml"), body).unwrap();
    }

    pub async fn start(&self) -> Session {
        self.start_with_journal(build_issue_publish_journal()).await
    }

    pub async fn start_with_journal(
        &self,
        journal: Arc<dyn crate::application::port::outbound::issue_publish_intent_journal_port::IssuePublishIntentJournalPort>,
    ) -> Session {
        self.start_with_boundaries(self.github.clone(), self.github.clone(), journal)
            .await
    }

    pub async fn start_with_boundaries(
        &self,
        snapshot: Arc<dyn IssueSnapshotPort>,
        update: Arc<dyn IssueUpdatePort>,
        journal: Arc<dyn crate::application::port::outbound::issue_publish_intent_journal_port::IssuePublishIntentJournalPort>,
    ) -> Session {
        let publisher = IssueJournaledPublishService::new(
            snapshot.clone(),
            update,
            journal,
            self.sensitive.clone(),
        )
        .with_feedback_store(crate::composition::issue_collection::build_feedback_store());
        let drafts = IssueDraftService::new(
            IssuePolicyService::new(Arc::new(FilesystemIssuePolicySourceAdapter::new())),
            IssueTemplateService::new(Arc::new(FilesystemIssueTemplateSourceAdapter::new())),
            snapshot,
            publisher,
            Arc::new(SandboxedIssueCodeValidatorRunnerAdapter::new()),
            Arc::new(EvidenceVaultUnlockService::new(self.sensitive.clone())),
        )
        .with_automation_policy(Arc::new(IssueAutomationPolicyService::new(Arc::new(
            FilesystemIssueAutomationPolicySourceAdapter::new(),
        ))));
        let services = GatewayServices {
            issue_drafts: Some(Arc::new(drafts)),
            ..Default::default()
        };
        let (client_side, gateway_side) = tokio::io::duplex(DUPLEX_BUFFER_BYTES);
        let driver = tokio::spawn(async move {
            let _ = run_session(
                gateway_side,
                GatewayHosts::new(EchoUpstream::new(true), Vec::new()),
                Arc::new(
                    crate::composition::repository_kernel::RepositoryKernelRegistry::default(),
                ),
                services,
                None,
            )
            .await;
        });
        let (websocket, _) = tokio_tungstenite::client_async_with_config(
            "ws://localhost/",
            client_side,
            Some(websocket_config()),
        )
        .await
        .expect("Gateway handshake");
        Session {
            websocket,
            driver,
            next_id: 0,
        }
    }
}

pub(super) struct Session {
    websocket: WebSocketStream<DuplexStream>,
    driver: JoinHandle<()>,
    next_id: usize,
}

impl Session {
    pub async fn call(&mut self, method: &str, params: Value) -> Value {
        self.next_id += 1;
        kernel_request(
            &mut self.websocket,
            &format!("shared-{}", self.next_id),
            method,
            params,
        )
        .await
    }

    pub async fn close(mut self) {
        let _ = self.websocket.close(None).await;
        self.driver.abort();
        let _ = (&mut self.driver).await;
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        self.driver.abort();
    }
}

pub(super) fn public_text() -> Value {
    json!({
        "title": "Synthetic public issue title",
        "body": "Synthetic public issue body",
        "spans": [],
    })
}
