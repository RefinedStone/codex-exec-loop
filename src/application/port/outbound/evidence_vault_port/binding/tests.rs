use super::*;
use crate::application::port::outbound::evidence_vault_port::VaultKey;
use crate::domain::issue_work::{IssueProjectScope, IssueReference};

fn key() -> VaultRecordKey {
    VaultRecordKey {
        key_id: "synthetic-binding-key".to_string(),
        key: VaultKey::new(vec![7; 32]),
    }
}

fn request() -> VaultBindingCommitRequest {
    VaultBindingCommitRequest {
        expected: VaultBindingToken::observed(
            VaultRecordScope::current_version(IssueReference {
                provider: IssueProvider::GitHub,
                server_id: "github.com".to_string(),
                repository_id: "R_binding".to_string(),
                issue_id: "I_binding".to_string(),
                project_scope: IssueProjectScope {
                    workspace_dir: "/synthetic/binding".to_string(),
                },
            }),
            Some([3; 32]),
        ),
        operation_id: "synthetic-operation".to_string(),
        base_version: format!("1:{}", "a".repeat(64)),
        approval_digest: [4; 32],
        fields: VaultBindingFields {
            title: Some(VaultPlaintext::new("synthetic-title-secret".to_string())),
            body: Some(VaultPlaintext::new("synthetic-body-secret".to_string())),
        },
        policy: VaultBindingCommitPolicy::ReplaceExpected,
    }
}

#[test]
fn generation_binds_the_entire_operation_and_each_field_presence() {
    let original = request();
    let expected = original.generation_version(&key()).unwrap();
    assert_eq!(
        expected,
        original.clone().generation_version(&key()).unwrap()
    );
    assert!(!expected.contains("synthetic-title-secret"));
    assert!(!expected.contains("synthetic-body-secret"));

    let mut variants = Vec::new();
    let mut changed = original.clone();
    changed
        .expected
        .scope
        .issue
        .project_scope
        .workspace_dir
        .push('x');
    variants.push(changed);
    let mut changed = original.clone();
    changed.expected.scope.issue.provider = IssueProvider::Jira;
    variants.push(changed);
    let mut changed = original.clone();
    changed.expected.scope.issue.server_id.push('x');
    variants.push(changed);
    let mut changed = original.clone();
    changed.expected.scope.issue.repository_id.push('x');
    variants.push(changed);
    let mut changed = original.clone();
    changed.expected.scope.issue.issue_id.push('x');
    variants.push(changed);
    let mut changed = original.clone();
    changed.expected.ciphertext_sha256 = None;
    variants.push(changed);
    let mut changed = original.clone();
    changed.expected.ciphertext_sha256 = Some([5; 32]);
    variants.push(changed);
    let mut changed = original.clone();
    changed.operation_id.push('x');
    variants.push(changed);
    let mut changed = original.clone();
    changed.base_version.push('x');
    variants.push(changed);
    let mut changed = original.clone();
    changed.approval_digest[0] ^= 1;
    variants.push(changed);
    let mut changed = original.clone();
    changed.policy = VaultBindingCommitPolicy::InstallIfMissing;
    variants.push(changed);
    let mut changed = original.clone();
    changed.fields.title = None;
    variants.push(changed);
    let mut changed = original.clone();
    changed.fields.title = Some(VaultPlaintext::new(String::new()));
    variants.push(changed);
    let mut changed = original.clone();
    changed.fields.body = None;
    variants.push(changed);
    let mut changed = original.clone();
    changed.fields.body = Some(VaultPlaintext::new("changed".to_string()));
    variants.push(changed);
    let names: std::collections::HashSet<_> = variants
        .iter()
        .map(|value| value.generation_version(&key()).unwrap())
        .collect();
    assert_eq!(names.len(), variants.len());
    assert!(!names.contains(&expected));
}

#[test]
fn generation_binds_key_material_and_identifier_separately() {
    let request = request();
    let first = request.generation_version(&key()).unwrap();
    let mut other_key = key();
    other_key.key = VaultKey::new(vec![8; 32]);
    assert_ne!(first, request.generation_version(&other_key).unwrap());
    other_key = key();
    other_key.key_id.push('x');
    assert_ne!(first, request.generation_version(&other_key).unwrap());
}

#[test]
fn generation_framing_does_not_conflate_adjacent_scope_parts() {
    let mut first = request();
    first.expected.scope.issue.server_id = "ab".to_string();
    first.expected.scope.issue.repository_id = "c".to_string();
    let mut second = first.clone();
    second.expected.scope.issue.server_id = "a".to_string();
    second.expected.scope.issue.repository_id = "bc".to_string();
    assert_ne!(
        first.generation_version(&key()).unwrap(),
        second.generation_version(&key()).unwrap()
    );
}

#[test]
fn generation_flattens_only_one_valid_display_suffix_and_keeps_source_identity() {
    let original = request();
    let first = original.generation_version(&key()).unwrap();
    let mut retry_from_generation = original.clone();
    retry_from_generation.base_version = first.clone();
    let second = retry_from_generation.generation_version(&key()).unwrap();
    assert_ne!(first, second);
    assert_eq!(second.matches(":binding:").count(), 1);
    assert!(second.starts_with(&format!("{}:binding:", original.base_version)));
}

#[test]
fn generation_rejects_malformed_caller_key_and_noncanonical_scope() {
    let mut invalid_key = key();
    invalid_key.key = VaultKey::new(vec![7; 16]);
    assert_eq!(
        request().generation_version(&invalid_key),
        Err(VaultStoreError::InvalidKey)
    );
    let mut value = request();
    value.expected.scope.field = VaultRecordField::Title;
    assert!(value.generation_version(&key()).is_err());
    value = request();
    value.operation_id.clear();
    assert!(value.generation_version(&key()).is_err());
}

/// A Serialize implementation would make inference ambiguous and fail compilation.
trait NotSerialize<A> {
    fn check() {}
}
impl<T: ?Sized> NotSerialize<()> for T {}
impl<T: ?Sized + serde::Serialize> NotSerialize<u8> for T {}

#[test]
fn binding_plaintext_and_tokens_have_no_json_transport() {
    let _ = <VaultBindingFields as NotSerialize<_>>::check;
    let _ = <VaultBindingToken as NotSerialize<_>>::check;
    let _ = <VaultBindingSnapshot as NotSerialize<_>>::check;
    let _ = <VaultBindingCommitRequest as NotSerialize<_>>::check;
    let _ = <VaultBindingReceipt as NotSerialize<_>>::check;
    let _ = <VaultBindingCommitResult as NotSerialize<_>>::check;
}
