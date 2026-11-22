use super::super::super::triage;
use super::*;

fn submissions(fixture: &Fixture) -> Vec<(IssueWorkEvent, bool)> {
    fixture
        .adapter()
        .with_connection(fixture.cwd(), |connection, _| {
            let mut statement = connection.prepare(
                "SELECT event, acknowledged FROM issue_work_events
                 WHERE workspace_dir=?1 ORDER BY revision, event_id",
            )?;
            let rows = statement.query_map([fixture.cwd()], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, bool>(1)?))
            })?;
            let mut submissions = Vec::new();
            for row in rows {
                let (encoded, acknowledged) = row?;
                let event: IssueWorkEvent = serde_json::from_str(&encoded)?;
                if matches!(event.kind, IssueWorkEventKind::Submitted { .. }) {
                    submissions.push((event, acknowledged));
                }
            }
            Ok(submissions)
        })
        .unwrap()
}

fn catalog_revision(fixture: &Fixture) -> u64 {
    fixture
        .adapter()
        .with_connection(fixture.cwd(), |connection, _| {
            triage::catalog_revision(connection, fixture.cwd())
        })
        .unwrap()
}

#[test]
fn registration_observation_recovers_one_submission_and_never_reemits_after_ack() {
    let fixture = Fixture::new("execution-registration-ack-loss");
    let pending = seed_with_tasks(&fixture, Vec::new());
    let mut observed = IssueWorkExecutionObservation {
        task_ids: vec!["task-1".into()],
        active_task_id: Some("task-1".into()),
        ..Default::default()
    };
    let adapter = fixture.adapter();
    let first = saved(
        adapter
            .apply_issue_execution(fixture.cwd(), &pending, 0, &observed, Some(3))
            .unwrap(),
    );
    let linked = adapter
        .get_issue_work(fixture.cwd(), &pending.order.issue)
        .unwrap()
        .unwrap();
    assert_eq!(linked.task_ids, observed.task_ids);
    assert_eq!(linked.revision, pending.revision + 1);
    assert_eq!(first.progress.attempts.execution, 0);
    let original = submissions(&fixture);
    assert_eq!(original.len(), 1);
    let (submitted, acknowledged) = &original[0];
    assert!(!*acknowledged);
    assert_eq!(submitted.issue, pending.order.issue);
    assert_eq!(submitted.work_order_id, pending.order.work_order_id);
    assert_eq!(
        submitted.kind,
        IssueWorkEventKind::Submitted {
            task_ids: observed.task_ids.clone()
        }
    );
    assert_eq!(submitted.revision, catalog_revision(&fixture));
    drop(adapter);

    let reopened = fixture.adapter();
    let repeated = reopened
        .apply_issue_execution(
            fixture.cwd(),
            &linked,
            first.progress.revision,
            &observed,
            Some(3),
        )
        .unwrap();
    assert!(matches!(
        repeated,
        IssueExecutionSaveResult::Saved { changed: false, .. }
    ));
    assert_eq!(saved(repeated), first);
    assert_eq!(submissions(&fixture), original);

    reopened
        .acknowledge_issue_work_event(fixture.cwd(), &submitted.event_id)
        .unwrap();
    // A real execution update and later supplemental lineage must not recreate an ACKed submission.
    observed.phase = IssueExecutionPhase::Running;
    observed.execution_ids = vec!["task-1".into()];
    let running = saved(
        reopened
            .apply_issue_execution(
                fixture.cwd(),
                &linked,
                first.progress.revision,
                &observed,
                Some(3),
            )
            .unwrap(),
    );
    observed.task_ids.push("supplement-1".into());
    saved(
        reopened
            .apply_issue_execution(
                fixture.cwd(),
                &linked,
                running.progress.revision,
                &observed,
                Some(3),
            )
            .unwrap(),
    );
    assert_eq!(submissions(&fixture), vec![(submitted.clone(), true)]);
    assert!(
        reopened
            .pending_issue_work_events(fixture.cwd())
            .unwrap()
            .iter()
            .all(|event| !matches!(event.kind, IssueWorkEventKind::Submitted { .. }))
    );
}

#[test]
fn registration_observation_does_not_backfill_an_existing_work_lineage() {
    let fixture = Fixture::new("execution-existing-registration");
    let work = seed(&fixture);
    let mut observed = running(&work);
    let adapter = fixture.adapter();
    let first = saved(
        adapter
            .apply_issue_execution(fixture.cwd(), &work, 0, &observed, Some(3))
            .unwrap(),
    );
    assert!(submissions(&fixture).is_empty());
    observed.task_ids.push("supplement-1".into());
    saved(
        adapter
            .apply_issue_execution(
                fixture.cwd(),
                &work,
                first.progress.revision,
                &observed,
                Some(3),
            )
            .unwrap(),
    );
    assert_eq!(
        adapter
            .get_issue_work(fixture.cwd(), &work.order.issue)
            .unwrap()
            .unwrap()
            .task_ids,
        observed.task_ids
    );
    assert!(submissions(&fixture).is_empty());
}

#[test]
fn registration_recovery_rolls_back_lineage_and_receipts_when_catalog_is_exhausted() {
    let fixture = Fixture::new("execution-registration-rollback");
    let pending = seed_with_tasks(&fixture, Vec::new());
    let mut observed = running(&pending);
    observed.task_ids = vec!["task-1".into()];
    let adapter = fixture.adapter();
    let tracked = adapter
        .get_issue(fixture.cwd(), &pending.order.issue)
        .unwrap()
        .unwrap();
    let original_revision = catalog_revision(&fixture);
    // Force a failure after receipts, progress and checkpoint writes without injecting an event.
    adapter
        .with_connection(fixture.cwd(), |connection, _| {
            assert_eq!(
                connection.execute(
                    "UPDATE issue_automation_catalog SET revision=?1 WHERE workspace_dir=?2",
                    params![i64::MAX, fixture.cwd()],
                )?,
                1
            );
            Ok(())
        })
        .unwrap();
    let error = adapter
        .apply_issue_execution(fixture.cwd(), &pending, 0, &observed, Some(3))
        .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("the issue automation revision exceeds SQLite's range")
    );
    drop(adapter);

    let reopened = fixture.adapter();
    assert_eq!(
        reopened
            .get_issue_work(fixture.cwd(), &pending.order.issue)
            .unwrap(),
        Some(pending.clone())
    );
    assert!(
        reopened
            .get_issue_execution(fixture.cwd(), &pending.order.issue)
            .unwrap()
            .is_none()
    );
    assert_eq!(
        reopened
            .get_issue(fixture.cwd(), &pending.order.issue)
            .unwrap()
            .unwrap()
            .to_record(),
        tracked.to_record()
    );
    assert_eq!(catalog_revision(&fixture), u64::try_from(i64::MAX).unwrap());
    assert!(
        reopened
            .pending_issue_work_events(fixture.cwd())
            .unwrap()
            .is_empty()
    );
    let receipt_count: i64 = reopened
        .with_connection(fixture.cwd(), |connection, _| {
            Ok(connection.query_row(
                "SELECT count(*) FROM issue_execution_receipts WHERE workspace_dir=?1",
                [fixture.cwd()],
                |row| row.get(0),
            )?)
        })
        .unwrap();
    assert_eq!(receipt_count, 0);

    reopened
        .with_connection(fixture.cwd(), |connection, _| {
            connection.execute(
                "UPDATE issue_automation_catalog SET revision=?1 WHERE workspace_dir=?2",
                params![sql_revision(original_revision)?, fixture.cwd()],
            )?;
            Ok(())
        })
        .unwrap();
    let recovered = saved(
        reopened
            .apply_issue_execution(fixture.cwd(), &pending, 0, &observed, Some(3))
            .unwrap(),
    );
    assert_eq!(recovered.progress.attempts.execution, 1);
    assert_eq!(
        reopened
            .get_issue_work(fixture.cwd(), &pending.order.issue)
            .unwrap()
            .unwrap()
            .task_ids,
        observed.task_ids
    );
    assert_eq!(submissions(&fixture).len(), 1);
}
