use std::collections::BTreeMap;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use super::*;
use crate::application::port::outbound::evidence_vault_port::{
    EvidenceVaultStorePort, VaultKey, VaultRecordSaveRequest,
};
use crate::domain::issue_work::{IssueProjectScope, IssueProvider, IssueReference};

mod processes;
mod safety;

struct Fixture {
    root: PathBuf,
}

impl Fixture {
    fn new(label: &str) -> Self {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "akra-binding-{label}-{}-{unique}",
            std::process::id()
        ));
        fs::create_dir_all(&root).unwrap();
        fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).unwrap();
        Self {
            root: fs::canonicalize(root).unwrap(),
        }
    }

    fn store(&self) -> FilesystemEvidenceVaultStoreAdapter {
        FilesystemEvidenceVaultStoreAdapter::new(self.root.clone())
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn key() -> VaultRecordKey {
    VaultRecordKey {
        key_id: "synthetic-binding-key".to_string(),
        key: VaultKey::new(vec![7; 32]),
    }
}

fn issue() -> IssueReference {
    IssueReference {
        provider: IssueProvider::GitHub,
        server_id: "github.com".to_string(),
        repository_id: "R_binding".to_string(),
        issue_id: "I_binding".to_string(),
        project_scope: IssueProjectScope {
            workspace_dir: "/synthetic/binding".to_string(),
        },
    }
}

fn load_request() -> VaultRecordLoadRequest {
    VaultRecordLoadRequest {
        scope: VaultRecordScope::current_version(issue()),
        key: key(),
    }
}

fn base_version() -> String {
    format!("1:{}", "a".repeat(64))
}

fn fields(title: Option<&str>, body: Option<&str>) -> VaultBindingFields {
    VaultBindingFields {
        title: title.map(|value| VaultPlaintext::new(value.to_string())),
        body: body.map(|value| VaultPlaintext::new(value.to_string())),
    }
}

fn old_fields() -> VaultBindingFields {
    fields(Some("synthetic-old-title"), Some("synthetic-old-body"))
}

fn desired_fields(label: &str) -> VaultBindingFields {
    fields(
        Some(&format!("synthetic-{label}-title")),
        Some(&format!("synthetic-{label}-body")),
    )
}

fn request(
    expected: VaultBindingToken,
    operation: &str,
    fields: VaultBindingFields,
) -> VaultBindingCommitRequest {
    VaultBindingCommitRequest {
        expected,
        operation_id: operation.to_string(),
        base_version: base_version(),
        approval_digest: [5; 32],
        fields,
        policy: VaultBindingCommitPolicy::ReplaceExpected,
    }
}

fn field_scope(version: &str, field: VaultRecordField) -> VaultRecordScope {
    VaultRecordScope {
        issue: issue(),
        field,
        binding: VaultRecordBinding::ConfirmedVersion(version.to_string()),
    }
}

fn record_path(root: &Path, scope: &VaultRecordScope) -> PathBuf {
    root.join(record_relative_path(&scope_bytes(scope)))
}

fn pointer_path(root: &Path) -> PathBuf {
    record_path(root, &load_request().scope)
}

fn seed_fields(
    store: &FilesystemEvidenceVaultStoreAdapter,
    version: &str,
    fields: &VaultBindingFields,
) {
    for (field, plaintext) in [
        (VaultRecordField::Title, &fields.title),
        (VaultRecordField::Body, &fields.body),
    ] {
        if let Some(plaintext) = plaintext {
            store
                .seed_record_for_test(VaultRecordSaveRequest {
                    scope: field_scope(version, field),
                    key: key(),
                    plaintext: plaintext.clone(),
                })
                .unwrap();
        }
    }
}

fn seed_legacy(store: &FilesystemEvidenceVaultStoreAdapter) -> VaultBindingToken {
    seed_fields(store, &base_version(), &old_fields());
    store
        .seed_record_for_test(VaultRecordSaveRequest {
            scope: load_request().scope,
            key: key(),
            plaintext: VaultPlaintext::new(base_version()),
        })
        .unwrap();
    snapshot(store).token
}

fn snapshot(store: &FilesystemEvidenceVaultStoreAdapter) -> VaultBindingSnapshot {
    store
        .begin_binding(load_request())
        .unwrap()
        .snapshot()
        .clone()
}

fn current_fields(store: &FilesystemEvidenceVaultStoreAdapter) -> VaultBindingFields {
    let mut tx = store.begin_binding(load_request()).unwrap();
    let VaultBindingState::Readable { version } = tx.snapshot().state.clone() else {
        panic!("readable binding expected")
    };
    tx.load_version_fields(&version).unwrap()
}

fn all_records(root: &Path) -> BTreeMap<String, Vec<u8>> {
    let Ok(entries) = fs::read_dir(root.join("records")) else {
        return BTreeMap::new();
    };
    entries
        .map(|entry| {
            let entry = entry.unwrap();
            (
                entry.file_name().to_string_lossy().into_owned(),
                fs::read(entry.path()).unwrap(),
            )
        })
        .collect()
}

#[test]
fn legacy_records_survive_new_generations_and_exact_retry_after_restart() {
    let fixture = Fixture::new("restart");
    let store = fixture.store();
    let expected = seed_legacy(&store);
    let operation = request(expected.clone(), "first", desired_fields("first"));
    let result = store
        .begin_binding(load_request())
        .unwrap()
        .commit(operation.clone())
        .unwrap();
    let VaultBindingCommitResult::Committed(receipt) = result else {
        panic!("commit expected")
    };
    assert_ne!(receipt.token, expected);
    assert_eq!(receipt.version.matches(":binding:").count(), 1);
    assert_eq!(current_fields(&store), operation.fields);
    let mut tx = store.begin_binding(load_request()).unwrap();
    assert_eq!(
        tx.load_version_fields(&base_version()).unwrap(),
        old_fields()
    );
    drop(tx);

    let before = all_records(&fixture.root);
    let restarted = fixture.store();
    assert_eq!(
        restarted
            .begin_binding(load_request())
            .unwrap()
            .commit(operation)
            .unwrap(),
        VaultBindingCommitResult::AlreadyCommitted(receipt)
    );
    assert_eq!(all_records(&fixture.root), before);
}

#[test]
fn same_review_different_mapping_conflicts_without_any_new_write() {
    let fixture = Fixture::new("different-mapping");
    let store = fixture.store();
    let expected = seed_legacy(&store);
    let first = request(
        expected.clone(),
        "approval",
        fields(Some("title-only"), None),
    );
    let mut second = first.clone();
    second.fields = fields(None, Some("body-only"));
    assert!(matches!(
        store
            .begin_binding(load_request())
            .unwrap()
            .commit(first.clone())
            .unwrap(),
        VaultBindingCommitResult::Committed(_)
    ));
    let before = all_records(&fixture.root);
    assert_eq!(
        store
            .begin_binding(load_request())
            .unwrap()
            .commit(second)
            .unwrap(),
        VaultBindingCommitResult::Conflict
    );
    assert_eq!(all_records(&fixture.root), before);
    assert_eq!(current_fields(&store), first.fields);
}

#[test]
fn immutable_generation_collision_is_rejected_before_other_fields_are_staged() {
    let fixture = Fixture::new("immutable");
    let store = fixture.store();
    let expected = seed_legacy(&store);
    let operation = request(expected, "immutable", desired_fields("expected"));
    let version = operation.generation_version(&key()).unwrap();
    seed_fields(
        &store,
        &version,
        &fields(Some("different-existing-title"), None),
    );
    let before = all_records(&fixture.root);
    assert_eq!(
        store
            .begin_binding(load_request())
            .unwrap()
            .commit(operation)
            .unwrap(),
        VaultBindingCommitResult::Conflict
    );
    assert_eq!(all_records(&fixture.root), before);
    assert_eq!(current_fields(&store), old_fields());
}

#[test]
fn staging_sync_failures_keep_the_old_pointer_and_retry_completes_partial_generation() {
    for stage in [VaultRecordField::Title, VaultRecordField::Body] {
        let fixture = Fixture::new("staging-error");
        let store = fixture.store();
        let expected = seed_legacy(&store);
        let operation = request(expected, "stage-retry", desired_fields("next"));
        if stage == VaultRecordField::Body {
            let version = operation.generation_version(&key()).unwrap();
            seed_fields(
                &store,
                &version,
                &VaultBindingFields {
                    title: operation.fields.title.clone(),
                    body: None,
                },
            );
        }
        let before_pointer = fs::read(pointer_path(&fixture.root)).unwrap();
        secure_fs::install_after_atomic_replace_error_hook("synthetic staged record sync failure");
        let result = store
            .begin_binding(load_request())
            .unwrap()
            .commit(operation.clone());
        assert!(
            matches!(result, Err(VaultStoreError::Storage(_))),
            "{result:?}"
        );
        assert_eq!(
            fs::read(pointer_path(&fixture.root)).unwrap(),
            before_pointer
        );
        assert_eq!(current_fields(&store), old_fields());
        assert!(matches!(
            store
                .begin_binding(load_request())
                .unwrap()
                .commit(operation.clone())
                .unwrap(),
            VaultBindingCommitResult::Committed(_)
        ));
        assert_eq!(current_fields(&store), operation.fields);
    }
}

#[test]
fn pointer_sync_failure_is_unknown_and_the_original_attempt_is_retryable() {
    let fixture = Fixture::new("pointer-error");
    let store = fixture.store();
    let expected = seed_legacy(&store);
    let operation = request(expected, "pointer-retry", desired_fields("next"));
    test_hooks::install_before_pointer(|| {
        secure_fs::install_after_atomic_replace_error_hook("synthetic pointer sync failure");
    });
    assert_eq!(
        store
            .begin_binding(load_request())
            .unwrap()
            .commit(operation.clone()),
        Err(VaultStoreError::WriteOutcomeUnknown)
    );
    assert_eq!(current_fields(&store), operation.fields);
    let before = all_records(&fixture.root);
    assert!(matches!(
        store
            .begin_binding(load_request())
            .unwrap()
            .commit(operation)
            .unwrap(),
        VaultBindingCommitResult::AlreadyCommitted(_)
    ));
    assert_eq!(all_records(&fixture.root), before);
}

#[test]
fn ciphertext_cas_keeps_a_noncooperating_pointer_change() {
    let fixture = Fixture::new("external-cas");
    let store = fixture.store();
    let expected = seed_legacy(&store);
    let operation = request(expected, "candidate", desired_fields("candidate"));
    let external_version = "1:synthetic-external-version";
    seed_fields(&store, external_version, &desired_fields("external"));
    let external = seal_record(
        &key(),
        &scope_bytes(&load_request().scope),
        &VaultPlaintext::new(external_version.to_string()),
    )
    .unwrap();
    let path = pointer_path(&fixture.root);
    let external_copy = external.clone();
    test_hooks::install_before_pointer(move || {
        secure_fs::install_before_atomic_replace_hook(move || {
            fs::write(&path, &external_copy).unwrap();
        });
    });
    assert_eq!(
        store
            .begin_binding(load_request())
            .unwrap()
            .commit(operation)
            .unwrap(),
        VaultBindingCommitResult::Conflict
    );
    assert_eq!(fs::read(pointer_path(&fixture.root)).unwrap(), external);
    assert_eq!(current_fields(&store), desired_fields("external"));
}

#[test]
fn unreadable_recovery_is_explicit_and_does_not_replace_a_readable_binding() {
    let fixture = Fixture::new("recover");
    let store = fixture.store();
    seed_legacy(&store);
    fs::write(pointer_path(&fixture.root), b"{synthetic-corrupt").unwrap();
    let observed = snapshot(&store);
    assert!(matches!(
        observed.state,
        VaultBindingState::Unreadable {
            error: VaultStoreError::CorruptRecord,
        }
    ));
    let mut operation = request(observed.token, "recover", desired_fields("recovered"));
    for policy in [
        VaultBindingCommitPolicy::ReplaceExpected,
        VaultBindingCommitPolicy::InstallIfMissing,
    ] {
        operation.policy = policy;
        assert_eq!(
            store
                .begin_binding(load_request())
                .unwrap()
                .commit(operation.clone())
                .unwrap(),
            VaultBindingCommitResult::Conflict
        );
    }
    operation.policy = VaultBindingCommitPolicy::RecoverUnreadable;
    assert!(matches!(
        store
            .begin_binding(load_request())
            .unwrap()
            .commit(operation.clone())
            .unwrap(),
        VaultBindingCommitResult::Committed(_)
    ));
    assert!(matches!(
        store
            .begin_binding(load_request())
            .unwrap()
            .commit(operation.clone())
            .unwrap(),
        VaultBindingCommitResult::AlreadyCommitted(_)
    ));
    operation.expected = snapshot(&store).token;
    operation.operation_id = "must-preserve-readable".to_string();
    let before = all_records(&fixture.root);
    assert_eq!(
        store
            .begin_binding(load_request())
            .unwrap()
            .commit(operation)
            .unwrap(),
        VaultBindingCommitResult::Conflict
    );
    assert_eq!(all_records(&fixture.root), before);
}

#[test]
fn ordinary_save_cannot_bypass_binding_transactions_but_staging_does_not_relock() {
    let fixture = Fixture::new("save-boundary");
    let store = fixture.store();
    for scope in [
        load_request().scope,
        field_scope(&base_version(), VaultRecordField::Title),
    ] {
        assert!(matches!(
            store.save(VaultRecordSaveRequest {
                scope,
                key: key(),
                plaintext: VaultPlaintext::new("must-not-write".to_string()),
            }),
            Err(VaultStoreError::Storage(_))
        ));
    }
    assert!(all_records(&fixture.root).is_empty());
    let tx = store.begin_binding(load_request()).unwrap();
    store
        .save(VaultRecordSaveRequest {
            scope: VaultRecordScope {
                issue: issue(),
                field: VaultRecordField::Title,
                binding: VaultRecordBinding::PublishIntent("safe-staging".to_string()),
            },
            key: key(),
            plaintext: VaultPlaintext::new("synthetic-staging".to_string()),
        })
        .unwrap();
    assert!(matches!(tx.snapshot().state, VaultBindingState::Missing));
}

#[test]
fn missing_tokens_are_bound_to_the_full_issue_and_cannot_be_cross_applied() {
    let fixture = Fixture::new("scope-token");
    let store = fixture.store();
    let first = snapshot(&store).token;
    let mut other = load_request();
    other
        .scope
        .issue
        .project_scope
        .workspace_dir
        .push_str("-other");
    let tx = store.begin_binding(other).unwrap();
    assert_eq!(
        tx.commit(request(first, "other", desired_fields("other")))
            .unwrap(),
        VaultBindingCommitResult::Conflict
    );
    assert!(all_records(&fixture.root).is_empty());
}
