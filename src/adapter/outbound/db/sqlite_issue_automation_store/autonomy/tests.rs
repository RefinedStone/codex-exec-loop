use std::sync::{Arc, Barrier};

use super::super::tests::Fixture;
use super::*;
use crate::application::port::outbound::issue_automation_store_port::IssueAutomationStorePort;
use crate::domain::issue_automation::autonomy::IssueAutonomyLevel;
mod auto;

fn intent(id: &str, level: IssueAutonomyLevel, revision: u64) -> IssueAutonomyIntent {
    IssueAutonomyIntent {
        command_id: id.into(),
        level,
        expected_revision: revision,
        policy_version: "a".repeat(64),
    }
}

#[test]
fn pending_selection_fences_old_assessments_and_retry_is_idempotent() {
    let fixture = Fixture::new("autonomy-fence");
    let adapter = fixture.adapter();
    let command = intent("off-1", IssueAutonomyLevel::Off, 0);
    assert_eq!(
        adapter.issue_autonomy_selection(fixture.cwd()).unwrap(),
        IssueAutonomySelection::default()
    );
    adapter
        .with_connection(fixture.cwd(), |connection, _| {
            assert!(permits_assessment(connection, fixture.cwd(), 0)?);
            Ok(())
        })
        .unwrap();
    let begun = adapter
        .begin_issue_autonomy(fixture.cwd(), &command)
        .unwrap();
    assert_eq!(begun.completed_revision, None);
    assert_eq!(
        adapter
            .begin_issue_autonomy(fixture.cwd(), &command)
            .unwrap(),
        begun
    );
    adapter
        .with_connection(fixture.cwd(), |connection, _| {
            assert!(!permits_assessment(connection, fixture.cwd(), 0)?);
            Ok(())
        })
        .unwrap();

    let first = adapter
        .finish_issue_autonomy(fixture.cwd(), &command)
        .unwrap();
    assert!(first.changed);
    assert_eq!(first.command.completed_revision, Some(1));
    let replay = adapter
        .finish_issue_autonomy(fixture.cwd(), &command)
        .unwrap();
    assert!(!replay.changed);
    assert_eq!(replay.command, first.command);
    assert_eq!(
        adapter.issue_autonomy_selection(fixture.cwd()).unwrap(),
        IssueAutonomySelection {
            requested_level: Some(IssueAutonomyLevel::Off),
            revision: 1,
            pending: None,
        }
    );
    adapter
        .with_connection(fixture.cwd(), |connection, _| {
            assert!(!permits_assessment(connection, fixture.cwd(), 0)?);
            assert!(permits_assessment(connection, fixture.cwd(), 1)?);
            Ok(())
        })
        .unwrap();

    let same_level = intent("off-2", IssueAutonomyLevel::Off, 1);
    let noop = adapter
        .begin_issue_autonomy(fixture.cwd(), &same_level)
        .unwrap();
    assert_eq!(noop.completed_revision, Some(1));
    assert!(
        !adapter
            .finish_issue_autonomy(fixture.cwd(), &same_level)
            .unwrap()
            .changed
    );
    assert_eq!(
        adapter
            .issue_autonomy_selection(fixture.cwd())
            .unwrap()
            .revision,
        1
    );
    assert!(
        adapter
            .begin_issue_autonomy(
                fixture.cwd(),
                &intent("off-1", IssueAutonomyLevel::Observe, 0)
            )
            .is_err()
    );
    assert!(
        adapter
            .begin_issue_autonomy(
                fixture.cwd(),
                &intent("stale", IssueAutonomyLevel::Observe, 0)
            )
            .is_err()
    );
}

#[test]
fn two_connections_cannot_admit_different_commands_at_the_same_revision() {
    let fixture = Fixture::new("autonomy-competing");
    fixture
        .adapter()
        .issue_autonomy_selection(fixture.cwd())
        .unwrap();
    let barrier = Arc::new(Barrier::new(2));
    let handles: Vec<_> = ["first", "second"]
        .into_iter()
        .map(|id| {
            let adapter = fixture.adapter();
            let workspace = fixture.cwd().to_string();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                barrier.wait();
                adapter.begin_issue_autonomy(&workspace, &intent(id, IssueAutonomyLevel::Off, 0))
            })
        })
        .collect();
    let accepted: Vec<_> = handles
        .into_iter()
        .map(|handle| handle.join().unwrap())
        .filter_map(Result::ok)
        .collect();
    assert_eq!(accepted.len(), 1);
    assert_eq!(
        fixture
            .adapter()
            .issue_autonomy_selection(fixture.cwd())
            .unwrap()
            .pending,
        Some(accepted[0].intent.clone())
    );
}

#[test]
fn concurrent_finish_reports_one_committed_change() {
    let fixture = Fixture::new("autonomy-finish");
    let command = intent("finish", IssueAutonomyLevel::Observe, 0);
    fixture
        .adapter()
        .begin_issue_autonomy(fixture.cwd(), &command)
        .unwrap();
    let barrier = Arc::new(Barrier::new(2));
    let handles: Vec<_> = (0..2)
        .map(|_| {
            let adapter = fixture.adapter();
            let workspace = fixture.cwd().to_string();
            let command = command.clone();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                barrier.wait();
                adapter.finish_issue_autonomy(&workspace, &command).unwrap()
            })
        })
        .collect();
    let results: Vec<_> = handles
        .into_iter()
        .map(|handle| handle.join().unwrap())
        .collect();
    assert_eq!(results.iter().filter(|result| result.changed).count(), 1);
    assert!(
        results
            .iter()
            .all(|result| result.command.completed_revision == Some(1))
    );
}

#[test]
fn version_six_migration_preserves_tracked_issues_and_has_no_override() {
    let fixture = Fixture::new("autonomy-migration");
    let (binding, issue) = fixture.insert();
    {
        let connection = Connection::open(fixture.database()).unwrap();
        super::super::tests::make_pre_admission_schema(&connection);
        connection.execute_batch("DROP TABLE issue_feedback_outbox; DROP TABLE issue_feedback_intents; DROP TABLE issue_feedback_state; DROP TABLE issue_feedback_grant_entries; DROP TABLE issue_execution_answers; DROP TABLE issue_execution_receipts; DROP TABLE issue_execution_progress; DROP TABLE issue_autonomy_work_fences; DROP TABLE issue_autonomy_commands; DROP TABLE issue_autonomy_selection; PRAGMA user_version=6;").unwrap();
    }
    let adapter = fixture.adapter();
    assert_eq!(
        adapter.issue_autonomy_selection(fixture.cwd()).unwrap(),
        IssueAutonomySelection::default()
    );
    assert_eq!(
        adapter.get_issue(fixture.cwd(), issue.reference()).unwrap(),
        Some(issue)
    );
    assert_eq!(
        adapter.list_bindings(fixture.cwd()).unwrap()[0].binding,
        binding
    );
    let connection = Connection::open(fixture.database()).unwrap();
    let version: i64 = connection
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .unwrap();
    assert_eq!(version, super::super::SCHEMA_VERSION);
}

#[test]
fn fresh_schema_requires_the_owned_autonomy_fence_table() {
    let fixture = Fixture::new("autonomy-fence-schema");
    fixture
        .adapter()
        .issue_autonomy_selection(fixture.cwd())
        .unwrap();
    let connection = Connection::open(fixture.database()).unwrap();
    let present: i64 = connection.query_row(
        "SELECT count(*) FROM sqlite_master WHERE type='table' AND name='issue_autonomy_work_fences'",
        [], |row| row.get(0),
    ).unwrap();
    assert_eq!(present, 1);
    connection
        .execute_batch("DROP TABLE issue_autonomy_work_fences;")
        .unwrap();
    drop(connection);
    assert!(
        fixture
            .adapter()
            .issue_autonomy_selection(fixture.cwd())
            .is_err()
    );
}
