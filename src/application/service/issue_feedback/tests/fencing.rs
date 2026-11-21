use super::*;
use std::sync::atomic::Ordering;

#[test]
fn current_policy_is_rechecked_after_actor_lookup_before_sent() {
    let fixture = Fixture::new(true, false);
    let policy = fixture.policy.clone();
    *fixture.provider.after_actor.lock().unwrap() = Some(Box::new(move || {
        *policy.user.lock().unwrap() = "[feedback]\ncomment = false\n".into();
    }));
    fixture.sync().unwrap();
    assert!(fixture.writes().is_empty());
    assert_eq!(
        fixture.store.record().state,
        IssueFeedbackIntentState::Abandoned
    );
    assert_eq!(fixture.store.snapshot_value().publication_attempts, 0);
}

#[test]
fn a_recorded_intent_does_not_adopt_a_new_source_work_or_authorization() {
    for change in 0..4 {
        let fixture = Fixture::new(true, false);
        let store = fixture.store.clone();
        *fixture.store.after_record.lock().unwrap() = Some(Box::new(move || {
            let mut data = store.data.lock().unwrap();
            match change {
                0 => data.snapshot.authorization_epoch += 1,
                1 => data.snapshot.item.observation_revision = Some(2),
                2 => data.snapshot.work.as_mut().unwrap().revision += 1,
                3 => data.snapshot.autonomy.revision += 1,
                _ => unreachable!(),
            }
        }));
        fixture.sync().unwrap();
        assert!(fixture.writes().is_empty(), "authority change {change}");
        assert_eq!(
            fixture.store.record().state,
            IssueFeedbackIntentState::Abandoned
        );
        assert_eq!(fixture.store.snapshot_value().publication_attempts, 0);
    }
}

#[test]
fn cancellation_after_recording_prevents_sent_and_the_external_call() {
    let fixture = Fixture::new(true, false);
    let cancel = Arc::new(IssueCollectionCancellation::new());
    let hook_cancel = cancel.clone();
    *fixture.store.after_record_read.lock().unwrap() = Some(Box::new(move || hook_cancel.cancel()));
    fixture
        .service()
        .sync_issue(&issue(), 123, &cancel)
        .unwrap();
    assert!(fixture.writes().is_empty());
    assert_eq!(
        fixture.store.record().state,
        IssueFeedbackIntentState::Abandoned
    );
    assert!(!fixture.journal.held.load(Ordering::SeqCst));
}

#[test]
fn unknown_title_body_publication_blocks_feedback_for_the_same_remote_issue_alias() {
    let fixture = Fixture::new(true, true);
    fixture
        .journal
        .records
        .lock()
        .unwrap()
        .push(support::pending_publication(issue()));
    fixture.sync().unwrap();
    assert!(!fixture.grant().unwrap());
    assert!(fixture.writes().is_empty());
    assert_eq!(fixture.provider.actor_reads.load(Ordering::SeqCst), 0);
    assert!(!fixture.store.snapshot_value().first_grant_entered);
}

#[test]
fn repository_feedback_true_cannot_enable_user_disabled_writes() {
    let fixture = Fixture::new(false, false);
    *fixture.policy.project.lock().unwrap() =
        Some("[feedback]\ncomment = true\nlabels = true\n".into());
    fixture.sync().unwrap();
    assert!(
        fixture.grant().unwrap(),
        "disabled labels need no provider mutation"
    );
    assert!(fixture.writes().is_empty());
    assert_eq!(fixture.provider.actor_reads.load(Ordering::SeqCst), 0);
}

#[test]
fn held_stale_unknown_and_pending_intervention_cannot_start_new_feedback() {
    for blocked in 0..5 {
        let fixture = Fixture::new(true, true);
        {
            let mut data = fixture.store.data.lock().unwrap();
            match blocked {
                0 => data.snapshot.item.tracked.as_mut().unwrap().flags.held = true,
                1 => data.snapshot.item.tracked.as_mut().unwrap().flags.stale = true,
                2 => data.snapshot.item.tracked.as_mut().unwrap().flags.unknown = true,
                3 => data.snapshot.pending_intervention = true,
                4 => data.snapshot.collection.config.mode = IssueCollectionMode::Off,
                _ => unreachable!(),
            }
        }
        fixture.sync().unwrap();
        assert!(!fixture.grant().unwrap());
        assert!(fixture.writes().is_empty(), "blocked context {blocked}");
    }
}
