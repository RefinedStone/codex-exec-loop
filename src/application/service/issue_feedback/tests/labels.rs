use super::*;
use crate::application::port::outbound::issue_feedback_port::IssueFeedbackLabelEvent;

#[test]
fn first_grant_waits_for_readback_and_never_resends_the_label_after_restart() {
    let fixture = Fixture::new(false, true);
    *fixture.provider.write_behavior.lock().unwrap() = WriteBehavior::Lost;
    *fixture.provider.read_behavior.lock().unwrap() = ReadBehavior::Unavailable;
    assert!(!fixture.grant().unwrap());
    assert!(!fixture.store.snapshot_value().first_grant_entered);
    assert!(!fixture.grant().unwrap());
    assert_eq!(fixture.writes().len(), 1);

    *fixture.provider.read_behavior.lock().unwrap() = ReadBehavior::Confirm;
    fixture
        .service()
        .reconcile_pending(&IssueCollectionCancellation::new())
        .unwrap();
    assert!(fixture.grant().unwrap());
    assert!(fixture.grant().unwrap());
    assert_eq!(fixture.writes().len(), 1);
    assert!(fixture.store.snapshot_value().first_grant_entered);
}

#[test]
fn an_existing_foreign_label_is_not_owned_from_the_same_actor_or_its_name() {
    let fixture = Fixture::new(false, true);
    {
        let mut label = fixture.provider.label.lock().unwrap();
        label.present = true;
        label.latest_event = Some(IssueFeedbackLabelEvent {
            event_id: "manual-event".into(),
            actor_id: Some(ACTOR.into()),
            applied: true,
        });
    }
    assert!(!fixture.grant().unwrap());
    assert!(fixture.writes().is_empty());
    assert!(fixture.store.snapshot_value().state.label_proof.is_none());
    assert!(!fixture.store.snapshot_value().first_grant_entered);
}

#[test]
fn missing_repository_label_is_not_created_and_does_not_open_the_grant() {
    let fixture = Fixture::new(false, true);
    fixture.provider.label.lock().unwrap().label = None;
    assert!(!fixture.grant().unwrap());
    assert!(fixture.writes().is_empty());
    assert!(!fixture.store.snapshot_value().first_grant_entered);
}

#[test]
fn a_completed_first_grant_is_not_reopened_by_later_label_policy_or_unknown_comment() {
    let fixture = Fixture::new(true, false);
    assert!(fixture.grant().unwrap());
    *fixture.policy.user.lock().unwrap() = "[feedback]\ncomment = true\nlabels = true\n".into();
    *fixture.provider.read_behavior.lock().unwrap() = ReadBehavior::Unavailable;
    fixture.sync().unwrap();
    assert_eq!(
        fixture.store.record().state,
        IssueFeedbackIntentState::NeedsConfirmation
    );
    assert!(
        fixture.grant().unwrap(),
        "an unknown status comment must not revoke ongoing work"
    );
    let writes = fixture.writes();
    assert_eq!(writes.len(), 1);
    assert!(matches!(
        writes[0].operation,
        IssueFeedbackOperation::CreateComment { .. }
    ));
}

#[test]
fn comment_disabled_does_not_block_terminal_owned_label_removal() {
    for state in [
        IssueAutomationState::Done,
        IssueAutomationState::Cancelled,
        IssueAutomationState::Failed,
    ] {
        let fixture = Fixture::new(false, true);
        assert!(fixture.grant().unwrap());
        let original = fixture.store.snapshot_value().state.label_proof.unwrap();
        {
            let mut data = fixture.store.data.lock().unwrap();
            data.snapshot.item.tracked.as_mut().unwrap().state = state;
            data.snapshot.item.tracked_revision = Some(2);
        }
        fixture.sync().unwrap();
        let writes = fixture.writes();
        assert_eq!(writes.len(), 2, "{state:?}");
        assert!(matches!(
            writes[1].operation,
            IssueFeedbackOperation::RemoveLabel { .. }
        ));
        let stored = fixture.store.snapshot_value();
        assert_eq!(stored.state.label_proof, Some(original));
        assert!(stored.state.label_removal_confirmed);
        assert!(!fixture.provider.label.lock().unwrap().present);
        fixture.sync().unwrap();
        assert_eq!(fixture.writes().len(), 2);
    }
}

#[test]
fn failed_execution_can_remove_owned_progress_before_the_tracked_summary_catches_up() {
    let fixture = Fixture::new(false, true);
    assert!(fixture.grant().unwrap());
    {
        let mut data = fixture.store.data.lock().unwrap();
        let progress = &mut data.snapshot.execution.as_mut().unwrap().progress;
        progress.state = IssueExecutionState::Failed;
        progress.revision += 1;
    }
    fixture.sync().unwrap();
    assert_eq!(fixture.writes().len(), 2);
    assert!(fixture.store.snapshot_value().state.label_removal_confirmed);
}

#[test]
fn a_new_manual_event_is_not_removed_even_when_the_actor_and_label_match() {
    let fixture = Fixture::new(false, true);
    assert!(fixture.grant().unwrap());
    fixture
        .store
        .data
        .lock()
        .unwrap()
        .snapshot
        .item
        .tracked
        .as_mut()
        .unwrap()
        .state = IssueAutomationState::Done;
    fixture
        .provider
        .label
        .lock()
        .unwrap()
        .latest_event
        .as_mut()
        .unwrap()
        .event_id = "later-human-event".into();
    fixture.sync().unwrap();
    assert_eq!(fixture.writes().len(), 1);
    assert!(!fixture.store.snapshot_value().state.label_removal_confirmed);
    assert!(fixture.provider.label.lock().unwrap().present);
}

#[test]
fn a_control_label_collision_blocks_add_and_later_remove() {
    let fixture = Fixture::new(false, true);
    fixture
        .policy
        .user
        .lock()
        .unwrap()
        .push_str("\n[labels]\nhold = 'akra:running'\n");
    assert!(!fixture.grant().unwrap());
    assert!(fixture.writes().is_empty());

    *fixture.policy.user.lock().unwrap() = "[feedback]\ncomment = false\nlabels = true\n".into();
    assert!(fixture.grant().unwrap());
    fixture
        .store
        .data
        .lock()
        .unwrap()
        .snapshot
        .item
        .tracked
        .as_mut()
        .unwrap()
        .state = IssueAutomationState::Done;
    fixture
        .policy
        .user
        .lock()
        .unwrap()
        .push_str("\n[labels]\nhold = 'akra:running'\n");
    fixture.sync().unwrap();
    assert_eq!(fixture.writes().len(), 1);
    assert!(!fixture.store.snapshot_value().state.label_removal_confirmed);
}
