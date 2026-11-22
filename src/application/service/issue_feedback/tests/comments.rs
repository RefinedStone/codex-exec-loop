use super::*;
use crate::domain::issue_automation::source::IssueSourceContent;
use std::sync::atomic::Ordering;

#[test]
fn lost_ack_is_read_back_once_and_later_status_edits_the_same_comment() {
    let fixture = Fixture::new(true, false);
    *fixture.provider.write_behavior.lock().unwrap() = WriteBehavior::Lost;

    fixture.sync().unwrap();
    assert_eq!(
        fixture.store.record().state,
        IssueFeedbackIntentState::Confirmed
    );
    assert_eq!(fixture.writes().len(), 1);
    fixture.sync().unwrap();
    assert_eq!(
        fixture.writes().len(),
        1,
        "publication accounting must not rewrite its own status"
    );

    {
        let mut data = fixture.store.data.lock().unwrap();
        let progress = &mut data.snapshot.execution.as_mut().unwrap().progress;
        progress.attempts.verification = 1;
        progress.revision += 1;
    }
    fixture.sync().unwrap();
    let writes = fixture.writes();
    assert_eq!(writes.len(), 2);
    assert!(matches!(
        &writes[0].operation,
        IssueFeedbackOperation::CreateComment { .. }
    ));
    let IssueFeedbackOperation::UpdateComment { owner, body, .. } = &writes[1].operation else {
        panic!("the next status must update the confirmed comment");
    };
    assert_eq!(owner.comment_id, COMMENT);
    assert!(!body.contains("게시:"));
    fixture.sync().unwrap();
    assert_eq!(fixture.writes().len(), 2);
    assert_eq!(fixture.store.snapshot_value().publication_attempts, 2);
    let events = fixture.service().pending_events().unwrap();
    assert_eq!(events.len(), 2);
    fixture
        .service()
        .acknowledge_event(&events[0].event_id)
        .unwrap();
    assert_eq!(fixture.service().pending_events().unwrap(), events[1..]);
}

#[test]
fn lost_write_and_failed_read_never_resend_after_restart_even_if_off_or_policy_unreadable() {
    for unreadable in [false, true] {
        let fixture = Fixture::new(true, false);
        *fixture.provider.write_behavior.lock().unwrap() = WriteBehavior::Lost;
        *fixture.provider.read_behavior.lock().unwrap() = ReadBehavior::Unavailable;
        fixture.sync().unwrap();
        let original = fixture.store.record();
        assert_eq!(original.state, IssueFeedbackIntentState::NeedsConfirmation);
        fixture.sync().unwrap();
        assert_eq!(fixture.writes().len(), 1);

        if unreadable {
            fixture.policy.fail.store(true, Ordering::SeqCst);
        } else {
            fixture
                .policy
                .user
                .lock()
                .unwrap()
                .push_str("\n[binding.github]\nmode = 'off'\n");
        }
        *fixture.provider.read_behavior.lock().unwrap() = ReadBehavior::Confirm;
        let restarted = fixture.service();
        restarted
            .reconcile_pending(&IssueCollectionCancellation::new())
            .unwrap();
        restarted
            .reconcile_pending(&IssueCollectionCancellation::new())
            .unwrap();
        let confirmed = fixture.store.record();
        assert_eq!(confirmed.intent, original.intent);
        assert_eq!(confirmed.state, IssueFeedbackIntentState::Confirmed);
        assert_eq!(fixture.writes().len(), 1);
        assert_eq!(restarted.pending_events().unwrap().len(), 1);
    }
}

#[test]
fn a_different_readback_comment_cannot_replace_the_durable_ack_identity() {
    let fixture = Fixture::new(true, false);
    *fixture.provider.read_behavior.lock().unwrap() = ReadBehavior::DifferentComment;
    fixture.sync().unwrap();
    let rejected = fixture.store.record();
    assert_eq!(rejected.acknowledged_comment_id.as_deref(), Some(COMMENT));
    assert!(rejected.contradictory_response);
    assert_eq!(rejected.state, IssueFeedbackIntentState::NeedsConfirmation);
    let reads = fixture.provider.reconciles.load(Ordering::SeqCst);

    *fixture.provider.read_behavior.lock().unwrap() = ReadBehavior::Confirm;
    fixture
        .service()
        .reconcile_pending(&IssueCollectionCancellation::new())
        .unwrap();
    fixture.sync().unwrap();
    assert_eq!(fixture.store.record(), rejected);
    assert_eq!(fixture.provider.reconciles.load(Ordering::SeqCst), reads);
    assert_eq!(fixture.writes().len(), 1);
    assert!(fixture.service().pending_events().unwrap().is_empty());
}

#[test]
fn contradictory_write_response_is_sticky_and_cannot_create_a_replacement() {
    let fixture = Fixture::new(true, false);
    *fixture.provider.write_behavior.lock().unwrap() = WriteBehavior::Contradictory;
    fixture.sync().unwrap();
    assert!(fixture.store.record().contradictory_response);
    fixture
        .service()
        .reconcile_pending(&IssueCollectionCancellation::new())
        .unwrap();
    fixture.sync().unwrap();
    assert_eq!(fixture.writes().len(), 1);
    assert_eq!(fixture.provider.reconciles.load(Ordering::SeqCst), 0);
}

#[test]
fn failure_to_save_an_ack_leaves_sent_and_restart_only_reads() {
    let fixture = Fixture::new(true, false);
    fixture.store.fail_ack_save.store(true, Ordering::SeqCst);
    assert!(fixture.sync().is_err());
    assert_eq!(fixture.writes().len(), 1);
    assert_eq!(fixture.store.record().state, IssueFeedbackIntentState::Sent);
    assert_eq!(fixture.store.record().acknowledged_comment_id, None);

    fixture
        .service()
        .reconcile_pending(&IssueCollectionCancellation::new())
        .unwrap();
    assert_eq!(
        fixture.store.record().state,
        IssueFeedbackIntentState::Confirmed
    );
    assert_eq!(fixture.writes().len(), 1);
    assert_eq!(fixture.store.snapshot_value().publication_attempts, 1);
}

#[test]
fn restart_abandons_recorded_without_claiming_or_sending_it() {
    let fixture = Fixture::new(true, false);
    let mut record = support::pending_record(issue());
    record.state = IssueFeedbackIntentState::Recorded;
    record.revision = 1;
    fixture.store.data.lock().unwrap().records.push(record);
    fixture
        .service()
        .reconcile_pending(&IssueCollectionCancellation::new())
        .unwrap();
    assert_eq!(
        fixture.store.record().state,
        IssueFeedbackIntentState::Abandoned
    );
    assert!(fixture.writes().is_empty());
    assert_eq!(fixture.provider.reconciles.load(Ordering::SeqCst), 0);
}

#[test]
fn changed_actor_does_not_fall_back_to_a_second_comment() {
    let fixture = Fixture::new(true, false);
    fixture.sync().unwrap();
    {
        let mut data = fixture.store.data.lock().unwrap();
        let progress = &mut data.snapshot.execution.as_mut().unwrap().progress;
        progress.attempts.verification += 1;
        progress.revision += 1;
    }
    *fixture.provider.actor.lock().unwrap() = "U_another".into();
    assert!(
        fixture
            .sync()
            .unwrap_err()
            .to_string()
            .contains("feedback_unconfirmed")
    );
    assert_eq!(fixture.writes().len(), 1);
    assert_eq!(
        fixture
            .store
            .snapshot_value()
            .state
            .comment_owner
            .unwrap()
            .comment_id,
        COMMENT
    );
}

#[test]
fn shared_status_uses_the_real_secret_detector_and_never_logs_raw_content() {
    let secret = format!("ghp_{}", "a".repeat(36));
    let raw = format!("상태: 실행 중\ncredential: {secret}");
    let desired = masked_status(&raw, &IssuePolicy::builtin()).unwrap();
    assert!(!desired.comment_body.contains(&secret));
    assert!(desired.comment_body.contains("상태: 실행 중"));
    assert!(!format!("{desired:?}").contains(&raw));

    let fixture = Fixture::new(true, false);
    {
        let mut data = fixture.store.data.lock().unwrap();
        let Some(IssueSourceRead::Observed(source)) = &mut data.snapshot.item.current else {
            unreachable!()
        };
        let IssueSourceContent::Shared(shared) = &mut source.content else {
            unreachable!()
        };
        shared.shared_copy.body = secret.clone();
        shared.shared_copy.title = "raw agent instructions must not be copied".into();
    }
    fixture.sync().unwrap();
    let writes = fixture.writes();
    let IssueFeedbackOperation::CreateComment { body, .. } = &writes[0].operation else {
        unreachable!()
    };
    assert!(!body.contains(&secret));
    assert!(!body.contains("raw agent instructions"));
    assert!(!format!("{:?}", writes[0]).contains(body));
}
