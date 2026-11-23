use super::*;

#[test]
fn version_twelve_migration_preserves_selection_pending_and_receipts_before_auto_reopen() {
    let fixture = Fixture::new("autonomy-auto-migration");
    let (binding, issue) = fixture.insert();
    let adapter = fixture.adapter();
    let gated = intent("gated-v12", IssueAutonomyLevel::Gated, 0);
    adapter.begin_issue_autonomy(fixture.cwd(), &gated).unwrap();
    let completed = adapter
        .finish_issue_autonomy(fixture.cwd(), &gated)
        .unwrap();
    let pending = intent("pending-v12", IssueAutonomyLevel::Propose, 1);
    let begun = adapter
        .begin_issue_autonomy(fixture.cwd(), &pending)
        .unwrap();
    let before = adapter.issue_autonomy_selection(fixture.cwd()).unwrap();
    drop(adapter);
    {
        let connection = Connection::open(fixture.database()).unwrap();
        super::super::super::tests::make_pre_auto_schema(&connection);
        connection.pragma_update(None, "user_version", 12).unwrap();
        super::super::super::validate_schema_version(&connection, 12).unwrap();
    }

    let migrated = fixture.adapter();
    assert_eq!(
        migrated.issue_autonomy_selection(fixture.cwd()).unwrap(),
        before
    );
    assert_eq!(
        migrated
            .get_issue_autonomy_command(fixture.cwd(), "gated-v12")
            .unwrap(),
        Some(completed.command.clone())
    );
    assert_eq!(
        migrated
            .get_issue_autonomy_command(fixture.cwd(), "pending-v12")
            .unwrap(),
        Some(begun.clone())
    );
    assert_eq!(
        migrated
            .begin_issue_autonomy(fixture.cwd(), &pending)
            .unwrap(),
        begun
    );
    assert_eq!(
        migrated
            .get_issue(fixture.cwd(), issue.reference())
            .unwrap(),
        Some(issue)
    );
    assert_eq!(
        migrated.list_bindings(fixture.cwd()).unwrap()[0].binding,
        binding
    );
    assert_eq!(
        migrated
            .finish_issue_autonomy(fixture.cwd(), &pending)
            .unwrap()
            .command
            .completed_revision,
        Some(2)
    );

    let auto = intent("auto-v13", IssueAutonomyLevel::Auto, 2);
    migrated.begin_issue_autonomy(fixture.cwd(), &auto).unwrap();
    let receipt = migrated
        .finish_issue_autonomy(fixture.cwd(), &auto)
        .unwrap();
    assert!(receipt.changed);
    assert_eq!(receipt.command.completed_revision, Some(3));
    drop(migrated);
    let reopened = fixture.adapter();
    assert_eq!(
        reopened.issue_autonomy_selection(fixture.cwd()).unwrap(),
        IssueAutonomySelection {
            requested_level: Some(IssueAutonomyLevel::Auto),
            revision: 3,
            pending: None,
        }
    );
    let replay = reopened
        .finish_issue_autonomy(fixture.cwd(), &auto)
        .unwrap();
    assert!(!replay.changed);
    assert_eq!(replay.command, receipt.command);
    assert_eq!(
        reopened
            .get_issue_autonomy_command(fixture.cwd(), "gated-v12")
            .unwrap(),
        Some(completed.command)
    );
    let connection = Connection::open(fixture.database()).unwrap();
    assert_eq!(
        connection
            .pragma_query_value(None, "user_version", |row| row.get::<_, i64>(0))
            .unwrap(),
        super::super::super::SCHEMA_VERSION
    );
    super::super::super::validate_schema(&connection).unwrap();
    assert!(
        connection
            .execute(
                "UPDATE issue_autonomy_selection SET requested_level='automatic'",
                []
            )
            .is_err()
    );
}

#[test]
fn a_version_twelve_database_with_a_changed_selection_check_is_not_silently_migrated() {
    let fixture = Fixture::new("autonomy-auto-schema-mismatch");
    fixture
        .adapter()
        .issue_autonomy_selection(fixture.cwd())
        .unwrap();
    {
        let connection = Connection::open(fixture.database()).unwrap();
        // A genuine v12 database has GATED_SCHEMA, not the newer five-value CHECK.
        connection.pragma_update(None, "user_version", 12).unwrap();
    }
    assert!(
        fixture
            .adapter()
            .issue_autonomy_selection(fixture.cwd())
            .is_err()
    );
    let connection = Connection::open(fixture.database()).unwrap();
    assert_eq!(
        connection
            .pragma_query_value(None, "user_version", |row| row.get::<_, i64>(0))
            .unwrap(),
        12
    );
}

#[test]
fn automatic_database_guards_require_the_matching_explicit_runtime_choice() {
    for requested in std::iter::once(None).chain(IssueAutonomyLevel::ALL.into_iter().map(Some)) {
        let selection = IssueAutonomySelection {
            requested_level: requested,
            revision: 3,
            pending: None,
        };
        assert_eq!(
            matches_automatic_mode(&selection, IssueAutomationAutonomy::Auto),
            requested == Some(IssueAutonomyLevel::Auto)
        );
        assert_eq!(
            matches_automatic_mode(&selection, IssueAutomationAutonomy::Gated),
            matches!(
                requested,
                Some(IssueAutonomyLevel::Gated | IssueAutonomyLevel::Auto)
            )
        );
        for inactive in [
            IssueAutomationAutonomy::Off,
            IssueAutomationAutonomy::Observe,
            IssueAutomationAutonomy::Propose,
        ] {
            assert!(!matches_automatic_mode(&selection, inactive));
        }
    }
}
