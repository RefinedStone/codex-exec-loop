use super::*;
use crate::adapter::outbound::filesystem::evidence_vault_store::FilesystemEvidenceVaultStoreAdapter;
use crate::adapter::outbound::filesystem::vault_restore_workspace::VaultRestoreWorkspaceAdapter;
use crate::adapter::outbound::vault_key_store::FakeVaultKeyStoreAdapter;
use crate::application::port::inbound::vault_restore_session_port::{
    VaultRestoreSessionHold, VaultRestoreSessionPort, VaultRestoreSessionPrepared,
    VaultRestoreSessionRequest,
};
use crate::application::port::outbound::issue_snapshot_port::{
    IssueRawFields, IssueSnapshotPort, IssueSnapshotReadRequest, IssueSnapshotResolveRequest,
    IssueSnapshotResponse,
};
use crate::application::port::outbound::vault_native_confirmation_port::VaultNativeConfirmationPort;
use crate::application::service::desktop_vault_restore::{
    DesktopVaultRestoreDependencies, DesktopVaultRestoreService,
};
use crate::application::service::evidence_vault_restore::EvidenceVaultRestoreService;
use crate::application::service::evidence_vault_unlock::EvidenceVaultUnlockService;
use crate::domain::issue_work::{
    IssueComparisonBasis, IssueProjectScope, IssueProvider, IssueReference,
};
use std::sync::{Mutex, atomic::AtomicUsize};

struct Snapshots {
    issue: IssueReference,
    resolves: AtomicUsize,
}

impl IssueSnapshotPort for Snapshots {
    fn resolve(
        &self,
        request: IssueSnapshotResolveRequest,
    ) -> anyhow::Result<IssueSnapshotResponse> {
        assert_eq!(request.project_scope, self.issue.project_scope);
        self.resolves.fetch_add(1, Ordering::SeqCst);
        let fields = IssueRawFields::new("synthetic title".into(), "synthetic shared body".into());
        Ok(IssueSnapshotResponse {
            issue: self.issue.clone(),
            comparison_basis: IssueComparisonBasis {
                field_hash: fields.field_hash(),
                provider_updated_at: "2026-11-25T00:00:00Z".into(),
            },
            raw_fields: fields,
        })
    }

    fn read(&self, _: IssueSnapshotReadRequest) -> anyhow::Result<IssueSnapshotResponse> {
        panic!("prepare must not start a restore");
    }
}

#[derive(Default)]
struct Confirmation(Mutex<Vec<String>>);

impl VaultNativeConfirmationPort for Confirmation {
    fn confirm(
        &self,
        request: &VaultNativeConfirmationRequest,
        _: &AtomicBool,
        _: Instant,
    ) -> Outcome {
        assert_eq!(request.purpose, Purpose::ViewOriginal);
        self.0.lock().unwrap().push(request.workspace_dir.clone());
        Outcome::Confirmed
    }
}

#[test]
fn windows_prepare_accepts_the_actual_canonical_git_scope_without_key_or_vault_access() {
    let folder = Folder::new();
    let repository = folder.0.join("한글 project");
    crate::private_directory::ensure_private_directory(&repository).unwrap();
    let git = crate::test_utils::git_command()
        .current_dir(&repository)
        .args(["-c", "core.hooksPath=", "init", "--quiet"])
        .output()
        .unwrap();
    assert!(
        git.status.success(),
        "synthetic repository initialization failed: {}",
        String::from_utf8_lossy(&git.stderr)
    );
    let canonical = fs::canonicalize(&repository).unwrap();
    let workspace = canonical.to_str().unwrap();
    assert!(workspace.starts_with(r"\\?\"));
    let issue = IssueReference {
        provider: IssueProvider::GitHub,
        server_id: "github.com".into(),
        repository_id: "R_synthetic".into(),
        issue_id: "I_synthetic".into(),
        project_scope: IssueProjectScope {
            workspace_dir: workspace.into(),
        },
    };
    let snapshots = Arc::new(Snapshots {
        issue: issue.clone(),
        resolves: AtomicUsize::new(0),
    });
    let confirmation = Arc::new(Confirmation::default());
    let keys = Arc::new(FakeVaultKeyStoreAdapter::new());
    let unlock = Arc::new(EvidenceVaultUnlockService::new(keys.clone()));
    let vault_root = folder.0.join("absent-vault");
    let restore = Arc::new(EvidenceVaultRestoreService::new(
        snapshots.clone(),
        Arc::new(FilesystemEvidenceVaultStoreAdapter::new(vault_root.clone())),
        unlock.clone(),
    ));
    let service = || {
        DesktopVaultRestoreService::new(
            DesktopVaultRestoreDependencies {
                supported: true,
                snapshots: snapshots.clone(),
                unlock: unlock.clone(),
                restore: restore.clone(),
                confirmation: confirmation.clone(),
                workspace: Arc::new(VaultRestoreWorkspaceAdapter),
            },
            Duration::from_secs(30),
        )
    };

    // Rewriting a differently spelled scope would change which encrypted record it identifies.
    let mut different_scope = issue.clone();
    different_scope.project_scope.workspace_dir = repository.to_str().unwrap().into();
    assert_ne!(different_scope.project_scope, issue.project_scope);
    assert_eq!(
        service().prepare(VaultRestoreSessionRequest {
            issue_url: "https://github.com/acme/project/issues/1".into(),
            issue: different_scope,
        }),
        Err(VaultRestoreSessionHold::NotBound)
    );
    assert_eq!(snapshots.resolves.load(Ordering::SeqCst), 0);
    assert!(confirmation.0.lock().unwrap().is_empty());

    let session = service();
    assert_eq!(
        session.prepare(VaultRestoreSessionRequest {
            issue_url: "https://github.com/acme/project/issues/1".into(),
            issue,
        }),
        Ok(VaultRestoreSessionPrepared { reconnect: true })
    );
    assert_eq!(snapshots.resolves.load(Ordering::SeqCst), 1);
    assert_eq!(*confirmation.0.lock().unwrap(), vec![workspace.to_string()]);
    assert_eq!(keys.load_count(), 0);
    assert_eq!(keys.initialization_count(), 0);
    assert!(!vault_root.exists());
    assert!(!repository.join(".git/akra-repository-id").exists());
    assert!(!repository.join(".git/akra-repository-id.lock").exists());
    session.cancel();
}
