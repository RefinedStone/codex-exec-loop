use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use super::*;
use crate::adapter::outbound::filesystem::evidence_vault_store::seal_record;
use crate::application::port::outbound::evidence_vault_backup_port::VaultBackupImportRequest;
use crate::application::port::outbound::evidence_vault_port::{
    VaultBindingTransaction, VaultRecordSaveRequest, VaultStoreError,
};
use crate::domain::issue_publish_intent::confirmed_version_name;
use crate::domain::issue_work::{IssueFieldHash, IssueProvider};
use crate::test_utils::vault_binding::{TestBindingHooks, begin_test_binding};

#[derive(Default)]
struct TestStore {
    records: Mutex<HashMap<VaultRecordScope, (VaultRecordKey, VaultPlaintext)>>,
    fail_pointer_once_for: Mutex<Option<String>>,
    fail_after_pointer_once: AtomicBool,
    writes: AtomicUsize,
}

impl EvidenceVaultStorePort for TestStore {
    fn save(&self, _: VaultRecordSaveRequest) -> Result<(), VaultStoreError> {
        panic!("backup import must use the binding transaction");
    }

    fn load(&self, request: VaultRecordLoadRequest) -> Result<VaultPlaintext, VaultStoreError> {
        let records = self.records.lock().unwrap();
        let (key, plaintext) = records
            .get(&request.scope)
            .ok_or(VaultStoreError::RecordNotFound)?;
        if key != &request.key {
            return Err(VaultStoreError::KeyMismatch);
        }
        Ok(plaintext.clone())
    }

    fn begin_binding(
        &self,
        request: VaultRecordLoadRequest,
    ) -> Result<Box<dyn VaultBindingTransaction + '_>, VaultStoreError> {
        begin_test_binding(
            self.records.lock().unwrap(),
            request,
            TestBindingHooks {
                before_write: Some(Box::new(|request| {
                    let mut fail = self.fail_pointer_once_for.lock().unwrap();
                    if request.scope.binding == VaultRecordBinding::CurrentVersion
                        && fail.as_deref() == Some(request.scope.issue.issue_id.as_str())
                    {
                        *fail = None;
                        return Err(VaultStoreError::Storage(
                            "pointer write interrupted".to_string(),
                        ));
                    }
                    self.writes.fetch_add(1, Ordering::SeqCst);
                    Ok(())
                })),
                after_pointer: Some(Box::new(|| {
                    if self.fail_after_pointer_once.swap(false, Ordering::SeqCst) {
                        return Err(VaultStoreError::WriteOutcomeUnknown);
                    }
                    Ok(())
                })),
            },
        )
    }
}

fn key(byte: u8) -> VaultRecordKey {
    VaultRecordKey {
        key_id: format!("fixture-key-{byte}"),
        key: VaultKey::new(vec![byte; 32]),
    }
}

fn issue(id: &str) -> IssueReference {
    IssueReference {
        provider: IssueProvider::GitHub,
        server_id: "github.com".to_string(),
        repository_id: "R_fixture".to_string(),
        issue_id: id.to_string(),
        project_scope: IssueProjectScope {
            workspace_dir: "/synthetic/backup".to_string(),
        },
    }
}

fn version() -> String {
    confirmed_version_name(&IssueFieldHash::of_fields("public title", "public body"))
}

fn staged(id: &str, body: &str) -> StagedBinding {
    StagedBinding {
        issue: issue(id),
        base_version: version(),
        fields: VaultBindingFields {
            title: None,
            body: Some(VaultPlaintext::new(body.to_string())),
        },
    }
}

fn import(
    store: &TestStore,
    mode: VaultBackupRecoveryMode,
    staged: Vec<StagedBinding>,
) -> Result<ImportOutcomes, VaultBackupError> {
    commit_import_bindings(store, &key(1), "fixture-backup", [3; 32], mode, staged)
}

#[test]
fn interrupted_fields_remain_invisible_and_retry_reuses_the_same_generation() {
    let store = TestStore::default();
    *store.fail_pointer_once_for.lock().unwrap() = Some("I_1".to_string());
    let error = import(
        &store,
        VaultBackupRecoveryMode::PreserveLocal,
        vec![staged("I_1", "original")],
    )
    .err()
    .unwrap();
    assert!(matches!(
        error,
        VaultBackupError::Record(VaultStoreError::Storage(_))
    ));
    let partial = store.records.lock().unwrap().clone();
    assert_eq!(partial.len(), 1);
    assert!(!partial.contains_key(&VaultRecordScope::current_version(issue("I_1"))));

    let report = import(
        &store,
        VaultBackupRecoveryMode::PreserveLocal,
        vec![staged("I_1", "original")],
    )
    .unwrap();
    assert_eq!(report.imported, [issue("I_1")]);
    let records = store.records.lock().unwrap();
    assert_eq!(records.len(), 2);
    for (scope, original) in partial {
        assert_eq!(records.get(&scope), Some(&original));
    }
    assert_eq!(store.writes.load(Ordering::SeqCst), 2);
}

#[test]
fn a_readable_binding_is_preserved_after_lost_ack_even_with_recovery_approval() {
    for mode in [
        VaultBackupRecoveryMode::PreserveLocal,
        VaultBackupRecoveryMode::RecoverUnreadable,
    ] {
        let store = TestStore::default();
        store.fail_after_pointer_once.store(true, Ordering::SeqCst);
        assert!(matches!(
            import(&store, mode, vec![staged("I_1", "original")]),
            Err(VaultBackupError::Record(
                VaultStoreError::WriteOutcomeUnknown
            ))
        ));
        let before = store.records.lock().unwrap().clone();
        let writes = store.writes.load(Ordering::SeqCst);
        let retry = import(
            &store,
            mode,
            vec![staged("I_1", "a different archive mapping")],
        )
        .unwrap();
        assert_eq!(retry.kept_local, [issue("I_1")]);
        assert!(retry.imported.is_empty());
        assert!(retry.recovered.is_empty());
        assert_eq!(*store.records.lock().unwrap(), before);
        assert_eq!(store.writes.load(Ordering::SeqCst), writes);
    }
}

#[test]
fn unreadable_recovery_requires_approval_and_preserves_the_new_readable_binding_on_retry() {
    let store = TestStore::default();
    let scope = VaultRecordScope::current_version(issue("I_1"));
    store
        .records
        .lock()
        .unwrap()
        .insert(scope, (key(2), VaultPlaintext::new(version())));
    let before = store.records.lock().unwrap().clone();
    assert!(matches!(
        import(
            &store,
            VaultBackupRecoveryMode::PreserveLocal,
            vec![staged("I_1", "original")]
        ),
        Err(VaultBackupError::LocalRecoveryRequired)
    ));
    assert_eq!(*store.records.lock().unwrap(), before);
    let recovered = import(
        &store,
        VaultBackupRecoveryMode::RecoverUnreadable,
        vec![staged("I_1", "original")],
    )
    .unwrap();
    assert_eq!(recovered.imported, [issue("I_1")]);
    assert_eq!(recovered.recovered, [issue("I_1")]);
    let restored = store.records.lock().unwrap().clone();
    let retry = import(
        &store,
        VaultBackupRecoveryMode::RecoverUnreadable,
        vec![staged("I_1", "original")],
    )
    .unwrap();
    assert_eq!(retry.kept_local, [issue("I_1")]);
    assert_eq!(*store.records.lock().unwrap(), restored);
}

#[test]
fn a_later_issue_failure_keeps_the_first_commit_and_retry_finishes_only_the_missing_pointer() {
    let store = TestStore::default();
    *store.fail_pointer_once_for.lock().unwrap() = Some("I_2".to_string());
    assert!(
        import(
            &store,
            VaultBackupRecoveryMode::PreserveLocal,
            vec![staged("I_1", "one"), staged("I_2", "two")],
        )
        .is_err()
    );
    let first =
        store.records.lock().unwrap()[&VaultRecordScope::current_version(issue("I_1"))].clone();
    assert!(
        !store
            .records
            .lock()
            .unwrap()
            .contains_key(&VaultRecordScope::current_version(issue("I_2")))
    );
    let report = import(
        &store,
        VaultBackupRecoveryMode::PreserveLocal,
        vec![staged("I_1", "one"), staged("I_2", "two")],
    )
    .unwrap();
    assert_eq!(report.kept_local, [issue("I_1")]);
    assert_eq!(report.imported, [issue("I_2")]);
    assert_eq!(
        store.records.lock().unwrap()[&VaultRecordScope::current_version(issue("I_1"))],
        first
    );
    assert_eq!(store.writes.load(Ordering::SeqCst), 4);
}

fn encrypted(scope: VaultRecordScope, plaintext: &str) -> StoredBackupRecord {
    let body = seal_record(
        &key(1),
        &scope_bytes(&scope),
        &VaultPlaintext::new(plaintext.to_string()),
    )
    .unwrap();
    StoredBackupRecord::new(&scope, String::from_utf8(body).unwrap())
}

fn archive(records: Vec<StoredBackupRecord>) -> StoredVaultBackup {
    StoredVaultBackup {
        format_version: ARCHIVE_FORMAT_VERSION,
        algorithm: ARCHIVE_ALGORITHM.to_string(),
        kdf: ARCHIVE_KDF.to_string(),
        backup_id: "0123456789abcdef0123456789abcdef".to_string(),
        key_id: key(1).key_id,
        wrap_nonce: String::new(),
        wrapped_key: String::new(),
        records,
    }
}

fn approval(source: &IssueReference) -> VaultBackupProjectApproval {
    VaultBackupProjectApproval {
        source: source.project_scope.clone(),
        local: IssueProjectScope {
            workspace_dir: "/synthetic/local".to_string(),
        },
    }
}

#[test]
fn archive_staging_keeps_optional_fields_and_rejects_duplicate_local_scopes() {
    let source = issue("I_1");
    let pointer = VaultRecordScope::current_version(source.clone());
    let body = VaultRecordScope {
        issue: source.clone(),
        field: VaultRecordField::Body,
        binding: VaultRecordBinding::ConfirmedVersion(version()),
    };
    let source_approval = approval(&source);
    let valid = archive(vec![
        encrypted(pointer.clone(), &version()),
        encrypted(body.clone(), ""),
    ]);
    let (staged, unapproved) =
        stage_import(&valid, &key(1), std::slice::from_ref(&source_approval)).unwrap();
    assert!(unapproved.is_empty());
    assert_eq!(staged.len(), 1);
    assert_eq!(staged[0].issue.project_scope, source_approval.local);
    assert!(staged[0].fields.title.is_none());
    assert_eq!(staged[0].fields.body.as_ref().unwrap().expose(), "");

    let duplicate = archive(vec![
        encrypted(pointer, &version()),
        encrypted(body.clone(), "one"),
        encrypted(body, "two"),
    ]);
    assert!(matches!(
        stage_import(&duplicate, &key(1), &[source_approval]),
        Err(VaultBackupError::CorruptArchive)
    ));
}

#[test]
fn archive_staging_authenticates_unapproved_records_and_rejects_orphan_versions() {
    let source = issue("I_1");
    let body = VaultRecordScope {
        issue: source.clone(),
        field: VaultRecordField::Body,
        binding: VaultRecordBinding::ConfirmedVersion(version()),
    };
    let orphan = archive(vec![encrypted(body, "original")]);
    assert!(matches!(
        stage_import(&orphan, &key(1), &[approval(&source)]),
        Err(VaultBackupError::CorruptArchive)
    ));
    let mut corrupt = encrypted(VaultRecordScope::current_version(source), &version());
    corrupt.record = "not an authenticated vault record".to_string();
    assert!(matches!(
        stage_import(&archive(vec![corrupt]), &key(1), &[]),
        Err(VaultBackupError::CorruptArchive)
    ));
}

#[test]
fn distinct_archive_projects_cannot_alias_the_same_local_binding() {
    let one = issue("I_1");
    let mut two = one.clone();
    two.project_scope.workspace_dir = "/synthetic/another-source".to_string();
    let records = archive(vec![
        encrypted(VaultRecordScope::current_version(one.clone()), &version()),
        encrypted(VaultRecordScope::current_version(two.clone()), &version()),
    ]);
    assert!(matches!(
        stage_import(&records, &key(1), &[approval(&one), approval(&two)]),
        Err(VaultBackupError::CorruptArchive)
    ));
}

fn authenticated_archive(records: Vec<StoredBackupRecord>) -> (String, VaultRecoveryKey) {
    let mut archive = archive(records);
    let recovery_key = VaultRecoveryKey::new(vec![8; 32]);
    // Synthetic fixture key/nonce only; this helper never handles live Vault data.
    let nonce = [6; NONCE_LEN];
    archive.wrap_nonce = BASE64.encode(nonce);
    let mut wrapped = key(1).key.expose().to_vec();
    wrap_key(&recovery_key, &decode_hex(&archive.backup_id).unwrap())
        .unwrap()
        .seal_in_place_append_tag(
            Nonce::assume_unique_for_key(nonce),
            Aad::from(wrap_associated_data(&archive)),
            &mut wrapped,
        )
        .unwrap();
    archive.wrapped_key = BASE64.encode(wrapped);
    (serde_json::to_string(&archive).unwrap(), recovery_key)
}

#[test]
fn a_later_empty_archive_pointer_is_rejected_before_any_local_filesystem_write() {
    let first = issue("I_1");
    let second = issue("I_2");
    let (archive, recovery_key) = authenticated_archive(vec![
        encrypted(VaultRecordScope::current_version(first.clone()), &version()),
        encrypted(
            VaultRecordScope {
                issue: first.clone(),
                field: VaultRecordField::Body,
                binding: VaultRecordBinding::ConfirmedVersion(version()),
            },
            "original",
        ),
        encrypted(VaultRecordScope::current_version(second), ""),
    ]);
    let root = crate::test_utils::platform_safe_temp_dir().join(format!(
        "akra-vault-malformed-archive-{}-{:032x}",
        std::process::id(),
        rand::random::<u128>(),
    ));
    let result =
        FilesystemEvidenceVaultBackupAdapter::new(root.clone()).import(VaultBackupImportRequest {
            archive,
            recovery_key,
            local_key: key(2),
            projects: vec![approval(&first)],
            recovery_mode: VaultBackupRecoveryMode::PreserveLocal,
        });
    let untouched = !root.exists();
    if root.exists() {
        std::fs::remove_dir_all(&root).unwrap();
    }
    assert_eq!(result, Err(VaultBackupError::CorruptArchive));
    assert!(
        untouched,
        "archive semantic validation must precede acquiring a local binding lease"
    );
}

#[cfg(any(unix, windows))]
#[test]
fn repeated_scope_selection_exports_one_record_per_scope_and_imports_successfully() {
    let root = crate::test_utils::platform_safe_temp_dir().join(format!(
        "akra-vault-repeated-export-{}-{:032x}",
        std::process::id(),
        rand::random::<u128>(),
    ));
    vault_fs::prepare_writer_root(&root.join("source")).unwrap();
    vault_fs::prepare_writer_root(&root.join("local")).unwrap();
    let root = std::fs::canonicalize(root).unwrap();
    let source_root = root.join("source");
    let local_root = root.join("local");
    let source = issue("I_1");
    let source_store = FilesystemEvidenceVaultStoreAdapter::new(source_root.clone());
    let transaction = source_store
        .begin_binding(VaultRecordLoadRequest {
            scope: VaultRecordScope::current_version(source.clone()),
            key: key(1),
        })
        .unwrap();
    let expected = transaction.snapshot().token.clone();
    let receipt = match transaction
        .commit(VaultBindingCommitRequest {
            expected,
            operation_id: "synthetic-source".to_string(),
            base_version: version(),
            approval_digest: [1; 32],
            fields: VaultBindingFields {
                title: None,
                body: Some(VaultPlaintext::new("original".to_string())),
            },
            policy: VaultBindingCommitPolicy::InstallIfMissing,
        })
        .unwrap()
    {
        VaultBindingCommitResult::Committed(receipt) => receipt,
        _ => panic!("new source binding"),
    };
    let mut scopes = vec![VaultRecordScope::current_version(source.clone())];
    for field in [VaultRecordField::Title, VaultRecordField::Body] {
        scopes.push(VaultRecordScope {
            issue: source.clone(),
            field,
            binding: VaultRecordBinding::ConfirmedVersion(receipt.version.clone()),
        });
    }
    scopes.extend(scopes.clone());
    let recovery_key = VaultRecoveryKey::new(vec![8; 32]);
    let archive = FilesystemEvidenceVaultBackupAdapter::new(source_root)
        .export(VaultBackupExportRequest {
            scopes,
            key: key(1),
            recovery_key: recovery_key.clone(),
        })
        .unwrap();
    assert_eq!(parse_archive(&archive).unwrap().records.len(), 2);
    let project = approval(&source);
    let report = FilesystemEvidenceVaultBackupAdapter::new(local_root.clone())
        .import(VaultBackupImportRequest {
            archive,
            recovery_key,
            local_key: key(2),
            projects: vec![project.clone()],
            recovery_mode: VaultBackupRecoveryMode::PreserveLocal,
        })
        .unwrap();
    let mut local_issue = source;
    local_issue.project_scope = project.local;
    assert_eq!(report.imported, [local_issue.clone()]);
    let local_store = FilesystemEvidenceVaultStoreAdapter::new(local_root);
    let mut transaction = local_store
        .begin_binding(VaultRecordLoadRequest {
            scope: VaultRecordScope::current_version(local_issue),
            key: key(2),
        })
        .unwrap();
    let VaultBindingState::Readable { version } = &transaction.snapshot().state else {
        panic!("imported binding");
    };
    let version = version.clone();
    let fields = transaction.load_version_fields(&version).unwrap();
    assert!(fields.title.is_none());
    assert_eq!(fields.body.as_ref().unwrap().expose(), "original");
    drop(transaction);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn an_archive_without_approved_projects_does_not_initialize_a_vault() {
    let source = issue("I_1");
    let (archive, recovery_key) = authenticated_archive(vec![encrypted(
        VaultRecordScope::current_version(source.clone()),
        &version(),
    )]);
    let root = crate::test_utils::platform_safe_temp_dir().join(format!(
        "akra-vault-unapproved-archive-{}-{:032x}",
        std::process::id(),
        rand::random::<u128>(),
    ));
    let store = FilesystemEvidenceVaultBackupAdapter::new(root.clone());
    let plan = store
        .prepare_import(VaultBackupSelection {
            archive: &archive,
            recovery_key: &recovery_key,
            projects: &[],
            recovery_mode: VaultBackupRecoveryMode::RecoverUnreadable,
        })
        .unwrap();
    let VaultBackupImportPlan::ReadOnly(report) = plan else {
        panic!("project preview must not ask for a destination key")
    };
    assert_eq!(report.unapproved_projects, [source.project_scope]);
    assert!(report.imported.is_empty());
    assert!(report.kept_local.is_empty());
    assert!(report.recovered.is_empty());
    assert!(!root.exists());
}

#[test]
fn a_valid_approved_archive_does_not_create_a_missing_vault_root() {
    let source = issue("I_1");
    let (archive, recovery_key) = authenticated_archive(vec![encrypted(
        VaultRecordScope::current_version(source.clone()),
        &version(),
    )]);
    let root = crate::test_utils::platform_safe_temp_dir().join(format!(
        "akra-vault-missing-root-{}-{:032x}",
        std::process::id(),
        rand::random::<u128>(),
    ));
    let result =
        FilesystemEvidenceVaultBackupAdapter::new(root.clone()).import(VaultBackupImportRequest {
            archive,
            recovery_key,
            local_key: key(2),
            projects: vec![approval(&source)],
            recovery_mode: VaultBackupRecoveryMode::PreserveLocal,
        });
    let untouched = !root.exists();
    if root.exists() {
        std::fs::remove_dir_all(&root).unwrap();
    }
    assert!(matches!(
        result,
        Err(VaultBackupError::Record(VaultStoreError::Storage(_)))
    ));
    assert!(
        untouched,
        "the host must prepare a private Vault root before an import"
    );
}

#[cfg(any(unix, windows))]
#[test]
fn real_filesystem_import_preserves_readable_bindings_and_requires_explicit_pointer_recovery() {
    let source = issue("I_recovery");
    let (archive, recovery_key) = authenticated_archive(vec![
        encrypted(
            VaultRecordScope::current_version(source.clone()),
            &version(),
        ),
        encrypted(
            VaultRecordScope {
                issue: source.clone(),
                field: VaultRecordField::Body,
                binding: VaultRecordBinding::ConfirmedVersion(version()),
            },
            "synthetic-original-from-backup",
        ),
    ]);
    let root = crate::test_utils::platform_safe_temp_dir().join(format!(
        "akra-vault-recovery-{}-{:032x}",
        std::process::id(),
        rand::random::<u128>(),
    ));
    vault_fs::prepare_writer_root(&root).unwrap();
    let root = std::fs::canonicalize(root).unwrap();
    let adapter = FilesystemEvidenceVaultBackupAdapter::new(root.clone());
    let project = approval(&source);
    let mut local = source;
    local.project_scope = project.local.clone();
    let request = |recovery_mode| VaultBackupImportRequest {
        archive: archive.clone(),
        recovery_key: recovery_key.clone(),
        local_key: key(2),
        projects: vec![project.clone()],
        recovery_mode,
    };

    let report = adapter
        .import(request(VaultBackupRecoveryMode::PreserveLocal))
        .unwrap();
    assert_eq!(report.imported, [local.clone()]);
    let pointer_scope = VaultRecordScope::current_version(local.clone());
    let pointer = record_relative_path(&scope_bytes(&pointer_scope));
    let installed = std::fs::read(root.join(&pointer)).unwrap();
    let report = adapter
        .import(request(VaultBackupRecoveryMode::RecoverUnreadable))
        .unwrap();
    assert_eq!(report.kept_local, [local.clone()]);
    assert!(report.imported.is_empty());
    assert!(report.recovered.is_empty());
    assert_eq!(std::fs::read(root.join(&pointer)).unwrap(), installed);

    let damaged = b"synthetic-damaged-pointer";
    vault_fs::write_file_atomic(&root, &pointer, damaged).unwrap();
    assert_eq!(
        adapter.import(request(VaultBackupRecoveryMode::PreserveLocal)),
        Err(VaultBackupError::LocalRecoveryRequired)
    );
    assert_eq!(std::fs::read(root.join(&pointer)).unwrap(), damaged);
    let report = adapter
        .import(request(VaultBackupRecoveryMode::RecoverUnreadable))
        .unwrap();
    assert_eq!(report.imported, [local.clone()]);
    assert_eq!(report.recovered, [local.clone()]);

    let store = FilesystemEvidenceVaultStoreAdapter::new(root.clone());
    let mut transaction = store
        .begin_binding(VaultRecordLoadRequest {
            scope: pointer_scope,
            key: key(2),
        })
        .unwrap();
    let VaultBindingState::Readable { version } = transaction.snapshot().state.clone() else {
        panic!("explicit recovery must install a readable generation")
    };
    let fields = transaction.load_version_fields(&version).unwrap();
    assert!(fields.title.is_none());
    assert_eq!(
        fields.body.unwrap().expose(),
        "synthetic-original-from-backup"
    );
    drop(transaction);

    let recovered = std::fs::read(root.join(&pointer)).unwrap();
    let report = adapter
        .import(request(VaultBackupRecoveryMode::RecoverUnreadable))
        .unwrap();
    assert_eq!(report.kept_local, [local]);
    assert!(report.recovered.is_empty());
    assert_eq!(std::fs::read(root.join(&pointer)).unwrap(), recovered);
    std::fs::remove_dir_all(root).unwrap();
}
