//! Canonical creation and legacy linked-worktree records share remote ownership.
use std::sync::atomic::Ordering;

use super::support::{
    CreateBehavior, Fixture, RAW_FIRST, RAW_SECOND, SealedRecord, intent, issue, key, request,
    shared_copy, target,
};
use crate::application::port::outbound::evidence_vault_port::{
    VaultPlaintext, VaultRecordBinding, VaultRecordField, VaultRecordScope,
};
use crate::domain::issue_creation::{IssueCreationIntentRecord, IssueCreationIntentState};
use crate::domain::issue_publish_intent::{IssuePublishIntentRecord, IssuePublishIntentState};
use crate::domain::issue_work::IssueFieldHash;

fn awaiting_adoption() -> (Fixture, IssueCreationIntentRecord) {
    let fixture = Fixture::new(CreateBehavior::Answer);
    let opened = fixture.service().open(&target()).unwrap();
    fixture.vault.fail_regular.store(true, Ordering::SeqCst);
    let confirmed = intent(
        fixture
            .service()
            .publish(request(&opened.intent_id, Some(RAW_FIRST)))
            .unwrap(),
    );
    fixture.vault.fail_regular.store(false, Ordering::SeqCst);
    assert_eq!(confirmed.state, IssueCreationIntentState::Confirmed);
    assert!(!confirmed.restore_link_finalized);
    assert!(fixture.publish_journal.records.lock().unwrap().is_empty());
    (fixture, confirmed)
}

fn later_linked_publish(sequence: u64) -> IssuePublishIntentRecord {
    let mut linked_issue = issue();
    linked_issue.project_scope.workspace_dir = "/work/project-linked".to_string();
    let hash = IssueFieldHash::of_fields(&shared_copy().title, &shared_copy().body);
    IssuePublishIntentRecord {
        intent_id: "later-linked-update-with-identical-shared-copy".to_string(),
        issue: linked_issue,
        previous_hash: hash.clone(),
        approved_hash: hash.clone(),
        pending_vault_fields: vec![],
        state: IssuePublishIntentState::Confirmed,
        observed_hash: Some(hash),
        restore_link_finalized: true,
        write_response_mismatch: false,
        write_attempted: Some(true),
        sequence,
    }
}

fn store_linked_publish(fixture: &Fixture, later: IssuePublishIntentRecord) {
    let version = later.confirmed_version();
    let mut records = fixture.vault.records.lock().unwrap();
    records.insert(
        VaultRecordScope {
            issue: later.issue.clone(),
            field: VaultRecordField::Body,
            binding: VaultRecordBinding::ConfirmedVersion(version.clone()),
        },
        SealedRecord {
            key: key(),
            plaintext: VaultPlaintext::new(RAW_SECOND.to_string()),
        },
    );
    records.insert(
        VaultRecordScope::current_version(later.issue.clone()),
        SealedRecord {
            key: key(),
            plaintext: VaultPlaintext::new(version),
        },
    );
    fixture.publish_journal.records.lock().unwrap().push(later);
}

#[test]
fn a_publish_from_another_workspace_scope_blocks_late_creation_adoption() {
    let (fixture, creation) = awaiting_adoption();
    store_linked_publish(&fixture, later_linked_publish(1));
    let before = fixture.vault.records.lock().unwrap().clone();

    let recovered = fixture
        .service()
        .recheck(&target(), &creation.intent_id, Some(key()))
        .unwrap();

    assert_eq!(recovered.state, IssueCreationIntentState::Confirmed);
    assert!(!recovered.restore_link_finalized);
    assert_eq!(fixture.creates(), 1);
    assert_eq!(fixture.publish_journal.records.lock().unwrap().len(), 1);
    assert!(fixture.current().is_none());
    assert_eq!(
        *fixture.vault.records.lock().unwrap(),
        before,
        "a canonical retry must not repoint or migrate the linked scope's ciphertext"
    );
}

#[test]
fn a_later_publish_from_another_scope_blocks_finalizing_an_existing_adoption() {
    let (fixture, creation) = awaiting_adoption();
    let publisher = fixture.publisher();
    let adopted = publisher
        .adopt_created_issue(&creation, Some(key()))
        .unwrap();
    assert!(!adopted.restore_link_finalized);
    store_linked_publish(&fixture, later_linked_publish(adopted.sequence + 1));
    let before = fixture.vault.records.lock().unwrap().clone();

    assert!(
        publisher
            .finalize_restore_link(
                &creation.repository.project_scope.workspace_dir,
                &adopted.intent_id,
                key(),
            )
            .is_err()
    );
    let recovered = fixture
        .service()
        .recheck(&target(), &creation.intent_id, Some(key()))
        .unwrap();

    assert_eq!(recovered.state, IssueCreationIntentState::Confirmed);
    assert!(!recovered.restore_link_finalized);
    assert_eq!(fixture.creates(), 1);
    assert!(fixture.current().is_none());
    assert_eq!(*fixture.vault.records.lock().unwrap(), before);
}
