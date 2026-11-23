use super::*;

#[test]
fn failure_limit_replay_keeps_its_first_decision_after_new_facts_and_human_resume() {
    let f = fixture("host-failure-replay");
    let first = fail(&f, "first");
    let below = apply(&f, &first);
    let first_marker = marker(&f, &first);
    assert_eq!(
        below.outcome,
        TaskVerificationFailureLimitOutcome::BelowLimit
    );
    fail(&f, "second");
    let third = fail(&f, "third");
    assert_eq!(apply(&f, &first), below);
    assert_eq!(marker(&f, &first), first_marker);
    assert_ne!(task(&f).status, TaskStatus::AwaitingUser);
    let paused = apply(&f, &third);
    assert!(paused.changed);
    let original_marker = marker(&f, &third);
    let mut resumed = task(&f);
    resumed.status = TaskStatus::InProgress;
    resumed.last_updated_by = TaskActor::User;
    resumed.updated_at = "2026-11-23T12:00:01Z".into();
    f.base.save(vec![resumed.clone()]);
    let revision = planning_revision(&f);
    let fresh = "f".repeat(64);
    f.base
        .adapter
        .recover_incomplete_host_verifications(&f.base.workspace, &fresh, NOW)
        .unwrap();
    let mut replay_request = request(&f, &third);
    replay_request.owner_token = fresh;
    replay_request.at = "2026-11-23T10:00:00Z".into();
    let replay = f
        .base
        .adapter
        .apply_host_verification_failure_limit(&f.base.workspace, &replay_request)
        .unwrap();
    assert_eq!(
        replay.outcome, paused.outcome,
        "the outcome is the historical decision, not current authority"
    );
    assert_eq!(replay.failure_count, paused.failure_count);
    assert_eq!(replay.planning_revision, paused.planning_revision);
    assert!(!replay.changed);
    assert_eq!(task(&f), resumed);
    assert_eq!(planning_revision(&f), revision);
    assert_eq!(marker(&f, &third), original_marker);
    let applications: i64 = authority_connection(&f.base.workspace).query_row(
        "SELECT count(*) FROM runtime_events WHERE event_kind='task_verification_failure_limit_applied'",
        [], |row| row.get(0),
    ).unwrap();
    assert_eq!(applications, 1);
}

#[test]
fn failure_limit_uses_the_trigger_cutoff_even_on_its_first_late_application() {
    let f = fixture("host-failure-cutoff");
    let receipts = three_failures(&f);
    let first = apply(&f, &receipts[0]);
    assert_eq!(first.failure_count, 1);
    assert_eq!(
        first.outcome,
        TaskVerificationFailureLimitOutcome::BelowLimit
    );
    assert_ne!(task(&f).status, TaskStatus::AwaitingUser);
    assert_eq!(apply(&f, &receipts[0]), first);
    assert_eq!(apply(&f, &receipts[2]).failure_count, 3);
}

#[test]
fn failure_limit_task_session_marker_and_audit_roll_back_without_losing_terminal_fact() {
    for fail_at in ["session", "task", "marker", "event"] {
        let f = fixture(&format!("host-failure-atomic-{fail_at}"));
        let receipts = three_failures(&f);
        let receipt = &receipts[2];
        let before_task = task(&f);
        let before_session = session_detail(&f);
        let before_revision = planning_revision(&f);
        let before_counters = f.base.counters();
        let before_events = f
            .base
            .adapter
            .pending_events(&f.base.workspace, 100)
            .unwrap();
        let sql = match fail_at {
            "session" => {
                "CREATE TRIGGER fail_failure_limit BEFORE UPDATE ON runtime_session_details BEGIN SELECT RAISE(ABORT,'session write failed'); END;"
            }
            "task" => {
                "CREATE TRIGGER fail_failure_limit BEFORE INSERT ON planning_task_mutation_events BEGIN SELECT RAISE(ABORT,'task write failed'); END;"
            }
            "marker" => {
                "CREATE TRIGGER fail_failure_limit BEFORE UPDATE OF failure_limit_application_json ON task_verification_runs BEGIN SELECT RAISE(ABORT,'marker write failed'); END;"
            }
            _ => {
                "CREATE TRIGGER fail_failure_limit BEFORE INSERT ON runtime_events WHEN NEW.event_kind='task_verification_failure_limit_applied' BEGIN SELECT RAISE(ABORT,'audit write failed'); END;"
            }
        };
        authority_connection(&f.base.workspace)
            .execute_batch(sql)
            .unwrap();
        assert!(
            f.base
                .adapter
                .apply_host_verification_failure_limit(&f.base.workspace, &request(&f, receipt))
                .is_err()
        );
        assert_eq!(task(&f), before_task);
        assert_eq!(session_detail(&f), before_session);
        assert_eq!(planning_revision(&f), before_revision);
        assert_eq!(f.base.counters(), before_counters);
        assert_eq!(
            f.base
                .adapter
                .pending_events(&f.base.workspace, 100)
                .unwrap(),
            before_events
        );
        assert_eq!(
            f.base
                .adapter
                .load_host_verification(&f.base.workspace, &receipt.attempt.attempt_id)
                .unwrap()
                .unwrap()
                .receipt
                .as_ref(),
            Some(receipt)
        );
        assert!(marker(&f, receipt).is_none());
        authority_connection(&f.base.workspace)
            .execute_batch("DROP TRIGGER fail_failure_limit")
            .unwrap();
        let retry = apply(&f, receipt);
        assert!(retry.changed);
        assert_eq!(retry.failure_count, 3);
        assert_eq!(planning_revision(&f), before_revision + 1);
        assert_eq!(f.base.counters(), before_counters);
        let waiting = session_detail(&f);
        assert_eq!(waiting.state_label, "awaiting_user");
        assert_eq!(waiting.completion_state_label, "awaiting_user");
    }
}

#[test]
fn concurrent_failure_limit_requests_change_the_task_once() {
    let f = fixture("host-failure-concurrent");
    let receipts = three_failures(&f);
    let request = request(&f, &receipts[2]);
    let before = planning_revision(&f);
    let barrier = Arc::new(Barrier::new(2));
    let handles = (0..2)
        .map(|_| {
            let workspace = f.base.workspace.clone();
            let request = request.clone();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                barrier.wait();
                SqlitePlanningAuthorityAdapter::new()
                    .apply_host_verification_failure_limit(&workspace, &request)
            })
        })
        .collect::<Vec<_>>();
    let results = handles
        .into_iter()
        .map(|handle| handle.join().unwrap().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(results.iter().filter(|result| result.changed).count(), 1);
    assert!(results.iter().all(|result| result.outcome
        == TaskVerificationFailureLimitOutcome::AwaitingUser
        && result.failure_count == 3));
    assert_eq!(planning_revision(&f), before + 1);
}

#[test]
fn legacy_host_schema_adds_only_private_policy_storage_and_preserves_receipts() {
    let f = fixture("host-failure-schema");
    let receipts = three_failures(&f);
    let counters = f.base.counters();
    let before = f
        .base
        .adapter
        .pending_events(&f.base.workspace, 100)
        .unwrap();
    let connection = authority_connection(&f.base.workspace);
    connection
        .execute_batch(
            "ALTER TABLE task_verification_runs DROP COLUMN failure_limit_application_json",
        )
        .unwrap();
    let schema: String = connection
        .query_row(
            "SELECT sql FROM sqlite_master WHERE name='task_verification_runs'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert!(!schema.contains("failure_limit_application_json"));
    for receipt in &receipts {
        let stored = f
            .base
            .adapter
            .load_host_verification(&f.base.workspace, &receipt.attempt.attempt_id)
            .unwrap()
            .unwrap();
        assert_eq!(stored.receipt.as_ref(), Some(receipt));
        assert!(marker(&f, receipt).is_none());
    }
    assert_eq!(f.base.counters(), counters);
    assert_eq!(
        f.base
            .adapter
            .pending_events(&f.base.workspace, 100)
            .unwrap(),
        before
    );
    assert_ne!(task(&f).status, TaskStatus::AwaitingUser);
    let columns: i64 = connection.query_row("SELECT count(*) FROM pragma_table_info('task_verification_runs') WHERE name='failure_limit_application_json'", [], |row| row.get(0)).unwrap();
    assert_eq!(columns, 1);
    assert!(apply(&f, &receipts[2]).changed);
}
