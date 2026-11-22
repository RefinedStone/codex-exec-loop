use super::*;

#[test]
fn unavailable_or_pending_control_disables_every_intervention() {
    let current = IssueWorkControl::default();
    for command in IssueActionCommand::ALL {
        for (control, pending) in [(None, false), (Some(&current), true)] {
            let result = action(command, control, true, pending);
            assert!(!result.enabled);
            assert!(result.reason.is_some());
        }
    }
}

#[test]
fn action_choices_follow_ownership_and_existing_control_without_starting_work() {
    let mut current = IssueWorkControl::default();
    assert!(action(IssueActionCommand::HoldIssue, Some(&current), false, false).enabled);
    assert!(
        !action(
            IssueActionCommand::ReleaseIssue,
            Some(&current),
            false,
            false
        )
        .enabled
    );
    assert!(!action(IssueActionCommand::SteerIssue, Some(&current), false, false).enabled);
    assert!(!action(IssueActionCommand::TrackIssue, Some(&current), false, false).enabled);
    assert!(
        action(
            IssueActionCommand::UntrackIssue,
            Some(&current),
            false,
            false
        )
        .enabled
    );

    current.held = true;
    assert!(!action(IssueActionCommand::HoldIssue, Some(&current), true, false).enabled);
    assert!(
        action(
            IssueActionCommand::ReleaseIssue,
            Some(&current),
            true,
            false
        )
        .enabled
    );
    assert!(action(IssueActionCommand::SteerIssue, Some(&current), true, false).enabled);

    current.taken_over = true;
    assert!(!action(IssueActionCommand::SteerIssue, Some(&current), true, false).enabled);
    assert!(
        !action(
            IssueActionCommand::TakeOverIssue,
            Some(&current),
            true,
            false
        )
        .enabled
    );

    current.cancelled = true;
    assert!(
        !action(
            IssueActionCommand::CancelIssueWork,
            Some(&current),
            true,
            false
        )
        .enabled
    );
    assert!(
        !action(
            IssueActionCommand::RetriageIssue,
            Some(&current),
            true,
            false
        )
        .enabled
    );
    current.tracked = false;
    assert!(action(IssueActionCommand::TrackIssue, Some(&current), true, false).enabled);
    assert!(
        !action(
            IssueActionCommand::UntrackIssue,
            Some(&current),
            true,
            false
        )
        .enabled
    );
}

#[test]
fn conversation_target_rejects_untrusted_thread_identifiers() {
    assert!(valid_thread_id("thread-123_abc"));
    for value in [
        "",
        " ",
        " thread",
        "thread ",
        "thread\nother",
        "thread\tother",
        "a\u{2003}b",
    ] {
        assert!(!valid_thread_id(value), "{value:?}");
    }
    assert!(!valid_thread_id(&"a".repeat(4097)));
}
