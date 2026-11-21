use super::*;
use crate::domain::issue_automation::collection::IssueCollectionMode;

#[test]
fn feedback_semantic_revision_and_publication_count_are_independent_of_replay_and_ui_ack() {
    let fixture = Fixture::new("feedback-semantic-accounting");
    let issue = seed(&fixture, "I_1");
    let adapter = fixture.adapter();
    let saved = save_desired(&adapter, &issue, "상태: 실행 중");
    let original_item = current(&adapter, &issue).item;
    let catalog_revision = adapter
        .read_triage_batch(fixture.cwd(), None, 100)
        .unwrap()
        .revision;

    let abandoned_intent = comment_intent(&current(&adapter, &issue), "never-sent");
    let abandoned = recorded(&adapter, &abandoned_intent);
    assert_eq!(current(&adapter, &issue).publication_attempts, 0);
    adapter
        .complete(
            &issue,
            &abandoned.intent.intent_id,
            abandoned.revision,
            &IssueFeedbackCompletion::Abandoned,
        )
        .unwrap()
        .unwrap();
    assert_eq!(current(&adapter, &issue).publication_attempts, 0);

    let intent = comment_intent(&current(&adapter, &issue), "status-create");
    let record = recorded(&adapter, &intent);
    assert_eq!(current(&adapter, &issue).publication_attempts, 0);
    let sent_record = sent(&adapter, &record);
    assert_eq!(current(&adapter, &issue).publication_attempts, 1);
    let receipt = comment_receipt(&sent_record, "IC_status");
    let done = confirmed(&adapter, &sent_record, receipt.clone());
    let after_confirmation = current(&adapter, &issue);
    assert_eq!(
        after_confirmation.state.desired_revision,
        saved.desired_revision
    );
    assert_eq!(after_confirmation.publication_attempts, 1);
    assert_eq!(after_confirmation.item, original_item);
    assert_eq!(count(&fixture, "issue_work_checkpoints"), 0);
    let events = adapter.pending_events(fixture.cwd()).unwrap();
    assert_eq!(events.len(), 1);
    assert!(events[0].event_id.starts_with("issue-feedback:"));
    assert_eq!(events[0].action, IssueFeedbackAction::CommentCreated);
    assert_eq!(events[0].revision, after_confirmation.state.revision);

    // Every retry observes the immutable fact; none obtains a second send right.
    assert!(matches!(
        adapter.record(after_confirmation.fence(), &intent).unwrap(),
        IssueFeedbackRecordResult::Existing(existing) if existing == done
    ));
    assert!(matches!(
        adapter.claim_sent(&issue, &intent.intent_id, record.revision, after_confirmation.fence()).unwrap(),
        IssueFeedbackClaimResult::Existing(existing) if existing == done
    ));
    assert_eq!(
        adapter
            .complete(
                &issue,
                &intent.intent_id,
                sent_record.revision,
                &IssueFeedbackCompletion::Confirmed(Box::new(receipt))
            )
            .unwrap(),
        Some(done)
    );
    assert_eq!(
        save_desired(&adapter, &issue, "상태: 실행 중"),
        after_confirmation.state
    );
    adapter
        .acknowledge_event(fixture.cwd(), &events[0].event_id)
        .unwrap();
    adapter
        .acknowledge_event(fixture.cwd(), &events[0].event_id)
        .unwrap();
    assert_eq!(current(&adapter, &issue), after_confirmation);
    assert!(adapter.pending_events(fixture.cwd()).unwrap().is_empty());
    assert_eq!(count(&fixture, "issue_feedback_outbox"), 1);
    assert_eq!(
        adapter
            .read_triage_batch(fixture.cwd(), None, 100)
            .unwrap()
            .revision,
        catalog_revision
    );
    drop(adapter);

    let reopened = fixture.adapter();
    assert_eq!(current(&reopened, &issue), after_confirmation);
    save_desired(&reopened, &issue, "상태: 완료됨");
    let update = comment_intent(&current(&reopened, &issue), "status-update");
    assert!(
        matches!(&update.operation, IssueFeedbackOperation::UpdateComment { owner, .. }
        if owner.comment_id == "IC_status")
    );
    let update_record = sent(&reopened, &recorded(&reopened, &update));
    confirmed(
        &reopened,
        &update_record,
        comment_receipt(&update_record, "IC_status"),
    );
    let latest = current(&reopened, &issue);
    assert_eq!(latest.publication_attempts, 2);
    assert_eq!(latest.state.desired_revision, saved.desired_revision + 1);
    assert_eq!(latest.state.comment_owner.unwrap().comment_id, "IC_status");
    assert_eq!(count(&fixture, "issue_feedback_state"), 1);
    assert_eq!(count(&fixture, "issue_feedback_outbox"), 2);
}

#[test]
fn feedback_sent_readback_can_finish_after_off_hold_and_newer_desired_state() {
    let fixture = Fixture::new("feedback-readonly-after-revocation");
    let issue = seed(&fixture, "I_1");
    let adapter = fixture.adapter();
    save_desired(&adapter, &issue, "상태: 실행 중");
    let intent = comment_intent(&current(&adapter, &issue), "sent-before-revocation");
    let record = sent(&adapter, &recorded(&adapter, &intent));
    let collection = adapter.get_collection(fixture.cwd()).unwrap().unwrap();
    adapter
        .set_collection_mode(
            fixture.cwd(),
            Some(collection.epoch),
            IssueCollectionMode::Off,
        )
        .unwrap();
    let mut tracked = adapter.get_issue(fixture.cwd(), &issue).unwrap().unwrap();
    tracked.set_flag(IssueAutomationFlag::Held, true);
    super::super::super::tests::save_changed(&adapter, fixture.cwd(), &tracked);
    save_desired(&adapter, &issue, "상태: 보류됨");
    let before = current(&adapter, &issue);
    let completed = confirmed(&adapter, &record, comment_receipt(&record, "IC_original"));
    let after = current(&adapter, &issue);
    assert_eq!(after.collection, before.collection);
    assert_eq!(after.item, before.item);
    assert_eq!(after.state.desired, before.state.desired);
    assert_eq!(after.state.desired_revision, before.state.desired_revision);
    assert_eq!(
        after.state.comment_owner.as_ref().unwrap().content_sha256,
        intent.operation.comment_content_hash().unwrap()
    );
    assert_eq!(after.publication_attempts, 1);
    assert!(matches!(
        adapter.claim_sent(&issue, &intent.intent_id, record.revision, after.fence()).unwrap(),
        IssueFeedbackClaimResult::Existing(existing) if existing == completed
    ));
    let next = comment_intent(&after, "must-not-send-after-revocation");
    assert!(matches!(
        adapter.record(after.fence(), &next).unwrap(),
        IssueFeedbackRecordResult::Conflict
    ));
    assert_eq!(count(&fixture, "issue_feedback_intents"), 1);
}

#[test]
fn feedback_ack_identity_and_contradiction_cannot_be_erased_by_readback() {
    let fixture = Fixture::new("feedback-ack-identity");
    let issue = seed(&fixture, "I_1");
    let adapter = fixture.adapter();
    save_desired(&adapter, &issue, "상태: 실행 중");
    let intent = comment_intent(&current(&adapter, &issue), "known-ack");
    let record = recorded(&adapter, &intent);
    assert!(
        adapter
            .complete(
                &issue,
                &intent.intent_id,
                record.revision,
                &IssueFeedbackCompletion::Confirmed(Box::new(comment_receipt(&record, "IC_known")))
            )
            .is_err()
    );
    let record = sent(&adapter, &record);
    let ack = adapter
        .complete(
            &issue,
            &intent.intent_id,
            record.revision,
            &IssueFeedbackCompletion::NeedsConfirmation {
                contradictory: false,
                acknowledged_comment_id: Some("IC_known".into()),
            },
        )
        .unwrap()
        .unwrap();
    assert_eq!(ack.acknowledged_comment_id.as_deref(), Some("IC_known"));
    assert_eq!(
        adapter
            .complete(
                &issue,
                &intent.intent_id,
                ack.revision,
                &IssueFeedbackCompletion::NeedsConfirmation {
                    contradictory: false,
                    acknowledged_comment_id: None,
                }
            )
            .unwrap(),
        Some(ack.clone())
    );
    assert!(
        adapter
            .complete(
                &issue,
                &intent.intent_id,
                ack.revision,
                &IssueFeedbackCompletion::NeedsConfirmation {
                    contradictory: false,
                    acknowledged_comment_id: Some("IC_other".into()),
                }
            )
            .is_err()
    );
    assert!(
        adapter
            .complete(
                &issue,
                &intent.intent_id,
                ack.revision,
                &IssueFeedbackCompletion::Confirmed(Box::new(comment_receipt(&ack, "IC_other")))
            )
            .is_err()
    );
    assert!(
        adapter
            .complete(
                &issue,
                &intent.intent_id,
                record.revision,
                &IssueFeedbackCompletion::Confirmed(Box::new(comment_receipt(&ack, "IC_known")))
            )
            .unwrap()
            .is_none()
    );
    assert_eq!(adapter.pending(fixture.cwd()).unwrap(), vec![ack.clone()]);
    confirmed(&adapter, &ack, comment_receipt(&ack, "IC_known"));

    let second = seed(&fixture, "I_2");
    save_desired(&adapter, &second, "상태: 실행 중");
    let intent = comment_intent(&current(&adapter, &second), "contradictory-ack");
    let record = sent(&adapter, &recorded(&adapter, &intent));
    let contradiction = adapter
        .complete(
            &second,
            &intent.intent_id,
            record.revision,
            &IssueFeedbackCompletion::NeedsConfirmation {
                contradictory: true,
                acknowledged_comment_id: None,
            },
        )
        .unwrap()
        .unwrap();
    assert_eq!(
        adapter
            .complete(
                &second,
                &intent.intent_id,
                contradiction.revision,
                &IssueFeedbackCompletion::NeedsConfirmation {
                    contradictory: false,
                    acknowledged_comment_id: None,
                }
            )
            .unwrap(),
        Some(contradiction.clone())
    );
    assert!(
        adapter
            .complete(
                &second,
                &intent.intent_id,
                contradiction.revision,
                &IssueFeedbackCompletion::Confirmed(Box::new(comment_receipt(
                    &contradiction,
                    "IC_guess"
                )))
            )
            .is_err()
    );
    assert_eq!(adapter.unresolved(&second).unwrap(), vec![contradiction]);
    assert!(current(&adapter, &second).state.comment_owner.is_none());
}

#[test]
fn feedback_first_grant_and_terminal_label_removal_preserve_the_original_add_proof() {
    let fixture = Fixture::new("feedback-label-lifecycle");
    let issue = seed(&fixture, "I_1");
    let adapter = fixture.adapter();
    let work = existing_work(&fixture, &issue);
    let start = make_intent(
        &current(&adapter, &issue),
        "progress-add",
        IssueFeedbackOperation::AddLabel {
            label: label(),
            previous_event_id: None,
        },
    );
    let record = recorded(&adapter, &start);
    assert_eq!(
        adapter
            .mark_first_grant(
                &issue,
                &work.order.work_order_id,
                current(&adapter, &issue).fence()
            )
            .unwrap(),
        IssueFeedbackGrantResult::Conflict
    );
    let record = sent(&adapter, &record);
    let proof = label_receipt(&record, "LE_added", true);
    confirmed(&adapter, &record, proof.clone());
    assert_eq!(current(&adapter, &issue).publication_attempts, 1);
    assert_eq!(
        adapter
            .mark_first_grant(
                &issue,
                &work.order.work_order_id,
                current(&adapter, &issue).fence()
            )
            .unwrap(),
        IssueFeedbackGrantResult::Marked
    );
    let snapshot = current(&adapter, &issue);
    assert!(snapshot.first_grant_entered);
    let late_add = make_intent(
        &snapshot,
        "progress-late-add",
        IssueFeedbackOperation::AddLabel {
            label: label(),
            previous_event_id: None,
        },
    );
    assert!(matches!(
        adapter.record(snapshot.fence(), &late_add).unwrap(),
        IssueFeedbackRecordResult::Conflict
    ));
    let early_remove = make_intent(
        &snapshot,
        "progress-early-remove",
        IssueFeedbackOperation::RemoveLabel {
            label: label(),
            owned_event_id: "LE_added".into(),
        },
    );
    assert!(matches!(
        adapter.record(snapshot.fence(), &early_remove).unwrap(),
        IssueFeedbackRecordResult::Conflict
    ));

    integrate(&fixture, &work);
    let terminal = current(&adapter, &issue);
    assert_eq!(
        terminal.item.tracked.as_ref().unwrap().state,
        IssueAutomationState::Done
    );
    let remove = make_intent(
        &terminal,
        "progress-remove",
        IssueFeedbackOperation::RemoveLabel {
            label: label(),
            owned_event_id: "LE_added".into(),
        },
    );
    let record = sent(&adapter, &recorded(&adapter, &remove));
    assert_eq!(
        adapter
            .mark_first_grant(
                &issue,
                &work.order.work_order_id,
                current(&adapter, &issue).fence()
            )
            .unwrap(),
        IssueFeedbackGrantResult::AlreadyEntered
    );
    confirmed(
        &adapter,
        &record,
        label_receipt(&record, "LE_removed", false),
    );
    let removed = current(&adapter, &issue);
    assert!(removed.state.label_removal_confirmed);
    assert_eq!(removed.state.label_proof, Some(proof));
    assert_eq!(removed.item, terminal.item);
    assert_eq!(removed.publication_attempts, 2);
    assert_eq!(removed.state.desired_revision, 0);
    let duplicate = make_intent(
        &removed,
        "progress-remove-again",
        IssueFeedbackOperation::RemoveLabel {
            label: label(),
            owned_event_id: "LE_added".into(),
        },
    );
    assert!(matches!(
        adapter.record(removed.fence(), &duplicate).unwrap(),
        IssueFeedbackRecordResult::Conflict
    ));
    let events = adapter.pending_events(fixture.cwd()).unwrap();
    assert_eq!(
        events.iter().map(|event| event.action).collect::<Vec<_>>(),
        vec![
            IssueFeedbackAction::LabelAdded,
            IssueFeedbackAction::LabelRemoved
        ]
    );
}

#[test]
fn feedback_outbox_failure_rolls_back_the_receipt_owner_and_state_in_the_same_transaction() {
    let fixture = Fixture::new("feedback-outbox-rollback");
    let issue = seed(&fixture, "I_1");
    let adapter = fixture.adapter();
    save_desired(&adapter, &issue, "상태: 실행 중");
    let first = comment_intent(&current(&adapter, &issue), "before-overflow");
    let first = sent(&adapter, &recorded(&adapter, &first));
    confirmed(&adapter, &first, comment_receipt(&first, "IC_status"));
    save_desired(&adapter, &issue, "상태: 완료됨");
    let update = comment_intent(&current(&adapter, &issue), "overflow-update");
    let update = sent(&adapter, &recorded(&adapter, &update));
    let before = current(&adapter, &issue);

    // A valid schema and existing row enter complete(); only the later outbox
    // append fails after the receipt and owner writes have already run in its TX.
    let connection = Connection::open(fixture.database()).unwrap();
    connection
        .execute("UPDATE issue_feedback_outbox SET sequence=?1", [i64::MAX])
        .unwrap();
    super::super::super::validate_schema(&connection).unwrap();
    drop(connection);
    let error = adapter
        .complete(
            &issue,
            &update.intent.intent_id,
            update.revision,
            &IssueFeedbackCompletion::Confirmed(Box::new(comment_receipt(&update, "IC_status"))),
        )
        .unwrap_err();
    assert!(error.to_string().contains("outbox sequence is exhausted"));
    assert_eq!(current(&adapter, &issue), before);
    assert_eq!(adapter.unresolved(&issue).unwrap(), vec![update]);
    assert_eq!(count(&fixture, "issue_feedback_outbox"), 1);
    assert_eq!(
        adapter.pending_events(fixture.cwd()).unwrap()[0].intent_id,
        first.intent.intent_id
    );
}
