//! 일반 Gateway와 분리된 로컬 열람·표식 재연결을 조립한다. 키나 디렉터리를 만들지 않는다.
use crate::adapter::outbound::filesystem::evidence_vault_store::FilesystemEvidenceVaultStoreAdapter;
use crate::adapter::outbound::filesystem::vault_restore_workspace::VaultRestoreWorkspaceAdapter;
use crate::adapter::outbound::github::GithubIssueSnapshotAdapter;
use crate::adapter::outbound::native_vault_confirmation::NativeVaultConfirmationAdapter;
use crate::adapter::outbound::vault_key_store::OsVaultKeyStoreAdapter;
use crate::application::port::inbound::vault_restore_session_port::{
    MAX_VIEWING_SESSION, VaultRestoreSessionPort,
};
use crate::application::port::outbound::issue_snapshot_port::IssueSnapshotPort;
use crate::application::service::desktop_vault_restore::{
    DesktopVaultRestoreDependencies, DesktopVaultRestoreService,
};
use crate::application::service::evidence_vault_restore::EvidenceVaultRestoreService;
use crate::application::service::evidence_vault_unlock::EvidenceVaultUnlockService;
use anyhow::Result;
use std::sync::Arc;

pub(crate) fn build() -> Result<Arc<dyn VaultRestoreSessionPort>> {
    build_in(crate::configuration::akra_home_dir()?)
}
fn build_in(home: std::path::PathBuf) -> Result<Arc<dyn VaultRestoreSessionPort>> {
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
    Ok(Arc::new(DesktopVaultRestoreService::new(
        DesktopVaultRestoreDependencies {
            supported: cfg!(any(target_os = "linux", target_os = "macos", windows)),
            snapshots,
            unlock,
            restore,
            confirmation: Arc::new(NativeVaultConfirmationAdapter),
            workspace: Arc::new(VaultRestoreWorkspaceAdapter),
        },
        MAX_VIEWING_SESSION,
    )))
}

#[cfg(test)]
mod tests {
    #[test]
    fn assembling_read_only_restore_does_not_create_runtime_or_vault_directories() {
        let absent_home = crate::test_utils::platform_safe_temp_dir().join(format!(
            "akra-restore-builder-{}-{}",
            std::process::id(),
            rand::random::<u64>(),
        ));
        assert!(!absent_home.exists());
        let _session = super::build_in(absent_home.clone()).unwrap();
        assert!(!absent_home.exists());
    }
}

#[cfg(windows)]
pub(crate) fn native_confirmation_dialog(
    request: &crate::application::port::outbound::vault_native_confirmation_port::VaultNativeConfirmationRequest,
) -> crate::application::port::outbound::vault_native_confirmation_port::VaultNativeConfirmationOutcome{
    crate::adapter::outbound::native_vault_confirmation::windows::show_dialog(request)
}

#[cfg(all(windows, test))]
mod windows_tests;
