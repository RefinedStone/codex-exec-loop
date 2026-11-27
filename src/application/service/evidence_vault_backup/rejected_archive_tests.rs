use super::*;
use crate::adapter::outbound::filesystem::evidence_vault_backup::FilesystemEvidenceVaultBackupAdapter;
use crate::adapter::outbound::filesystem::evidence_vault_store::FilesystemEvidenceVaultStoreAdapter;
use crate::adapter::outbound::vault_key_store::FakeVaultKeyStoreAdapter;

#[test]
fn a_rejected_archive_does_not_initialize_a_key_on_a_fresh_device() {
    let key_store = Arc::new(FakeVaultKeyStoreAdapter::new());
    let root = crate::test_utils::platform_safe_temp_dir()
        .join(format!("rejected-vault-{}", rand::random::<u128>()));
    let service = EvidenceVaultBackupService::new(
        Arc::new(FilesystemEvidenceVaultStoreAdapter::new(root.clone())),
        Arc::new(FilesystemEvidenceVaultBackupAdapter::new(root)),
        Arc::new(EvidenceVaultUnlockService::new(key_store.clone())),
    );
    let result = service.import(VaultBackupApprovedImport {
        archive: "not an archive".to_owned(),
        recovery_key: VaultRecoveryKey::new(vec![0; VAULT_RECOVERY_KEY_LEN]),
        decision: VaultUserDecision::Confirmed,
        recovery_mode: VaultBackupRecoveryMode::PreserveLocal,
        projects: Vec::new(),
    });
    assert_eq!(
        result,
        Err(VaultBackupFailure::Backup(VaultBackupError::CorruptArchive))
    );
    assert!(key_store.stored_key().is_none());
}
