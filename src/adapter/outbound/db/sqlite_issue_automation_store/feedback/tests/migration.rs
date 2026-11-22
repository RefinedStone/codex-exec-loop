use super::*;
use crate::domain::issue_automation::autonomy::{IssueAutonomyIntent, IssueAutonomyLevel};
use rusqlite::types::Value;

fn make_version_eight(connection: &Connection) {
    connection
        .execute_batch(
            "DROP TABLE issue_feedback_outbox;
         DROP TABLE issue_feedback_intents;
         DROP TABLE issue_feedback_state;
         DROP TABLE issue_feedback_grant_entries;",
        )
        .unwrap();
    connection.pragma_update(None, "user_version", 8).unwrap();
    super::super::super::validate_schema_version(connection, 8).unwrap();
}

fn legacy_schema(connection: &Connection) -> Vec<(String, String)> {
    connection
        .prepare(
            "SELECT name,sql FROM sqlite_master
         WHERE name NOT LIKE 'sqlite_%' AND name NOT LIKE 'issue_feedback_%'
         ORDER BY name",
        )
        .unwrap()
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap()
}

fn legacy_rows(connection: &Connection) -> Vec<(String, Vec<Vec<Value>>)> {
    let tables = connection
        .prepare(
            "SELECT name FROM sqlite_master WHERE type='table'
         AND name NOT LIKE 'sqlite_%' AND name NOT LIKE 'issue_feedback_%' ORDER BY name",
        )
        .unwrap()
        .query_map([], |row| row.get::<_, String>(0))
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap();
    tables
        .into_iter()
        .map(|table| {
            let mut statement = connection
                .prepare(&format!("SELECT * FROM {table}"))
                .unwrap();
            let columns = statement.column_count();
            let mut values = statement
                .query_map([], |row| {
                    (0..columns)
                        .map(|column| row.get::<_, Value>(column))
                        .collect()
                })
                .unwrap()
                .collect::<rusqlite::Result<Vec<Vec<Value>>>>()
                .unwrap();
            values.sort_by_key(|row| format!("{row:?}"));
            (table, values)
        })
        .collect()
}

#[test]
fn feedback_version_eight_migration_preserves_all_existing_rows_and_only_seeds_grant_facts() {
    let fixture = Fixture::new("feedback-migration-eight");
    let issue = seed(&fixture, "I_1");
    let adapter = fixture.adapter();
    let work = existing_work(&fixture, &issue);
    integrate(&fixture, &work);
    let pending = IssueAutonomyIntent {
        command_id: "migration-pending-autonomy".into(),
        level: IssueAutonomyLevel::Off,
        expected_revision: 0,
        policy_version: resolve_issue_automation_policy(Some(USER_POLICY), None)
            .unwrap()
            .version,
    };
    let pending = adapter
        .begin_issue_autonomy(fixture.cwd(), &pending)
        .unwrap();
    let original = current(&adapter, &issue);
    assert!(!original.first_grant_entered);
    assert!(original.autonomy.pending.is_some());

    let connection = Connection::open(fixture.database()).unwrap();
    make_version_eight(&connection);
    let old_schema = legacy_schema(&connection);
    let old_rows = legacy_rows(&connection);
    assert_eq!(
        super::super::super::schema_identity(&connection).unwrap(),
        (super::super::super::APPLICATION_ID, 8)
    );
    drop(connection);
    drop(adapter);

    let reopened = fixture.adapter();
    let migrated = current(&reopened, &issue);
    assert!(migrated.first_grant_entered);
    assert_eq!(migrated.collection, original.collection);
    assert_eq!(migrated.item, original.item);
    assert_eq!(migrated.work, original.work);
    assert_eq!(migrated.execution, original.execution);
    assert_eq!(migrated.autonomy, original.autonomy);
    assert_eq!(migrated.authorization_epoch, original.authorization_epoch);
    assert_eq!(migrated.state, original.state);
    assert_eq!(migrated.publication_attempts, 0);
    assert_eq!(
        reopened
            .get_issue_autonomy_command(fixture.cwd(), &pending.intent.command_id)
            .unwrap(),
        Some(pending)
    );
    for table in [
        "issue_feedback_state",
        "issue_feedback_intents",
        "issue_feedback_outbox",
    ] {
        assert_eq!(count(&fixture, table), 0, "{table}");
    }
    assert_eq!(count(&fixture, "issue_feedback_grant_entries"), 1);
    let connection = Connection::open(fixture.database()).unwrap();
    assert_eq!(
        super::super::super::schema_identity(&connection).unwrap(),
        (super::super::super::APPLICATION_ID, 9)
    );
    super::super::super::validate_schema(&connection).unwrap();
    assert_eq!(legacy_schema(&connection), old_schema);
    assert_eq!(legacy_rows(&connection), old_rows);
    for (name, expected) in super::super::schema::TABLES
        .into_iter()
        .chain(super::super::schema::INDEXES)
    {
        let sql: String = connection
            .query_row(
                "SELECT sql FROM sqlite_master WHERE name=?1",
                [name],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(
            super::super::super::normalized_schema(&sql),
            super::super::super::normalized_schema(expected),
            "{name}"
        );
    }
    drop(connection);
    drop(reopened);
    assert_eq!(current(&fixture.adapter(), &issue), migrated);
    assert_eq!(count(&fixture, "issue_feedback_grant_entries"), 1);
}

#[test]
fn feedback_version_eight_corrupt_legacy_work_rolls_back_schema_and_prior_seeded_markers() {
    let fixture = Fixture::new("feedback-migration-rollback");
    let first = seed(&fixture, "I_A");
    existing_work(&fixture, &first);
    let second = seed(&fixture, "I_Z");
    let bad_work = existing_work(&fixture, &second);
    let connection = Connection::open(fixture.database()).unwrap();
    make_version_eight(&connection);
    connection
        .execute(
            "UPDATE issue_work_checkpoints SET checkpoint='{}' WHERE work_order_id=?1",
            [&bad_work.order.work_order_id],
        )
        .unwrap();
    // The exact v8 schema passes. The sorted first checkpoint is valid, so
    // migration must create the tables and seed it before the second row fails.
    super::super::super::validate_schema_version(&connection, 8).unwrap();
    let old_rows = legacy_rows(&connection);
    let old_schema = legacy_schema(&connection);
    drop(connection);
    assert!(fixture.adapter().list_bindings(fixture.cwd()).is_err());
    let connection = Connection::open(fixture.database()).unwrap();
    assert_eq!(
        super::super::super::schema_identity(&connection).unwrap(),
        (super::super::super::APPLICATION_ID, 8)
    );
    super::super::super::validate_schema_version(&connection, 8).unwrap();
    assert_eq!(legacy_schema(&connection), old_schema);
    assert_eq!(legacy_rows(&connection), old_rows);
    assert_eq!(
        connection
            .query_row(
                "SELECT count(*) FROM sqlite_master WHERE name LIKE 'issue_feedback_%'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .unwrap(),
        0
    );
}

#[test]
fn feedback_current_schema_does_not_silently_recreate_a_missing_remote_unique_index() {
    let fixture = Fixture::new("feedback-current-schema-index");
    let issue = seed(&fixture, "I_1");
    let connection = Connection::open(fixture.database()).unwrap();
    connection
        .execute_batch("DROP INDEX issue_feedback_one_unresolved")
        .unwrap();
    drop(connection);
    assert!(fixture.adapter().snapshot(&issue).is_err());
    let connection = Connection::open(fixture.database()).unwrap();
    assert_eq!(
        connection
            .query_row(
                "SELECT count(*) FROM sqlite_master WHERE name='issue_feedback_one_unresolved'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .unwrap(),
        0
    );
    assert_eq!(
        super::super::super::schema_identity(&connection).unwrap(),
        (super::super::super::APPLICATION_ID, 9)
    );
}
