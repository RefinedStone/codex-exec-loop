use super::*;
use crate::domain::issue_automation::autonomy::{IssueAutonomyIntent, IssueAutonomyLevel};

#[test]
fn version_thirteen_migration_keeps_legacy_binding_ids_readable_without_relaxing_new_commands() {
    for id in [" legacy binding ".to_owned(), "x".repeat(513)] {
        let fixture = Fixture::new("binding-pause-legacy-binding-id");
        let binding = insert(&fixture, &id);
        {
            let connection = Connection::open(fixture.database()).unwrap();
            super::super::super::tests::make_pre_binding_pause_schema(&connection);
            connection.pragma_update(None, "user_version", 13).unwrap();
        }
        let reopened = fixture.adapter();
        let state = reopened
            .issue_binding_pause_state(fixture.cwd(), &binding)
            .unwrap();
        assert_eq!(state.binding_id, binding);
        assert_eq!(state.revision, 0);
        assert!(state.allows_automation());
        assert_eq!(
            reopened
                .list_issue_binding_pause_states(fixture.cwd(), None, 64)
                .unwrap(),
            vec![state]
        );
        assert!(
            reopened
                .begin_issue_binding_pause(
                    fixture.cwd(),
                    &intent(&fixture, &binding, "new-pause", true)
                )
                .is_err()
        );
        assert_eq!(count(&fixture, "issue_binding_pause_commands"), 0);
    }
}

#[test]
fn version_thirteen_migration_preserves_old_rows_and_receipts_and_seeds_only_open_states() {
    let fixture = Fixture::new("binding-pause-migration");
    let (binding, issue) = fixture.insert();
    let adapter = fixture.adapter();
    let command = IssueAutonomyIntent {
        command_id: "old-auto".into(),
        level: IssueAutonomyLevel::Auto,
        expected_revision: 0,
        policy_version: "a".repeat(64),
    };
    adapter
        .begin_issue_autonomy(fixture.cwd(), &command)
        .unwrap();
    let old = adapter
        .finish_issue_autonomy(fixture.cwd(), &command)
        .unwrap();
    let selection = adapter.issue_autonomy_selection(fixture.cwd()).unwrap();
    let catalog = adapter
        .read_triage_batch(fixture.cwd(), None, 100)
        .unwrap()
        .revision;
    {
        let connection = Connection::open(fixture.database()).unwrap();
        super::super::super::tests::make_pre_binding_pause_schema(&connection);
        connection.pragma_update(None, "user_version", 13).unwrap();
        super::super::super::validate_schema_version(&connection, 13).unwrap();
    }
    let reopened = fixture.adapter();
    assert_eq!(
        reopened
            .get_issue(fixture.cwd(), issue.reference())
            .unwrap(),
        Some(issue)
    );
    assert_eq!(
        reopened
            .get_binding(fixture.cwd(), binding.id())
            .unwrap()
            .unwrap()
            .binding,
        binding
    );
    assert_eq!(
        reopened.issue_autonomy_selection(fixture.cwd()).unwrap(),
        selection
    );
    assert_eq!(
        reopened
            .get_issue_autonomy_command(fixture.cwd(), "old-auto")
            .unwrap(),
        Some(old.command)
    );
    assert_eq!(
        reopened
            .read_triage_batch(fixture.cwd(), None, 100)
            .unwrap()
            .revision,
        catalog
    );
    let state = reopened
        .issue_binding_pause_state(fixture.cwd(), binding.id())
        .unwrap();
    assert_eq!(state.revision, 0);
    assert!(state.allows_automation());
    assert_eq!(count(&fixture, "issue_binding_pause_states"), 1);
    assert_eq!(count(&fixture, "issue_binding_pause_commands"), 0);
    assert_eq!(count(&fixture, "issue_binding_pause_work_fences"), 0);
    let connection = Connection::open(fixture.database()).unwrap();
    assert_eq!(
        super::super::super::schema_identity(&connection).unwrap(),
        (
            super::super::super::APPLICATION_ID,
            super::super::super::SCHEMA_VERSION
        )
    );
    super::super::super::validate_schema(&connection).unwrap();
}

#[test]
fn missing_state_is_not_recreated_as_running_and_changed_v13_schema_is_rejected() {
    let fixture = Fixture::new("binding-pause-missing-state");
    let binding = insert(&fixture, "binding");
    Connection::open(fixture.database())
        .unwrap()
        .execute("DELETE FROM issue_binding_pause_states", [])
        .unwrap();
    assert!(
        fixture
            .adapter()
            .issue_binding_pause_state(fixture.cwd(), &binding)
            .is_err()
    );
    assert!(
        fixture
            .adapter()
            .list_issue_binding_pause_states(fixture.cwd(), None, 1)
            .is_err()
    );
    let fixture = Fixture::new("binding-pause-schema-mismatch");
    let binding = insert(&fixture, "binding");
    Connection::open(fixture.database())
        .unwrap()
        .pragma_update(None, "user_version", 13)
        .unwrap();
    assert!(
        fixture
            .adapter()
            .issue_binding_pause_state(fixture.cwd(), &binding)
            .is_err()
    );
    assert_eq!(count(&fixture, "issue_binding_pause_commands"), 0);
}

#[test]
fn failed_begin_and_finish_roll_back_state_receipt_and_outbox_together() {
    for fail_at_finish in [false, true] {
        let fixture = Fixture::new("binding-pause-rollback");
        let binding = insert(&fixture, "binding");
        let command = intent(&fixture, &binding, "pause", true);
        if fail_at_finish {
            fixture
                .adapter()
                .begin_issue_binding_pause(fixture.cwd(), &command)
                .unwrap();
        }
        let adapter = fixture.adapter();
        let before = adapter
            .issue_binding_pause_state(fixture.cwd(), &binding)
            .unwrap();
        Connection::open(fixture.database())
            .unwrap()
            .execute(
                "UPDATE issue_automation_catalog SET revision=?1",
                [MAX_ISSUE_AUTOMATION_REVISION as i64],
            )
            .unwrap();
        if fail_at_finish {
            assert!(
                adapter
                    .finish_issue_binding_pause(
                        fixture.cwd(),
                        &command,
                        IssueBindingPauseCompletion::Apply
                    )
                    .is_err()
            );
        } else {
            assert!(
                adapter
                    .begin_issue_binding_pause(fixture.cwd(), &command)
                    .is_err()
            );
        }
        assert_eq!(
            adapter
                .issue_binding_pause_state(fixture.cwd(), &binding)
                .unwrap(),
            before
        );
        assert_eq!(
            count(&fixture, "issue_binding_pause_commands"),
            i64::from(fail_at_finish)
        );
        assert!(
            adapter
                .pending_issue_binding_pause_events(fixture.cwd(), 64)
                .unwrap()
                .is_empty()
        );
    }
}
