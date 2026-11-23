use super::*;

#[test]
fn arbiter_simultaneous_claims_share_one_running_or_daily_reservation() {
    for (running, daily) in [(1, 5), (5, 1)] {
        let fixture = Fixture::new("arbiter-budget-race");
        let first = prepare_unanswered_arbiter(&fixture, "I_first", &budget(5, 5));
        let second = prepare_unanswered_arbiter(&fixture, "I_second", &budget(5, 5));
        let barrier = Arc::new(Barrier::new(2));
        let handles: Vec<_> = [first, second]
            .iter()
            .map(|prepared| {
                let mut request = claim_request(&fixture, prepared, DUE);
                request.settings.max_running = running;
                request.settings.max_started_per_day = daily;
                let adapter = fixture.adapter();
                let workspace = fixture.cwd().to_owned();
                let barrier = barrier.clone();
                std::thread::spawn(move || {
                    barrier.wait();
                    adapter.claim_issue_arbiter(&workspace, &request).unwrap()
                })
            })
            .collect();
        let results: Vec<_> = handles
            .into_iter()
            .map(|handle| handle.join().unwrap())
            .collect();
        assert_eq!(
            results
                .iter()
                .filter(|result| matches!(result, IssueArbiterClaimResult::Claimed(_)))
                .count(),
            1
        );
        assert_eq!(results.iter().filter(|result| matches!(result, IssueArbiterClaimResult::Deferred(commit)
            if commit.event.outcome == (IssueArbiterFinishedOutcome::Waiting { reason: IssueArbiterWaitReason::BudgetExhausted }))).count(), 1);
        assert_eq!(count(&fixture, "issue_arbiter_attempts"), 1);
        assert_eq!(count(&fixture, "issue_arbiter_finished_events"), 1);
    }
}

#[test]
fn arbiter_daily_reservations_are_not_refunded_and_clock_rollback_cannot_reopen_a_day() {
    const DAY: u64 = 86_400_000;
    let fixture = Fixture::new("arbiter-clock-highwater");
    let prepared: Vec<_> = ["I_first", "I_same_day", "I_tomorrow", "I_clock_rollback"]
        .into_iter()
        .map(|id| prepare_unanswered_arbiter(&fixture, id, &budget(5, 5)))
        .collect();
    let requests: Vec<_> = prepared
        .iter()
        .zip([2 * DAY + DUE, 2 * DAY + DUE + 5, 3 * DAY + DUE, DAY + DUE])
        .map(|(prepared, at)| {
            let mut request = claim_request(&fixture, prepared, at);
            request.settings.max_running = 5;
            request.settings.max_started_per_day = 1;
            request
        })
        .collect();
    let first = start(&fixture, &requests[0]);
    let mut unknown = finish_request(&first, &requests[0]);
    unknown.now_unix_ms = first.deadline_at_unix_ms;
    assert_eq!(
        finished(&fixture, &unknown).event.outcome,
        IssueArbiterFinishedOutcome::Waiting {
            reason: IssueArbiterWaitReason::OutcomeUnknown
        }
    );
    assert!(matches!(
        fixture
            .adapter()
            .claim_issue_arbiter(fixture.cwd(), &requests[1])
            .unwrap(),
        IssueArbiterClaimResult::Deferred(_)
    ));
    let tomorrow = start(&fixture, &requests[2]);
    assert_eq!(tomorrow.utc_day, 3);
    finished(&fixture, &finish_request(&tomorrow, &requests[2]));
    assert!(matches!(
        fixture
            .adapter()
            .claim_issue_arbiter(fixture.cwd(), &requests[3])
            .unwrap(),
        IssueArbiterClaimResult::Deferred(_)
    ));
    assert_eq!(count(&fixture, "issue_arbiter_attempts"), 2);
    let current_day: i64 = Connection::open(fixture.database())
        .unwrap()
        .query_row(
            "SELECT utc_day FROM issue_arbiter_budget_clock",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(current_day, 3);
    let mut recovered_budget = requests[1].clone();
    recovered_budget.now_unix_ms = 4 * DAY + DUE;
    assert!(
        matches!(
            fixture
                .adapter()
                .claim_issue_arbiter(fixture.cwd(), &recovered_budget)
                .unwrap(),
            IssueArbiterClaimResult::Deferred(_)
        ),
        "a previously deferred decision never starts a delayed automatic retry"
    );
}

#[test]
fn arbiter_pending_pagination_includes_a_decision_already_answered_by_a_human() {
    let fixture = Fixture::new("arbiter-pending-pagination");
    let first = prepare_unanswered_arbiter(&fixture, "I_pending_a", &budget(5, 5));
    let second = prepare_unanswered_arbiter(&fixture, "I_pending_b", &budget(5, 5));
    let mut expected = Vec::new();
    for prepared in [&first, &second] {
        let mut request = claim_request(&fixture, prepared, DUE);
        request.settings.max_running = 2;
        expected.push(start(&fixture, &request));
    }
    save_human(&fixture, &first);
    expected.sort_by(|left, right| left.attempt_id.cmp(&right.attempt_id));
    let adapter = fixture.adapter();
    let page = adapter
        .unfinished_issue_arbiter_attempts(fixture.cwd(), None, 1)
        .unwrap();
    assert_eq!(page, expected[..1]);
    assert_eq!(
        adapter
            .unfinished_issue_arbiter_attempts(fixture.cwd(), Some(&page[0].attempt_id), 1)
            .unwrap(),
        expected[1..]
    );
    assert!(
        adapter
            .unfinished_issue_arbiter_attempts(fixture.cwd(), Some(&expected[1].attempt_id), 256)
            .unwrap()
            .is_empty()
    );
    assert!(
        adapter
            .unfinished_issue_arbiter_attempts(fixture.cwd(), None, 0)
            .unwrap()
            .is_empty()
    );
    let id = first.work.decision_id.as_ref().unwrap().clone();
    assert!(
        adapter
            .issue_decision_arbiter_waits(fixture.cwd(), &[id.clone(), id])
            .is_err()
    );
}

#[test]
fn arbiter_finish_rolls_back_the_answer_and_attempt_when_the_catalog_cannot_advance() {
    let fixture = Fixture::new("arbiter-transaction-rollback");
    let prepared = prepare_unanswered_arbiter(&fixture, "I_atomic", &budget(1, 5));
    let original = decision(&fixture, &prepared);
    let claim = claim_request(&fixture, &prepared, DUE);
    let attempt = start(&fixture, &claim);
    let catalog = fixture
        .adapter()
        .issue_decision_arbiter_waits(fixture.cwd(), &[])
        .unwrap()
        .catalog_revision;
    let connection = Connection::open(fixture.database()).unwrap();
    connection
        .execute(
            "UPDATE issue_automation_catalog SET revision=?1",
            [i64::MAX],
        )
        .unwrap();
    assert!(
        fixture
            .adapter()
            .finish_issue_arbiter(fixture.cwd(), &finish_request(&attempt, &claim))
            .is_err()
    );
    assert_eq!(decision(&fixture, &prepared), original);
    assert_eq!(
        fixture
            .adapter()
            .issue_decision_arbiter_waits(fixture.cwd(), &[])
            .unwrap()
            .catalog_revision,
        i64::MAX as u64
    );
    assert_eq!(
        fixture
            .adapter()
            .unfinished_issue_arbiter_attempts(fixture.cwd(), None, 256)
            .unwrap(),
        vec![attempt.clone()]
    );
    assert_eq!(count(&fixture, "issue_arbiter_finished_events"), 0);
    connection
        .execute(
            "UPDATE issue_automation_catalog SET revision=?1",
            [i64::try_from(catalog).unwrap()],
        )
        .unwrap();
    assert_eq!(
        finished(&fixture, &finish_request(&attempt, &claim))
            .event
            .outcome,
        IssueArbiterFinishedOutcome::Applied {}
    );
}

#[test]
fn arbiter_schema_eleven_migration_preserves_existing_decisions_without_creating_calls() {
    let fixture = Fixture::new("arbiter-schema-eleven");
    let prepared = prepare_unanswered_default(&fixture, "I_existing", &budget(1, 5));
    let original = decision(&fixture, &prepared);
    let before = fixture
        .adapter()
        .read_triage_batch(fixture.cwd(), None, 100)
        .unwrap();
    let selection = fixture
        .adapter()
        .issue_autonomy_selection(fixture.cwd())
        .unwrap();
    let connection = Connection::open(fixture.database()).unwrap();
    super::super::super::super::tests::make_pre_arbiter_schema(&connection);
    connection.pragma_update(None, "user_version", 11).unwrap();
    drop(connection);
    let reopened = fixture.adapter();
    assert!(
        reopened
            .unfinished_issue_arbiter_attempts(fixture.cwd(), None, 256)
            .unwrap()
            .is_empty()
    );
    assert!(
        reopened
            .pending_issue_arbiter_events(fixture.cwd(), 256)
            .unwrap()
            .is_empty()
    );
    assert_eq!(decision(&fixture, &prepared), original);
    assert_eq!(
        reopened
            .read_triage_batch(fixture.cwd(), None, 100)
            .unwrap(),
        before
    );
    assert_eq!(
        reopened.issue_autonomy_selection(fixture.cwd()).unwrap(),
        selection
    );
    let version: i64 = Connection::open(fixture.database())
        .unwrap()
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .unwrap();
    assert_eq!(version, 12);
    assert_eq!(count(&fixture, "issue_arbiter_attempts"), 0);
}
