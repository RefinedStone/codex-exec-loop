use super::*;

#[test]
fn version_nine_migration_preserves_legacy_human_work_without_inventing_daily_start_times() {
    let fixture = Fixture::new("admission-migration-nine");
    let budget = budget(1, 1);
    let prepared = prepare(
        &fixture,
        "I_legacy",
        IssueWorkStartAuthority::Human,
        &budget,
    );
    let work = saved(run(&fixture, &prepared, &budget, MILLIS_PER_UTC_DAY));
    let selection = fixture
        .adapter()
        .issue_autonomy_selection(fixture.cwd())
        .unwrap();
    let connection = Connection::open(fixture.database()).unwrap();
    let mut legacy = serde_json::to_value(&work).unwrap();
    legacy.as_object_mut().unwrap().remove("startAuthority");
    let encoded = serde_json::to_string(&legacy).unwrap();
    connection.execute(
        "UPDATE issue_work_checkpoints SET checkpoint=?1 WHERE workspace_dir=?2 AND work_order_id=?3",
        params![encoded, fixture.cwd(), work.order.work_order_id],
    ).unwrap();
    super::super::super::tests::make_pre_admission_schema(&connection);
    connection.pragma_update(None, "user_version", 9).unwrap();
    super::super::super::validate_schema_version(&connection, 9).unwrap();
    drop(connection);

    let reopened = fixture.adapter();
    assert_eq!(
        reopened
            .get_issue_work(fixture.cwd(), &work.order.issue)
            .unwrap(),
        Some(work.clone())
    );
    assert_eq!(
        reopened.issue_autonomy_selection(fixture.cwd()).unwrap(),
        selection
    );
    let connection = Connection::open(fixture.database()).unwrap();
    assert_eq!(
        super::super::super::schema_identity(&connection).unwrap(),
        (
            super::super::super::APPLICATION_ID,
            super::super::super::SCHEMA_VERSION
        )
    );
    super::super::super::validate_schema(&connection).unwrap();
    let unchanged: String = connection.query_row(
        "SELECT checkpoint FROM issue_work_checkpoints WHERE workspace_dir=?1 AND work_order_id=?2",
        params![fixture.cwd(), work.order.work_order_id], |row| row.get(0),
    ).unwrap();
    assert_eq!(
        unchanged, encoded,
        "migration does not rewrite legacy authorization evidence"
    );
    drop(connection);
    assert_eq!(
        count(&fixture, "issue_work_admissions"),
        0,
        "v9 has no original admission timestamps"
    );
    let new = prepare(&fixture, "I_new", IssueWorkStartAuthority::Human, &budget);
    assert!(
        matches!(run(&fixture, &new, &budget, 2 * MILLIS_PER_UTC_DAY),
            IssueWorkAdmissionResult::BudgetExhausted { wait, .. } if wait.exhausted == vec![IssueStartBudgetLimit::Running]
        ),
        "legacy nonterminal checkpoints still reserve running capacity"
    );
    transition(&fixture, &prepared, IssueTransition::Cancel);
    saved(run(&fixture, &new, &budget, 2 * MILLIS_PER_UTC_DAY));
    assert_eq!(count(&fixture, "issue_work_admissions"), 1);
}
