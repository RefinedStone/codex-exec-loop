use super::*;
use crate::adapter::outbound::filesystem::evidence_vault_backup::FilesystemEvidenceVaultBackupAdapter;
use crate::adapter::outbound::filesystem::evidence_vault_store::FilesystemEvidenceVaultStoreAdapter;
use crate::adapter::outbound::filesystem::issue_publish_intent_journal::IssuePublishIntentJournalAdapter;
use crate::adapter::outbound::filesystem::vault_fs;
use crate::application::port::outbound::evidence_vault_backup_port::{
    EvidenceVaultBackupPort, VaultBackupError, VaultBackupExportRequest, VaultBackupImportRequest,
    VaultBackupProjectApproval, VaultBackupRecoveryMode, VaultRecoveryKey,
};
use crate::application::port::outbound::evidence_vault_port::{
    VaultBindingCommitPolicy, VaultBindingCommitRequest, VaultBindingCommitResult,
    VaultBindingFields, VaultBindingState, VaultKey, VaultPlaintext, VaultRecordBinding,
    VaultRecordField, VaultRecordKey, VaultRecordSaveRequest, VaultStoreError,
};

#[test]
fn typed_delete_is_scoped_and_retries_after_interruption_without_allowing_backup_restore() {
    let root = vault_fs::DisposableVault::new();
    let vault_root = root.0.join("vault");
    vault_fs::prepare_writer_root(&vault_root).unwrap();
    let runtime = root.0.join("runtime");
    vault_fs::prepare_writer_root(&runtime).unwrap();
    let journal = IssuePublishIntentJournalAdapter::new(move |_| Ok(runtime.clone()));
    let vault = FilesystemEvidenceVaultStoreAdapter::new(vault_root.clone());
    let backup = FilesystemEvidenceVaultBackupAdapter::new(vault_root.clone());
    let key = VaultRecordKey {
        key_id: "fixture-key".to_owned(),
        key: VaultKey::new(vec![19; 32]),
    };
    let project = IssueProjectScope {
        workspace_dir: "/fixture/workspace".to_owned(),
    };
    let request = |issue: &str| VaultRecordLoadRequest {
        scope: VaultRecordScope::current_version(IssueReference {
            provider: IssueProvider::GitHub,
            server_id: "github.com".to_owned(),
            repository_id: "R_fixture".to_owned(),
            issue_id: issue.to_owned(),
            project_scope: project.clone(),
        }),
        key: key.clone(),
    };
    let first = request("I_first");
    let second = request("I_second");
    let install = |request: &VaultRecordLoadRequest, value: &str| {
        let binding = vault.begin_binding(request.clone()).unwrap();
        let expected = binding.snapshot().token.clone();
        let result = binding
            .commit(VaultBindingCommitRequest {
                expected,
                operation_id: value.to_owned(),
                base_version: "fixture-revision".to_owned(),
                approval_digest: [1; 32],
                fields: VaultBindingFields {
                    title: None,
                    body: Some(VaultPlaintext::new(value.to_owned())),
                },
                policy: VaultBindingCommitPolicy::InstallIfMissing,
            })
            .unwrap();
        let VaultBindingCommitResult::Committed(receipt) = result else {
            panic!("{result:?}")
        };
        receipt.version
    };
    let one = install(&first, "one");
    let other = install(&second, "two");
    let scope = |request: &VaultRecordLoadRequest, version: &str, field| VaultRecordScope {
        issue: request.scope.issue.clone(),
        binding: VaultRecordBinding::ConfirmedVersion(version.to_owned()),
        field,
    };
    let legacy = scope(&first, "legacy-unindexed", VaultRecordField::Body);
    vault
        .seed_record_for_test(VaultRecordSaveRequest {
            scope: legacy,
            key: key.clone(),
            plaintext: VaultPlaintext::new("legacy".to_owned()),
        })
        .unwrap();
    let recovery_key = VaultRecoveryKey::new(vec![21; 32]);
    let archive = backup
        .export(VaultBackupExportRequest {
            scopes: vec![
                first.scope.clone(),
                scope(&first, &one, VaultRecordField::Body),
                scope(&first, &one, VaultRecordField::Title),
            ],
            key: key.clone(),
            recovery_key: recovery_key.clone(),
        })
        .unwrap();
    let deletion = |vault: &FilesystemEvidenceVaultStoreAdapter, mut input: &[u8]| {
        let mut stdout = Vec::new();
        let status = execute(
            (vault, &journal),
            Execution {
                request: first.clone(),
                action: VaultLifecycleAction::DeleteIssue,
            },
            (&mut input, &mut stdout),
        );
        (status, String::from_utf8(stdout).unwrap())
    };
    assert_eq!(deletion(&vault, b"DELETE I_second\n").0.unwrap(), 1);
    assert!(matches!(
        vault.begin_binding(first.clone()).unwrap().snapshot().state,
        VaultBindingState::Readable { .. }
    ));

    vault_fs::install_after_atomic_replace_error_hook("fixture lost tombstone acknowledgement");
    let (interrupted, _) = deletion(&vault, b"DELETE I_first\n");
    assert_eq!(
        interrupted.unwrap_err().downcast_ref::<VaultStoreError>(),
        Some(&VaultStoreError::WriteOutcomeUnknown)
    );
    let reopened = FilesystemEvidenceVaultStoreAdapter::new(vault_root);
    assert_eq!(
        reopened
            .begin_binding(first.clone())
            .unwrap()
            .snapshot()
            .state,
        VaultBindingState::Deleted
    );
    assert_eq!(
        backup.import(VaultBackupImportRequest {
            archive,
            recovery_key,
            local_key: key.clone(),
            projects: vec![VaultBackupProjectApproval {
                source: project.clone(),
                local: project
            }],
            recovery_mode: VaultBackupRecoveryMode::RecoverUnreadable,
        }),
        Err(VaultBackupError::Record(VaultStoreError::IssueDeleted))
    );

    let (status, output) = deletion(&reopened, b"DELETE I_first\n");
    assert_eq!(status.unwrap(), 2);
    assert!(output.contains("removed: 2; retained: 0; cleanup pending: 0"));
    assert!(output.contains("not verified or erased"));
    let (status, output) = deletion(&reopened, b"DELETE I_first\n");
    assert_eq!(status.unwrap(), 2);
    assert!(output.contains("removed: 0; retained: 0; cleanup pending: 0"));
    let mut binding = reopened.begin_binding(second).unwrap();
    assert_eq!(
        binding
            .load_version_fields(&other)
            .unwrap()
            .body
            .unwrap()
            .expose(),
        "two"
    );
}
