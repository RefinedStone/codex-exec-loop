//! Creation must cross the remote-write boundary at most once per durable intent.
//! These tests use only in-memory ports and synthetic issue/vault values.
mod concurrency;
mod handoff;
mod scope;
mod support;

use std::sync::atomic::Ordering;
use std::sync::{Arc, Barrier};

use super::{IssueCreationApprovedRequest, IssueCreationOutcome};
use crate::application::port::inbound::issue_draft_port::IssueCreationRestartRequest;
use crate::application::port::outbound::evidence_vault_port::{
    VaultKey, VaultPlaintext, VaultRecordBinding, VaultRecordField, VaultRecordScope,
};
use crate::domain::issue_creation::IssueCreationIntentState;
use crate::domain::issue_publish_intent::{IssuePublishIntentRecord, IssuePublishIntentState};
use crate::domain::issue_work::IssueFieldHash;
use support::{
    CREATED_URL, CreateBehavior, Fixture, RAW_FIRST, RAW_SECOND, SealedRecord, WORKSPACE, intent,
    issue, key, repository, request, shared_copy, snapshot, target,
};

#[test]
fn two_windows_and_a_restarted_service_reuse_the_same_prepared_intent() {
    let fixture = Fixture::new(CreateBehavior::Answer);
    let barrier = Arc::new(Barrier::new(3));
    let records = std::thread::scope(|scope| {
        let jobs: Vec<_> = (0..2)
            .map(|_| {
                let service = fixture.service();
                let barrier = barrier.clone();
                scope.spawn(move || {
                    barrier.wait();
                    service.open(&target()).unwrap()
                })
            })
            .collect();
        barrier.wait();
        jobs.into_iter()
            .map(|job| job.join().unwrap())
            .collect::<Vec<_>>()
    });
    assert_eq!(records[0].intent_id, records[1].intent_id);
    assert_eq!(records[0].state, IssueCreationIntentState::Prepared);
    let mut alias = target();
    alias.cwd = format!("{WORKSPACE}/.");
    assert_eq!(
        fixture.service().open(&alias).unwrap().intent_id,
        records[0].intent_id
    );
    assert_eq!(fixture.journal.records.lock().unwrap().len(), 1);
    assert_eq!(fixture.creates(), 0);
}

#[test]
fn approval_binds_repository_intent_copy_policy_and_template_before_any_write() {
    let mutations: [fn(&mut IssueCreationApprovedRequest); 5] = [
        |request| request.approval.repository.repository_id = "R_OTHER".to_string(),
        |request| request.approval.intent_id = "another-intent".to_string(),
        |request| request.approval.shared_copy_hash = IssueFieldHash::of_fields("other", "copy"),
        |request| request.approval.policy_version = "other-policy".to_string(),
        |request| request.approval.template_version = None,
    ];
    for change in mutations {
        let fixture = Fixture::new(CreateBehavior::Answer);
        let opened = fixture.service().open(&target()).unwrap();
        let mut publish = request(&opened.intent_id, Some(RAW_FIRST));
        change(&mut publish);

        assert!(matches!(
            fixture.service().publish(publish).unwrap(),
            IssueCreationOutcome::Refused(_)
        ));

        assert_eq!(fixture.creates(), 0);
        assert_eq!(fixture.vault.creation_attempts.load(Ordering::SeqCst), 0);
        assert!(fixture.publish_journal.records.lock().unwrap().is_empty());
    }
}

#[test]
fn creation_stages_and_records_before_post_then_adopts_the_real_issue_identity() {
    let fixture = Fixture::new(CreateBehavior::Answer);
    let opened = fixture.service().open(&target()).unwrap();
    let journal = fixture.journal.clone();
    let vault = fixture.vault.clone();
    let expected_id = opened.intent_id.clone();
    *fixture.remote.before_create.lock().unwrap() = Some(Arc::new(move || {
        assert_eq!(
            journal.history.lock().unwrap().last(),
            Some(&IssueCreationIntentState::Sent)
        );
        let staged = vault.creation.lock().unwrap();
        assert_eq!(staged.len(), 1);
        let (scope, _) = staged.iter().next().unwrap();
        assert_eq!(scope.repository, repository());
        assert_eq!(scope.intent_id, expected_id);
        assert_eq!(
            scope.approved_hash,
            IssueFieldHash::of_fields(&shared_copy().title, &shared_copy().body)
        );
        assert!(
            vault.records.lock().unwrap().is_empty(),
            "an issue ID does not exist before POST"
        );
    }));

    let result = intent(
        fixture
            .service()
            .publish(request(&opened.intent_id, Some(RAW_FIRST)))
            .unwrap(),
    );

    assert_eq!(result.state, IssueCreationIntentState::Confirmed);
    assert!(result.restore_link_finalized);
    assert_eq!(result.created.as_ref().unwrap().issue, issue());
    assert_eq!(fixture.creates(), 1);
    assert_eq!(fixture.remote.updates.load(Ordering::SeqCst), 0);
    assert_eq!(
        fixture.remote.posts.lock().unwrap()[0].shared_copy,
        shared_copy()
    );
    let published = fixture.publish_journal.records.lock().unwrap()[0].clone();
    assert_eq!(published.intent_id, result.adoption_intent_id());
    let current_version = fixture.current().expect("the adopted binding is current");
    assert!(
        crate::domain::issue_publish_intent::confirmed_version_matches_hash(
            &current_version,
            &published.approved_hash,
        )
    );
    let records = fixture.vault.records.lock().unwrap();
    assert!(records.keys().all(|scope| scope.issue == issue()));
    assert_eq!(
        records
            .get(&VaultRecordScope {
                issue: issue(),
                field: VaultRecordField::Body,
                binding: VaultRecordBinding::ConfirmedVersion(current_version),
            })
            .unwrap()
            .plaintext
            .expose(),
        RAW_FIRST
    );
    let stored = serde_json::to_string(&*fixture.journal.records.lock().unwrap()).unwrap();
    assert!(
        !stored.contains(RAW_FIRST),
        "raw draft text must not enter the intent journal"
    );
}

#[test]
fn simultaneous_windows_send_only_one_post_for_the_same_intent() {
    let fixture = Fixture::new(CreateBehavior::Answer);
    let opened = fixture.service().open(&target()).unwrap();
    let barrier = Arc::new(Barrier::new(3));
    std::thread::scope(|scope| {
        let jobs: Vec<_> = (0..2)
            .map(|_| {
                let service = fixture.service();
                let barrier = barrier.clone();
                let id = opened.intent_id.clone();
                scope.spawn(move || {
                    barrier.wait();
                    intent(service.publish(request(&id, None)).unwrap())
                })
            })
            .collect();
        barrier.wait();
        for job in jobs {
            assert_eq!(
                job.join().unwrap().state,
                IssueCreationIntentState::Confirmed
            );
        }
    });
    assert_eq!(fixture.creates(), 1);
    assert_eq!(fixture.remote.updates.load(Ordering::SeqCst), 0);
}

#[test]
fn repeated_publish_after_restart_never_reposts_or_replaces_staged_originals() {
    let fixture = Fixture::new(CreateBehavior::Answer);
    let opened = fixture.service().open(&target()).unwrap();
    let first = intent(
        fixture
            .service()
            .publish(request(&opened.intent_id, Some(RAW_FIRST)))
            .unwrap(),
    );
    let staged = fixture.staged();
    let current = fixture.current();

    for _ in 0..2 {
        let repeated = intent(
            fixture
                .service()
                .publish(request(&opened.intent_id, Some(RAW_SECOND)))
                .unwrap(),
        );
        assert_eq!(repeated.intent_id, first.intent_id);
        assert_eq!(repeated.state, IssueCreationIntentState::Confirmed);
    }

    assert_eq!(fixture.creates(), 1);
    assert_eq!(fixture.staged(), staged);
    assert_eq!(fixture.current(), current);
    assert_eq!(fixture.vault.creation_attempts.load(Ordering::SeqCst), 1);
}

#[test]
fn a_lost_answer_never_triggers_automatic_search_or_another_post() {
    let fixture = Fixture::new(CreateBehavior::ApplyAndLoseAnswer);
    let opened = fixture.service().open(&target()).unwrap();
    let pending = intent(
        fixture
            .service()
            .publish(request(&opened.intent_id, Some(RAW_FIRST)))
            .unwrap(),
    );
    assert_eq!(pending.state, IssueCreationIntentState::NeedsConfirmation);
    assert!(pending.created.is_none());
    let staged = fixture.staged();

    for _ in 0..2 {
        assert_eq!(
            fixture.service().open(&target()).unwrap().intent_id,
            opened.intent_id
        );
        fixture
            .service()
            .recheck(&target(), &opened.intent_id, Some(key()))
            .unwrap();
        fixture
            .service()
            .publish(request(&opened.intent_id, Some(RAW_SECOND)))
            .unwrap();
    }

    assert_eq!(fixture.creates(), 1);
    assert_eq!(fixture.remote.reads.load(Ordering::SeqCst), 0);
    assert_eq!(fixture.remote.resolves.load(Ordering::SeqCst), 0);
    assert_eq!(fixture.staged(), staged);
}

#[test]
fn an_explicit_url_recovers_a_lost_answer_without_repeating_creation() {
    let fixture = Fixture::new(CreateBehavior::ApplyAndLoseAnswer);
    let opened = fixture.service().open(&target()).unwrap();
    fixture
        .service()
        .publish(request(&opened.intent_id, Some(RAW_FIRST)))
        .unwrap();

    let confirmed = fixture
        .service()
        .confirm(&target(), &opened.intent_id, CREATED_URL, Some(key()))
        .unwrap();

    assert_eq!(confirmed.state, IssueCreationIntentState::Confirmed);
    assert!(confirmed.restore_link_finalized);
    assert_eq!(confirmed.created.unwrap().issue, issue());
    assert_eq!(fixture.creates(), 1);
    assert_eq!(fixture.remote.resolves.load(Ordering::SeqCst), 1);
    assert!(fixture.current().is_some());
}

#[test]
fn failed_readback_keeps_the_candidate_and_recovers_with_read_only_recheck() {
    let fixture = Fixture::new(CreateBehavior::Answer);
    let opened = fixture.service().open(&target()).unwrap();
    fixture.remote.fail_reads.store(true, Ordering::SeqCst);
    let pending = intent(
        fixture
            .service()
            .publish(request(&opened.intent_id, Some(RAW_FIRST)))
            .unwrap(),
    );
    assert_eq!(pending.state, IssueCreationIntentState::NeedsConfirmation);
    assert_eq!(pending.created.as_ref().unwrap().issue, issue());

    fixture.remote.fail_reads.store(false, Ordering::SeqCst);
    let confirmed = fixture
        .service()
        .recheck(&target(), &opened.intent_id, Some(key()))
        .unwrap();

    assert_eq!(confirmed.state, IssueCreationIntentState::Confirmed);
    assert!(confirmed.restore_link_finalized);
    assert_eq!(fixture.creates(), 1);
    assert_eq!(fixture.remote.updates.load(Ordering::SeqCst), 0);
}

#[test]
fn remote_failure_or_contradictory_creation_answers_never_allow_an_automatic_repost() {
    for behavior in [
        CreateBehavior::FailWithoutApply,
        CreateBehavior::WrongRepositoryAnswer,
        CreateBehavior::WrongCopyAnswer,
    ] {
        let fixture = Fixture::new(behavior);
        let opened = fixture.service().open(&target()).unwrap();
        let first = intent(
            fixture
                .service()
                .publish(request(&opened.intent_id, None))
                .unwrap(),
        );
        assert_ne!(
            first.state,
            IssueCreationIntentState::Confirmed,
            "{behavior:?}"
        );
        let again = intent(
            fixture
                .service()
                .publish(request(&opened.intent_id, None))
                .unwrap(),
        );
        assert_ne!(
            again.state,
            IssueCreationIntentState::Confirmed,
            "{behavior:?}"
        );
        let checked = fixture
            .service()
            .recheck(&target(), &opened.intent_id, None)
            .unwrap();
        assert_ne!(
            checked.state,
            IssueCreationIntentState::Confirmed,
            "{behavior:?}"
        );
        assert_eq!(fixture.creates(), 1, "{behavior:?}");
        assert!(fixture.publish_journal.records.lock().unwrap().is_empty());
    }
}

#[test]
fn a_recorded_but_unsent_intent_cannot_adopt_an_existing_url() {
    let fixture = Fixture::new(CreateBehavior::Answer);
    let opened = fixture.service().open(&target()).unwrap();
    fixture
        .remote
        .snapshots
        .lock()
        .unwrap()
        .insert(7, snapshot(issue(), shared_copy()));
    *fixture.journal.fail_before.lock().unwrap() = Some(IssueCreationIntentState::Sent);

    assert!(
        fixture
            .service()
            .publish(request(&opened.intent_id, Some(RAW_FIRST)))
            .is_err()
    );
    assert_eq!(
        fixture
            .service()
            .intent(&target(), &opened.intent_id)
            .unwrap()
            .state,
        IssueCreationIntentState::Recorded
    );
    assert!(
        fixture
            .service()
            .confirm(&target(), &opened.intent_id, CREATED_URL, Some(key()))
            .is_err()
    );

    assert_eq!(fixture.creates(), 0);
    assert_eq!(fixture.remote.resolves.load(Ordering::SeqCst), 0);
    assert!(fixture.publish_journal.records.lock().unwrap().is_empty());
}

#[test]
fn held_intents_reject_url_confirmation_and_do_not_restage_changed_raw_text() {
    let fixture = Fixture::new(CreateBehavior::Answer);
    let opened = fixture.service().open(&target()).unwrap();
    fixture.vault.fail_creation.store(true, Ordering::SeqCst);
    let held = intent(
        fixture
            .service()
            .publish(request(&opened.intent_id, Some(RAW_FIRST)))
            .unwrap(),
    );
    assert_eq!(held.state, IssueCreationIntentState::Held);
    let attempts = fixture.vault.creation_attempts.load(Ordering::SeqCst);
    let staged = fixture.staged();
    fixture
        .remote
        .snapshots
        .lock()
        .unwrap()
        .insert(7, snapshot(issue(), shared_copy()));
    fixture.vault.fail_creation.store(false, Ordering::SeqCst);

    fixture
        .service()
        .publish(request(&opened.intent_id, Some(RAW_SECOND)))
        .unwrap();
    assert!(
        fixture
            .service()
            .confirm(&target(), &opened.intent_id, CREATED_URL, Some(key()))
            .is_err()
    );

    assert_eq!(fixture.creates(), 0);
    assert_eq!(
        fixture.vault.creation_attempts.load(Ordering::SeqCst),
        attempts
    );
    assert_eq!(fixture.staged(), staged);
    assert_eq!(
        fixture
            .service()
            .intent(&target(), &opened.intent_id)
            .unwrap()
            .state,
        IssueCreationIntentState::Held
    );
}

#[test]
fn explicitly_choosing_shared_copy_only_can_resume_a_held_intent_without_staging_raw_text() {
    let fixture = Fixture::new(CreateBehavior::Answer);
    let opened = fixture.service().open(&target()).unwrap();
    fixture.vault.fail_creation.store(true, Ordering::SeqCst);
    fixture
        .service()
        .publish(request(&opened.intent_id, Some(RAW_FIRST)))
        .unwrap();
    let attempts = fixture.vault.creation_attempts.load(Ordering::SeqCst);
    let mut shared_only = request(&opened.intent_id, Some(RAW_SECOND));
    shared_only.shared_copy_only = true;

    let confirmed = intent(fixture.service().publish(shared_only).unwrap());

    assert_eq!(confirmed.state, IssueCreationIntentState::Confirmed);
    assert!(confirmed.shared_copy_only);
    assert!(confirmed.pending_vault_fields.is_empty());
    assert_eq!(fixture.creates(), 1);
    assert_eq!(
        fixture.vault.creation_attempts.load(Ordering::SeqCst),
        attempts
    );
    assert!(
        fixture
            .vault
            .records
            .lock()
            .unwrap()
            .values()
            .all(|record| record.plaintext.expose() != RAW_FIRST
                && record.plaintext.expose() != RAW_SECOND)
    );
}

#[test]
fn confirmation_rejects_foreign_repository_wrong_stable_id_and_changed_shared_copy() {
    for case in 0..4 {
        let fixture = Fixture::new(CreateBehavior::Answer);
        let opened = fixture.service().open(&target()).unwrap();
        fixture.remote.fail_reads.store(true, Ordering::SeqCst);
        fixture
            .service()
            .publish(request(&opened.intent_id, Some(RAW_FIRST)))
            .unwrap();
        fixture.remote.fail_reads.store(false, Ordering::SeqCst);
        let url = match case {
            0 => "https://untrusted.example/acme/widget/issues/7",
            1 => "https://github.com/other/widget/issues/7",
            2 => {
                let mut other = issue();
                other.issue_id = "I_OTHER".to_string();
                fixture
                    .remote
                    .snapshots
                    .lock()
                    .unwrap()
                    .insert(8, snapshot(other, shared_copy()));
                "https://github.com/acme/widget/issues/8"
            }
            _ => {
                let mut changed = shared_copy();
                changed.body = "Modified after creation".to_string();
                fixture
                    .remote
                    .snapshots
                    .lock()
                    .unwrap()
                    .insert(7, snapshot(issue(), changed));
                CREATED_URL
            }
        };

        assert!(
            fixture
                .service()
                .confirm(&target(), &opened.intent_id, url, Some(key()))
                .is_err(),
            "case {case}"
        );

        assert_eq!(fixture.creates(), 1);
        assert!(fixture.publish_journal.records.lock().unwrap().is_empty());
        assert!(fixture.current().is_none());
    }
}

#[test]
fn a_committed_sent_state_with_a_lost_journal_acknowledgement_never_reposts() {
    let fixture = Fixture::new(CreateBehavior::Answer);
    let opened = fixture.service().open(&target()).unwrap();
    *fixture.journal.fail_after.lock().unwrap() = Some(IssueCreationIntentState::Sent);

    assert!(
        fixture
            .service()
            .publish(request(&opened.intent_id, Some(RAW_FIRST)))
            .is_err()
    );
    assert_eq!(
        fixture
            .service()
            .intent(&target(), &opened.intent_id)
            .unwrap()
            .state,
        IssueCreationIntentState::Sent
    );
    assert_eq!(fixture.creates(), 0);
    fixture
        .service()
        .publish(request(&opened.intent_id, Some(RAW_SECOND)))
        .unwrap();
    fixture
        .service()
        .recheck(&target(), &opened.intent_id, Some(key()))
        .unwrap();

    assert_eq!(fixture.creates(), 0);
    assert_eq!(fixture.vault.creation_attempts.load(Ordering::SeqCst), 1);
}

#[test]
fn confirmed_creation_recovers_vault_adoption_with_the_original_key_and_without_reposting() {
    let fixture = Fixture::new(CreateBehavior::Answer);
    let opened = fixture.service().open(&target()).unwrap();
    fixture.vault.fail_regular.store(true, Ordering::SeqCst);
    let confirmed = intent(
        fixture
            .service()
            .publish(request(&opened.intent_id, Some(RAW_FIRST)))
            .unwrap(),
    );
    assert_eq!(confirmed.state, IssueCreationIntentState::Confirmed);
    assert!(!confirmed.restore_link_finalized);
    fixture.vault.fail_regular.store(false, Ordering::SeqCst);
    let mut wrong_key = key();
    wrong_key.key = VaultKey::new(vec![9; 32]);

    let still_unbound = fixture
        .service()
        .recheck(&target(), &opened.intent_id, Some(wrong_key))
        .unwrap();
    assert!(!still_unbound.restore_link_finalized);
    let recovered = fixture
        .service()
        .confirm(&target(), &opened.intent_id, CREATED_URL, Some(key()))
        .unwrap();

    assert!(recovered.restore_link_finalized);
    assert_eq!(fixture.creates(), 1);
    assert_eq!(fixture.vault.creation_attempts.load(Ordering::SeqCst), 1);
}

#[test]
fn failed_adoption_journal_save_recovers_without_creating_another_issue() {
    let fixture = Fixture::new(CreateBehavior::Answer);
    let opened = fixture.service().open(&target()).unwrap();
    fixture
        .publish_journal
        .fail_save
        .store(true, Ordering::SeqCst);
    let confirmed = intent(
        fixture
            .service()
            .publish(request(&opened.intent_id, Some(RAW_FIRST)))
            .unwrap(),
    );
    assert_eq!(confirmed.state, IssueCreationIntentState::Confirmed);
    assert!(!confirmed.restore_link_finalized);

    fixture
        .publish_journal
        .fail_save
        .store(false, Ordering::SeqCst);
    let recovered = fixture
        .service()
        .recheck(&target(), &opened.intent_id, Some(key()))
        .unwrap();

    assert!(recovered.restore_link_finalized);
    assert_eq!(fixture.creates(), 1);
    assert_eq!(fixture.publish_journal.records.lock().unwrap().len(), 1);
}

#[test]
fn late_adoption_cannot_overwrite_another_publish_or_current_restore_mapping() {
    for current_mapping in [false, true] {
        let fixture = Fixture::new(CreateBehavior::Answer);
        let opened = fixture.service().open(&target()).unwrap();
        fixture
            .publish_journal
            .fail_save
            .store(true, Ordering::SeqCst);
        let confirmed = intent(
            fixture
                .service()
                .publish(request(&opened.intent_id, Some(RAW_FIRST)))
                .unwrap(),
        );
        assert!(!confirmed.restore_link_finalized);
        fixture
            .publish_journal
            .fail_save
            .store(false, Ordering::SeqCst);
        let hash = IssueFieldHash::of_fields(&shared_copy().title, &shared_copy().body);
        let newer = IssuePublishIntentRecord {
            intent_id: "newer-update-with-the-same-shared-copy".to_string(),
            issue: issue(),
            previous_hash: hash.clone(),
            approved_hash: hash.clone(),
            pending_vault_fields: vec![],
            state: IssuePublishIntentState::Confirmed,
            observed_hash: Some(hash),
            restore_link_finalized: true,

            restore_binding_attempt: None,
            write_response_mismatch: false,
            write_attempted: Some(true),
            sequence: 99,
        };
        if current_mapping {
            fixture.vault.records.lock().unwrap().insert(
                VaultRecordScope::current_version(issue()),
                SealedRecord {
                    key: key(),
                    plaintext: VaultPlaintext::new(newer.confirmed_version()),
                },
            );
        } else {
            fixture.publish_journal.records.lock().unwrap().push(newer);
        }
        let before = fixture.vault.records.lock().unwrap().clone();

        let rejected = fixture
            .service()
            .recheck(&target(), &opened.intent_id, Some(key()))
            .unwrap();

        assert_eq!(rejected.state, IssueCreationIntentState::Confirmed);
        assert!(!rejected.restore_link_finalized);
        assert_eq!(*fixture.vault.records.lock().unwrap(), before);
        assert_eq!(fixture.creates(), 1);
    }
}

#[test]
fn restarting_an_uncertain_intent_requires_explicit_acknowledgement_and_a_new_approval() {
    let fixture = Fixture::new(CreateBehavior::ApplyAndLoseAnswer);
    let opened = fixture.service().open(&target()).unwrap();
    fixture
        .service()
        .publish(request(&opened.intent_id, Some(RAW_FIRST)))
        .unwrap();
    let mut restart = IssueCreationRestartRequest {
        target: target(),
        previous_intent_id: opened.intent_id.clone(),
        acknowledge_uncertain: false,
    };
    assert!(fixture.service().restart(&restart).is_err());
    assert_eq!(fixture.journal.records.lock().unwrap().len(), 1);

    restart.acknowledge_uncertain = true;
    let next = fixture.service().restart(&restart).unwrap();

    assert_ne!(next.intent_id, opened.intent_id);
    assert_eq!(next.state, IssueCreationIntentState::Prepared);
    assert_eq!(
        next.predecessor_intent_id.as_deref(),
        Some(opened.intent_id.as_str())
    );
    assert_eq!(
        fixture.service().open(&target()).unwrap().intent_id,
        next.intent_id
    );
    assert_eq!(fixture.creates(), 1);
    fixture
        .service()
        .publish(request(&opened.intent_id, Some(RAW_SECOND)))
        .unwrap();
    assert_eq!(fixture.creates(), 1);
    fixture
        .service()
        .publish(request(&next.intent_id, Some(RAW_SECOND)))
        .unwrap();
    assert_eq!(
        fixture.creates(),
        2,
        "only an explicitly new intent and approval authorize another POST"
    );
}

#[test]
fn recorded_recovery_uses_the_frozen_original_even_if_a_new_draft_masks_to_the_same_copy() {
    let fixture = Fixture::new(CreateBehavior::Answer);
    let opened = fixture.service().open(&target()).unwrap();
    *fixture.journal.fail_before.lock().unwrap() = Some(IssueCreationIntentState::Sent);
    assert!(
        fixture
            .service()
            .publish(request(&opened.intent_id, Some(RAW_FIRST)))
            .is_err()
    );
    assert_eq!(fixture.creates(), 0);
    let staged = fixture.staged();

    let confirmed = intent(
        fixture
            .service()
            .publish(request(&opened.intent_id, Some(RAW_SECOND)))
            .unwrap(),
    );

    assert_eq!(confirmed.state, IssueCreationIntentState::Confirmed);
    assert!(confirmed.restore_link_finalized);
    assert_eq!(fixture.creates(), 1);
    assert_eq!(fixture.staged(), staged);
    assert_eq!(fixture.vault.creation_attempts.load(Ordering::SeqCst), 1);
    let adopted = fixture.publish_journal.records.lock().unwrap()[0].clone();
    let current_version = fixture
        .current()
        .expect("the original staged spans remain current");
    assert!(
        crate::domain::issue_publish_intent::confirmed_version_matches_hash(
            &current_version,
            &adopted.approved_hash,
        )
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
                binding: VaultRecordBinding::ConfirmedVersion(current_version),
            })
            .unwrap()
            .plaintext
            .expose(),
        RAW_FIRST
    );
}

#[test]
fn a_confirmation_journal_failure_recovers_without_another_post_or_original_replacement() {
    for committed in [false, true] {
        let fixture = Fixture::new(CreateBehavior::Answer);
        let opened = fixture.service().open(&target()).unwrap();
        let failure = if committed {
            &fixture.journal.fail_after
        } else {
            &fixture.journal.fail_before
        };
        *failure.lock().unwrap() = Some(IssueCreationIntentState::Confirmed);

        assert!(
            fixture
                .service()
                .publish(request(&opened.intent_id, Some(RAW_FIRST)))
                .is_err()
        );
        assert_eq!(fixture.creates(), 1);
        let staged = fixture.staged();
        fixture
            .service()
            .publish(request(&opened.intent_id, Some(RAW_SECOND)))
            .unwrap();
        let recovered = fixture
            .service()
            .recheck(&target(), &opened.intent_id, Some(key()))
            .unwrap();

        assert_eq!(recovered.state, IssueCreationIntentState::Confirmed);
        assert!(recovered.restore_link_finalized);
        assert_eq!(fixture.creates(), 1);
        assert_eq!(fixture.staged(), staged);
        assert_eq!(fixture.vault.creation_attempts.load(Ordering::SeqCst), 1);
    }
}

#[test]
fn failed_restart_keeps_the_previous_uncertain_attempt_and_requires_acknowledgement_again() {
    let fixture = Fixture::new(CreateBehavior::ApplyAndLoseAnswer);
    let opened = fixture.service().open(&target()).unwrap();
    fixture
        .service()
        .publish(request(&opened.intent_id, Some(RAW_FIRST)))
        .unwrap();
    *fixture.journal.fail_before.lock().unwrap() = Some(IssueCreationIntentState::Prepared);
    let mut restart = IssueCreationRestartRequest {
        target: target(),
        previous_intent_id: opened.intent_id.clone(),
        acknowledge_uncertain: true,
    };

    assert!(fixture.service().restart(&restart).is_err());

    let pending = fixture.service().open(&target()).unwrap();
    assert_eq!(pending.intent_id, opened.intent_id);
    assert_eq!(pending.state, IssueCreationIntentState::NeedsConfirmation);
    restart.acknowledge_uncertain = false;
    assert!(fixture.service().restart(&restart).is_err());
    fixture
        .service()
        .publish(request(&opened.intent_id, Some(RAW_SECOND)))
        .unwrap();
    assert_eq!(fixture.creates(), 1);
}
