use super::*;
use crate::application::port::outbound::evidence_vault_port::{
    EvidenceVaultStorePort, VaultBindingCommitPolicy, VaultBindingCommitRequest,
    VaultBindingCommitResult, VaultBindingFields, VaultBindingState, VaultKey, VaultPlaintext,
    VaultRecordLoadRequest,
};
use crate::domain::issue_work::{IssueProjectScope, IssueProvider, IssueReference};

#[test]
fn retention_removes_expired_records_but_keeps_young_current_and_reconnect_sources() {
    let root = vault_fs::DisposableVault::new();
    let store = FilesystemEvidenceVaultStoreAdapter::new(root.0.clone());
    let request = VaultRecordLoadRequest {
        scope: VaultRecordScope::current_version(IssueReference {
            provider: IssueProvider::GitHub,
            server_id: "github.com".into(),
            repository_id: "R_fixture".into(),
            issue_id: "I_fixture".into(),
            project_scope: IssueProjectScope {
                workspace_dir: "/fixture".into(),
            },
        }),
        key: VaultRecordKey {
            key_id: "fixture".into(),
            key: VaultKey::new(vec![7; 32]),
        },
    };
    let install = |operation: &str| {
        let binding = store.begin_binding(request.clone()).unwrap();
        let expected = binding.snapshot().token.clone();
        match binding
            .commit(VaultBindingCommitRequest {
                expected,
                operation_id: operation.into(),
                base_version: "synthetic-revision".into(),
                approval_digest: [0; 32],
                fields: VaultBindingFields {
                    title: None,
                    body: Some(VaultPlaintext::new(operation.into())),
                },
                policy: VaultBindingCommitPolicy::ReplaceExpected,
            })
            .unwrap()
        {
            VaultBindingCommitResult::Committed(receipt) => receipt.version,
            other => panic!("expected fixture generation: {other:?}"),
        }
    };
    let expired = install("expired");
    let pinned = install("reviewed source");
    store
        .begin_binding(request.clone())
        .unwrap()
        .retain_reconnect_source(&pinned)
        .unwrap();
    let fresh = install("recent");
    let current = install("current");
    let mut inventory = Inventory::read(&store, &request.scope).unwrap();
    for entry in &mut inventory.records {
        if matches!(&entry.kind, Kind::Generation(version) if version == &expired || version == &pinned || version == &current)
        {
            entry.retired_at = Some(0);
        }
    }
    inventory.persist(&store, &request.scope).unwrap();
    let expired_scope = VaultRecordScope {
        issue: request.scope.issue.clone(),
        field: VaultRecordField::Body,
        binding: VaultRecordBinding::ConfirmedVersion(expired.clone()),
    };
    let expired_ciphertext = std::fs::read(
        root.0
            .join(record_relative_path(&scope_bytes(&expired_scope))),
    )
    .unwrap();

    let binding = store.begin_binding(request.clone()).unwrap();
    let expected = binding.maintenance_revision().unwrap();
    let report = binding
        .maintain(
            &expected,
            &VaultCleanupPolicy {
                action: VaultLifecycleAction::RetainRecent,
                disposable_intents: Vec::new(),
            },
        )
        .unwrap();

    assert_eq!(report.removed, 1);
    assert!(
        !contains_file_bytes(&root.0, &expired_ciphertext),
        "retention must not leave a recoverable copy in the fixture Vault"
    );
    let mut binding = store.begin_binding(request).unwrap();
    assert_eq!(
        binding.snapshot().state,
        VaultBindingState::Readable {
            version: current.clone()
        }
    );
    assert!(
        binding
            .load_version_fields(&expired)
            .unwrap()
            .body
            .is_none()
    );
    for (version, expected) in [
        (pinned, "reviewed source"),
        (fresh, "recent"),
        (current, "current"),
    ] {
        assert_eq!(
            binding
                .load_version_fields(&version)
                .unwrap()
                .body
                .unwrap()
                .expose(),
            expected
        );
    }
}

fn contains_file_bytes(root: &std::path::Path, expected: &[u8]) -> bool {
    std::fs::read_dir(root).unwrap().any(|entry| {
        let entry = entry.unwrap();
        let kind = entry.file_type().unwrap();
        if kind.is_dir() {
            contains_file_bytes(&entry.path(), expected)
        } else {
            kind.is_file() && std::fs::read(entry.path()).unwrap() == expected
        }
    })
}
