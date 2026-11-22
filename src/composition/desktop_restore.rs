//! 일반 Gateway와 분리된 로컬 열람만 조립한다. 키나 디렉터리를 만들지 않는다.
use anyhow::{Result, bail};
use std::path::Path;
use std::sync::Arc;

use crate::adapter::outbound::db::sqlite_planning_authority_adapter::SqlitePlanningAuthorityAdapter;
use crate::adapter::outbound::filesystem::evidence_vault_store::FilesystemEvidenceVaultStoreAdapter;
use crate::adapter::outbound::github::GithubIssueSnapshotAdapter;
use crate::adapter::outbound::native_vault_confirmation::NativeVaultConfirmationAdapter;
use crate::adapter::outbound::vault_key_store::OsVaultKeyStoreAdapter;
use crate::application::port::outbound::issue_snapshot_port::IssueSnapshotPort;
use crate::application::port::outbound::vault_native_confirmation_port::VaultNativeConfirmationPort;
use crate::application::service::evidence_vault_restore::EvidenceVaultRestoreService;
use crate::application::service::evidence_vault_unlock::EvidenceVaultUnlockService;

pub(crate) type WorkspaceResolver = Arc<dyn Fn(&str) -> Result<String> + Send + Sync>;
pub(crate) struct DesktopRestoreServices {
    pub supported: bool,
    pub snapshots: Arc<dyn IssueSnapshotPort>,
    pub unlock: Arc<EvidenceVaultUnlockService>,
    pub restore: Arc<EvidenceVaultRestoreService>,
    pub confirmation: Arc<dyn VaultNativeConfirmationPort>,
    pub workspace: WorkspaceResolver,
}
pub(crate) fn build() -> Result<DesktopRestoreServices> {
    build_in(crate::configuration::akra_home_dir()?)
}

fn build_in(home: std::path::PathBuf) -> Result<DesktopRestoreServices> {
    let snapshots: Arc<dyn IssueSnapshotPort> =
        Arc::new(GithubIssueSnapshotAdapter::new(home.clone()));
    let store = Arc::new(FilesystemEvidenceVaultStoreAdapter::new(
        home.join("evidence-vault"),
    ));
    let unlock = Arc::new(EvidenceVaultUnlockService::new(Arc::new(
        OsVaultKeyStoreAdapter::new(),
    )));
    let restore = Arc::new(EvidenceVaultRestoreService::new(
        snapshots.clone(),
        store,
        unlock.clone(),
    ));
    Ok(DesktopRestoreServices {
        supported: cfg!(any(target_os = "linux", target_os = "macos")),
        snapshots,
        unlock,
        restore,
        confirmation: Arc::new(NativeVaultConfirmationAdapter),
        workspace: Arc::new(canonical_workspace),
    })
}
fn canonical_workspace(workspace: &str) -> Result<String> {
    let path = Path::new(workspace);
    if !path.is_absolute() {
        bail!("invalid restore workspace");
    }
    let canonical = std::fs::canonicalize(path)?;
    if !canonical.is_dir() {
        bail!("invalid restore workspace");
    }
    let Some(primary) = SqlitePlanningAuthorityAdapter::resolve_primary_checkout_root(
        canonical
            .to_str()
            .ok_or_else(|| anyhow::anyhow!("invalid restore workspace"))?,
    )?
    else {
        bail!("restore workspace has no primary checkout");
    };
    let primary = std::fs::canonicalize(primary)?;
    if primary != canonical {
        bail!("restore workspace is not the primary checkout");
    }
    primary
        .to_str()
        .map(str::to_string)
        .ok_or_else(|| anyhow::anyhow!("invalid restore workspace"))
}

#[cfg(test)]
mod tests {
    #[test]
    fn assembling_read_only_restore_does_not_create_runtime_or_vault_directories() {
        let absent_home = crate::test_utils::platform_safe_temp_dir().join(format!(
            "akra-restore-builder-{}-{}",
            std::process::id(),
            rand::random::<u64>()
        ));
        assert!(!absent_home.exists());
        let _services = super::build_in(absent_home.clone()).unwrap();
        assert!(!absent_home.exists());
    }
}
