//! Two publisher instances share the same durable ownership boundary.
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::time::Duration;

use super::support::{
    CreateBehavior, Fixture, RAW_FIRST, RAW_SECOND, WORKSPACE, intent, issue, key, request,
    shared_copy, target,
};
use crate::application::port::outbound::evidence_vault_port::{
    VaultPlaintext, VaultRecordBinding, VaultRecordField, VaultRecordScope,
};
use crate::application::service::issue_publish::IssueApprovedPublishRequest;
use crate::application::service::issue_publish_journal::{
    IssueJournaledPublishOutcome, IssueJournaledPublishRequest, IssueRestorableField,
};
use crate::domain::issue_creation::IssueCreationIntentState;
use crate::domain::issue_publish_intent::IssuePublishVaultField;
use crate::domain::issue_validation::{IssueRequirements, validate_issue};
use crate::domain::issue_work::{
    IssueFieldHash, IssuePublishApproval, IssueWriteApproval, IssueWriteOperation,
};

const WAIT: Duration = Duration::from_secs(5);

fn update_request() -> IssueJournaledPublishRequest {
    let copy = shared_copy();
    let hash = IssueFieldHash::of_fields(&copy.title, &copy.body);
    IssueJournaledPublishRequest {
        publish: IssueApprovedPublishRequest {
            issue: issue(),
            validated: validate_issue(&copy, &IssueRequirements::default()).unwrap(),
            approval: IssuePublishApproval {
                write: IssueWriteApproval {
                    issue: issue(),
                    operation: IssueWriteOperation::Body,
                },
                shared_copy_hash: hash.clone(),
                policy_version: "policy-v1".to_string(),
                template_version: Some("template-v1".to_string()),
            },
            edit_basis: hash,
            policy_version: "policy-v1".to_string(),
            template_version: Some("template-v1".to_string()),
        },
        restorable: vec![IssueRestorableField {
            field: IssuePublishVaultField::Body,
            plaintext: VaultPlaintext::new(RAW_SECOND.to_string()),
        }],
        vault_key: Some(key()),
        shared_copy_only: false,
    }
}

#[test]
fn competing_publishers_cannot_let_late_creation_adoption_replace_a_newer_original() {
    let fixture = Fixture::new(CreateBehavior::Answer);
    let opened = fixture.service().open(&target()).unwrap();
    fixture
        .publish_journal
        .fail_save
        .store(true, Ordering::SeqCst);
    let pending = intent(
        fixture
            .service()
            .publish(request(&opened.intent_id, Some(RAW_FIRST)))
            .unwrap(),
    );
    assert_eq!(pending.state, IssueCreationIntentState::Confirmed);
    assert!(!pending.restore_link_finalized);
    fixture
        .publish_journal
        .fail_save
        .store(false, Ordering::SeqCst);
    fixture.remote.allow_updates.store(true, Ordering::SeqCst);

    // Stop adoption after its ownership checks, before its first issue-scoped write.
    // A competing publisher must wait here even though it owns a separate service mutex.
    let (entered_tx, entered_rx) = mpsc::channel();
    let (resume_tx, resume_rx) = mpsc::channel();
    let resume_rx = Mutex::new(resume_rx);
    let paused = AtomicBool::new(false);
    let adoption_id = pending.adoption_intent_id();
    *fixture.vault.before_save.lock().unwrap() = Some(Arc::new(move |request| {
        if matches!(
            &request.scope.binding,
            VaultRecordBinding::PublishIntent(id) if id == &adoption_id
        ) && !paused.swap(true, Ordering::SeqCst)
        {
            entered_tx.send(()).unwrap();
            resume_rx
                .lock()
                .unwrap()
                .recv_timeout(WAIT * 3)
                .expect("the test must release the paused adoption");
        }
    }));

    let (contended_tx, contended_rx) = mpsc::channel();
    *fixture.publish_journal.on_lease_contended.lock().unwrap() = Some(Arc::new(move || {
        let _ = contended_tx.send(());
    }));

    let creation_service = fixture.service();
    let update_service = fixture.publisher();
    let id = opened.intent_id;
    let (creation, updated, saw_contention) = std::thread::scope(|scope| {
        let adopting = scope.spawn(move || creation_service.recheck(&target(), &id, Some(key())));
        let entered = entered_rx.recv_timeout(WAIT);
        let updating = scope.spawn(move || {
            let outcome = update_service.publish(update_request()).unwrap();
            let IssueJournaledPublishOutcome::Confirmed(record) = outcome else {
                panic!("the later update was not confirmed: {outcome:?}");
            };
            update_service.finalize_restore_link(WORKSPACE, &record.intent_id, key())
        });
        let saw_contention = entered.is_ok() && contended_rx.recv_timeout(WAIT).is_ok();
        // Release before any assertion so a broken lease fails instead of deadlocking.
        let _ = resume_tx.send(());
        let creation = adopting.join().unwrap();
        let updated = updating.join().unwrap();
        (creation.unwrap(), updated.unwrap(), saw_contention)
    });

    assert!(
        saw_contention,
        "independent publishers must contend on the journal lease"
    );
    assert_eq!(creation.state, IssueCreationIntentState::Confirmed);
    assert_eq!(fixture.creates(), 1);
    assert_eq!(fixture.remote.updates.load(Ordering::SeqCst), 1);
    assert_eq!(fixture.current(), Some(updated.confirmed_version()));
    let records = fixture.publish_journal.records.lock().unwrap();
    let adoption = records
        .iter()
        .find(|record| record.intent_id == creation.adoption_intent_id())
        .unwrap();
    assert!(
        updated.sequence > adoption.sequence,
        "each owner must receive a distinct order"
    );
    assert_eq!(
        fixture
            .vault
            .records
            .lock()
            .unwrap()
            .get(&VaultRecordScope {
                issue: issue(),
                field: VaultRecordField::Body,
                binding: VaultRecordBinding::ConfirmedVersion(updated.confirmed_version()),
            })
            .unwrap()
            .plaintext
            .expose(),
        RAW_SECOND,
    );
}
