use super::*;
use crate::domain::issue_automation::autonomy::{IssueAutonomyIntent, IssueAutonomyLevel};

#[test]
fn pending_generation_is_durable_and_one_binding_does_not_pause_another() {
    let fixture = Fixture::new("binding-pause-isolation");
    let a = insert(&fixture, "binding-a");
    let b = insert(&fixture, "binding-b");
    let command = intent(&fixture, &a, "pause-a", true);
    let accepted = fixture
        .adapter()
        .begin_issue_binding_pause(fixture.cwd(), &command)
        .unwrap();
    assert_eq!(accepted.accepted_revision, 1);
    assert!(accepted.receipt.is_none());
    let reopened = fixture.adapter();
    let state = reopened
        .issue_binding_pause_state(fixture.cwd(), &a)
        .unwrap();
    assert_eq!(state.pending, Some(command.clone()));
    assert!(!state.allows_automation());
    assert!(!state.paused, "acceptance and completed pause are distinct");
    assert!(
        reopened
            .issue_binding_pause_state(fixture.cwd(), &b)
            .unwrap()
            .allows_automation()
    );
    assert_eq!(
        reopened
            .begin_issue_binding_pause(fixture.cwd(), &command)
            .unwrap(),
        accepted
    );
    let mut reused = command.clone();
    reused.expected_revision = 1;
    assert!(
        reopened
            .begin_issue_binding_pause(fixture.cwd(), &reused)
            .is_err()
    );
    let mut competing = command.clone();
    competing.command_id = "competing".into();
    assert!(
        reopened
            .begin_issue_binding_pause(fixture.cwd(), &competing)
            .is_err()
    );
    let completed = finish(&fixture, &command);
    assert_eq!(
        completed.receipt.as_ref().unwrap().outcome,
        IssueBindingPauseOutcome::Paused
    );
    assert_eq!(finish(&fixture, &command), completed);
    assert_eq!(count(&fixture, "issue_binding_pause_commands"), 1);
}

#[test]
fn noop_and_historical_replay_never_change_the_current_generation() {
    let fixture = Fixture::new("binding-pause-replay");
    let binding = insert(&fixture, "binding");
    let unchanged = intent(&fixture, &binding, "already-running", false);
    let noop = fixture
        .adapter()
        .begin_issue_binding_pause(fixture.cwd(), &unchanged)
        .unwrap();
    let receipt = noop.receipt.as_ref().unwrap();
    assert_eq!(receipt.outcome, IssueBindingPauseOutcome::Unchanged);
    assert_eq!(receipt.revision, 0);
    assert!(!receipt.changed);
    let pause = intent(&fixture, &binding, "pause", true);
    fixture
        .adapter()
        .begin_issue_binding_pause(fixture.cwd(), &pause)
        .unwrap();
    let first = finish(&fixture, &pause);
    let paused_noop = intent(&fixture, &binding, "already-paused", true);
    let second = fixture
        .adapter()
        .begin_issue_binding_pause(fixture.cwd(), &paused_noop)
        .unwrap();
    assert_eq!(
        second.receipt.as_ref().unwrap().outcome,
        IssueBindingPauseOutcome::Unchanged
    );
    assert_eq!(second.accepted_revision, 1);
    let resume = intent(&fixture, &binding, "resume", false);
    fixture
        .adapter()
        .begin_issue_binding_pause(fixture.cwd(), &resume)
        .unwrap();
    let resumed = finish(&fixture, &resume);
    assert_eq!(
        resumed.receipt.as_ref().unwrap().outcome,
        IssueBindingPauseOutcome::Resumed
    );
    let adapter = fixture.adapter();
    assert_eq!(
        adapter
            .begin_issue_binding_pause(fixture.cwd(), &pause)
            .unwrap(),
        first
    );
    assert_eq!(finish(&fixture, &pause), first);
    let current = adapter
        .issue_binding_pause_state(fixture.cwd(), &binding)
        .unwrap();
    assert!(current.allows_automation());
    assert_eq!(current.revision, 2);
    assert_eq!(count(&fixture, "issue_binding_pause_work_fences"), 0);
}

#[test]
fn resume_rechecks_metadata_and_global_selection_without_requiring_policy_for_pause() {
    for change_metadata in [false, true] {
        let fixture = Fixture::new("binding-resume-basis");
        let binding = insert(&fixture, "binding");
        let pause = intent(&fixture, &binding, "pause-without-policy", true);
        assert!(pause.resume_basis.is_none());
        let adapter = fixture.adapter();
        adapter
            .begin_issue_binding_pause(fixture.cwd(), &pause)
            .unwrap();
        finish(&fixture, &pause);
        let resume = intent(&fixture, &binding, "resume", false);
        adapter
            .begin_issue_binding_pause(fixture.cwd(), &resume)
            .unwrap();
        if change_metadata {
            let mut stored = adapter
                .get_binding(fixture.cwd(), &binding)
                .unwrap()
                .unwrap();
            let different = if stored.binding.autonomy() == IssueAutomationAutonomy::Off {
                IssueAutomationAutonomy::Observe
            } else {
                IssueAutomationAutonomy::Off
            };
            assert!(stored.binding.set_autonomy(different));
            adapter
                .save_binding(fixture.cwd(), stored.revision, &stored.binding)
                .unwrap();
        } else {
            let autonomy = IssueAutonomyIntent {
                command_id: "global-off".into(),
                level: IssueAutonomyLevel::Off,
                expected_revision: 0,
                policy_version: "b".repeat(64),
            };
            adapter
                .begin_issue_autonomy(fixture.cwd(), &autonomy)
                .unwrap();
        }
        let completed = finish(&fixture, &resume);
        assert_eq!(
            completed.receipt.as_ref().unwrap().outcome,
            IssueBindingPauseOutcome::ResumeSuperseded
        );
        assert!(!completed.receipt.as_ref().unwrap().changed);
        let state = adapter
            .issue_binding_pause_state(fixture.cwd(), &binding)
            .unwrap();
        assert!(state.paused && state.pending.is_none());
        assert_eq!(state.revision, 2);
    }
}

#[test]
fn outbox_uses_the_first_receipt_and_exact_ack_even_after_resume_and_reopen() {
    let fixture = Fixture::new("binding-pause-outbox");
    let binding = insert(&fixture, "binding");
    let pause = intent(&fixture, &binding, "pause", true);
    fixture
        .adapter()
        .begin_issue_binding_pause(fixture.cwd(), &pause)
        .unwrap();
    let first = finish(&fixture, &pause);
    let event = fixture
        .adapter()
        .pending_issue_binding_pause_events(fixture.cwd(), 1)
        .unwrap()
        .pop()
        .unwrap();
    assert_eq!(event.receipt, first.receipt.unwrap());
    let resume = intent(&fixture, &binding, "resume", false);
    fixture
        .adapter()
        .begin_issue_binding_pause(fixture.cwd(), &resume)
        .unwrap();
    finish(&fixture, &resume);
    let reopened = fixture.adapter();
    assert!(
        reopened
            .pending_issue_binding_pause_events(fixture.cwd(), 64)
            .unwrap()
            .contains(&event)
    );
    let mut forged = event.clone();
    forged.receipt.changed = false;
    assert!(
        reopened
            .acknowledge_issue_binding_pause_event(fixture.cwd(), &forged)
            .is_err()
    );
    forged = event.clone();
    forged.event_id.push('x');
    assert!(
        reopened
            .acknowledge_issue_binding_pause_event(fixture.cwd(), &forged)
            .is_err()
    );
    reopened
        .acknowledge_issue_binding_pause_event(fixture.cwd(), &event)
        .unwrap();
    reopened
        .acknowledge_issue_binding_pause_event(fixture.cwd(), &event)
        .unwrap();
    assert_eq!(
        reopened
            .pending_issue_binding_pause_events(fixture.cwd(), 64)
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        reopened
            .get_issue_binding_pause_command(fixture.cwd(), "pause")
            .unwrap()
            .unwrap()
            .receipt,
        Some(event.receipt)
    );
}

#[test]
fn binding_and_pending_pages_are_bounded_and_use_stable_binding_keys() {
    let fixture = Fixture::new("binding-pause-pages");
    let adapter = fixture.adapter();
    for index in 0..65 {
        let binding = insert(&fixture, &format!("binding-{index:03}"));
        let command = intent(&fixture, &binding, &format!("pause-{index:03}"), true);
        adapter
            .begin_issue_binding_pause(fixture.cwd(), &command)
            .unwrap();
    }
    let page = adapter
        .list_issue_binding_pause_states(fixture.cwd(), None, 64)
        .unwrap();
    assert_eq!(page.len(), 64);
    let next = adapter
        .list_issue_binding_pause_states(fixture.cwd(), Some(&page[63].binding_id), 64)
        .unwrap();
    assert_eq!(next.len(), 1);
    let commands = adapter
        .pending_issue_binding_pause_commands(fixture.cwd(), None, 64)
        .unwrap();
    assert_eq!(commands.len(), 64);
    assert_eq!(
        adapter
            .pending_issue_binding_pause_commands(
                fixture.cwd(),
                Some(&commands[63].intent.binding_id),
                64
            )
            .unwrap()
            .len(),
        1
    );
    for limit in [0, 65] {
        assert!(
            adapter
                .list_issue_binding_pause_states(fixture.cwd(), None, limit)
                .is_err()
        );
        assert!(
            adapter
                .pending_issue_binding_pause_commands(fixture.cwd(), None, limit)
                .is_err()
        );
        assert!(
            adapter
                .pending_issue_binding_pause_events(fixture.cwd(), limit)
                .is_err()
        );
    }
}
