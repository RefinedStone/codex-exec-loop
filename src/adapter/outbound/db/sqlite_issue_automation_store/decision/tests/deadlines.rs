use super::*;
use crate::domain::issue_automation::policy::IssueAutomationTimeouts;

const NOW: u64 = 1_800_000_000_000;
const HOUR: u64 = 3_600_000;

fn timeouts(remind_before_seconds: u64) -> IssueAutomationTimeouts {
    IssueAutomationTimeouts {
        start_seconds: 24 * 60 * 60,
        remind_before_seconds,
    }
}

fn timed(fixture: &Fixture) -> (IssueTriageFence, IssueTriageAssessment, IssueDecisionCommit) {
    let (fence, assessment, request) = prepared(fixture);
    let opened = commit(
        fixture
            .adapter()
            .ensure_issue_decision_timed(
                fixture.cwd(),
                fence,
                &assessment,
                &request,
                NOW,
                &timeouts(2 * 60 * 60),
            )
            .unwrap(),
    );
    (fence, assessment, opened)
}

fn decision_fence(triage: IssueTriageFence, opened: &IssueDecisionCommit) -> IssueDecisionFence {
    IssueDecisionFence {
        triage,
        decision_revision: opened.record.revision,
    }
}

#[test]
fn reminder_boundary_restart_and_ack_preserve_one_receipt_and_the_decision_revision() {
    let fixture = Fixture::new("decision-reminder-restart");
    let (triage, assessment, opened) = timed(&fixture);
    let expected = decision_fence(triage, &opened);
    let id = opened.record.request.id();
    let adapter = fixture.adapter();
    assert!(
        adapter
            .remind_issue_decision(
                fixture.cwd(),
                expected,
                &assessment,
                id,
                NOW + 22 * HOUR - 1
            )
            .unwrap()
            .is_none()
    );
    let event = adapter
        .remind_issue_decision(fixture.cwd(), expected, &assessment, id, NOW + 22 * HOUR)
        .unwrap()
        .unwrap();
    assert!(event.event_id.starts_with("issue-decision-reminder:"));
    assert_eq!(event.fact, opened.record.request.fact());
    assert_eq!(event.revision, opened.catalog_revision + 1);
    assert_eq!(event.fact.state, IssueDecisionState::Open);
    drop(adapter);

    let reopened = fixture.adapter();
    assert_eq!(
        reopened
            .pending_issue_decision_reminders(fixture.cwd(), 10)
            .unwrap(),
        vec![event.clone()],
        "a crash before ACK replays the original event identity"
    );
    assert!(
        reopened
            .remind_issue_decision(fixture.cwd(), expected, &assessment, id, NOW + 23 * HOUR)
            .unwrap()
            .is_none()
    );
    for _ in 0..2 {
        reopened
            .acknowledge_issue_decision_reminder(fixture.cwd(), &event.event_id)
            .unwrap();
    }
    assert!(
        reopened
            .pending_issue_decision_reminders(fixture.cwd(), 10)
            .unwrap()
            .is_empty()
    );
    let deadline = reopened
        .issue_decision_deadlines(fixture.cwd(), None, 10)
        .unwrap()
        .remove(0);
    assert!(deadline.reminded);
    assert_eq!(deadline.record, opened.record);
    let current = reopened
        .get_issue_decision_commit(fixture.cwd(), id)
        .unwrap()
        .unwrap();
    assert_eq!(current.record, opened.record);
    assert_eq!(current.catalog_revision, event.revision);
    assert!(
        fixture
            .adapter()
            .remind_issue_decision(fixture.cwd(), expected, &assessment, id, NOW + 23 * HOUR)
            .unwrap()
            .is_none(),
        "ACK keeps the receipt and cannot reopen the reminder"
    );
}

#[test]
fn simultaneous_reminders_commit_one_event_and_one_catalog_change() {
    let fixture = Fixture::new("decision-reminder-race");
    let (triage, assessment, opened) = timed(&fixture);
    let expected = decision_fence(triage, &opened);
    let barrier = Arc::new(Barrier::new(3));
    let mut threads = Vec::new();
    for _ in 0..2 {
        let adapter = fixture.adapter();
        let workspace = fixture.cwd().to_string();
        let current = assessment.clone();
        let id = opened.record.request.id().clone();
        let barrier = barrier.clone();
        threads.push(std::thread::spawn(move || {
            barrier.wait();
            adapter
                .remind_issue_decision(&workspace, expected, &current, &id, NOW + 22 * HOUR)
                .unwrap()
        }));
    }
    barrier.wait();
    let events: Vec<_> = threads
        .into_iter()
        .filter_map(|thread| thread.join().unwrap())
        .collect();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].revision, opened.catalog_revision + 1);
    assert_eq!(
        fixture
            .adapter()
            .pending_issue_decision_reminders(fixture.cwd(), 10)
            .unwrap(),
        events
    );
    assert_eq!(
        fixture
            .adapter()
            .get_issue_decision(fixture.cwd(), opened.record.request.id())
            .unwrap(),
        Some(opened.record)
    );
}

#[test]
fn overdue_answered_and_stale_decisions_do_not_generate_reminders() {
    let fixture = Fixture::new("decision-reminder-fences");
    let (triage, assessment, opened) = timed(&fixture);
    let expected = decision_fence(triage, &opened);
    let adapter = fixture.adapter();
    let id = opened.record.request.id();
    for now in [NOW + 24 * HOUR, NOW + 25 * HOUR] {
        assert!(
            adapter
                .remind_issue_decision(fixture.cwd(), expected, &assessment, id, now)
                .unwrap()
                .is_none()
        );
    }
    let mut stale = expected;
    stale.decision_revision += 1;
    assert!(
        adapter
            .remind_issue_decision(fixture.cwd(), stale, &assessment, id, NOW + 22 * HOUR)
            .unwrap()
            .is_none()
    );
    stale = expected;
    stale.triage.collection_epoch += 1;
    assert!(
        adapter
            .remind_issue_decision(fixture.cwd(), stale, &assessment, id, NOW + 22 * HOUR)
            .unwrap()
            .is_none()
    );
    let answer = human_answered(&opened.record.request, &assessment, "accept");
    commit(
        adapter
            .save_issue_decision(fixture.cwd(), expected, &assessment, &answer)
            .unwrap(),
    );
    assert!(
        adapter
            .remind_issue_decision(fixture.cwd(), expected, &assessment, id, NOW + 22 * HOUR)
            .unwrap()
            .is_none()
    );
    assert!(
        adapter
            .issue_decision_deadlines(fixture.cwd(), None, 10)
            .unwrap()
            .is_empty()
    );
    assert!(
        adapter
            .pending_issue_decision_reminders(fixture.cwd(), 10)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn changed_actor_basis_cannot_emit_a_stale_reminder_with_a_fresh_row_fence() {
    let fixture = Fixture::new("decision-reminder-actor");
    let (triage, assessment, opened) = timed(&fixture);
    let mut changed = observation(&fixture);
    changed.author.as_mut().unwrap().id = "U_other".into();
    observe(&fixture, changed);
    let adapter = fixture.adapter();
    assert!(
        adapter
            .remind_issue_decision(
                fixture.cwd(),
                decision_fence(triage, &opened),
                &assessment,
                opened.record.request.id(),
                NOW + 22 * HOUR,
            )
            .unwrap()
            .is_none()
    );
    let (fence, item) = subject(&fixture);
    let current = assess(&item, None);
    adapter.save_triage(fixture.cwd(), fence, &current).unwrap();
    let (fence, item) = subject(&fixture);
    let current = item.triage.unwrap().assessment;
    let before = adapter
        .get_issue_decision_commit(fixture.cwd(), opened.record.request.id())
        .unwrap()
        .unwrap();
    assert!(
        adapter
            .remind_issue_decision(
                fixture.cwd(),
                decision_fence(fence, &opened),
                &current,
                opened.record.request.id(),
                NOW + 22 * HOUR,
            )
            .unwrap()
            .is_none()
    );
    assert_eq!(
        adapter
            .get_issue_decision_commit(fixture.cwd(), opened.record.request.id())
            .unwrap()
            .unwrap(),
        before
    );
    assert!(
        adapter
            .pending_issue_decision_reminders(fixture.cwd(), 10)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn deadline_keyset_pages_are_bounded_and_include_disabled_reminder_schedules() {
    let fixture = Fixture::new("decision-deadline-pages");
    let (fence, assessment, original) = prepared(&fixture);
    let adapter = fixture.adapter();
    for index in 0..260 {
        let id = format!("deadline-{index:03}");
        let mut definition = original.definition().clone();
        definition.question = shared(&format!("공유된 결정 {index}"));
        let request =
            IssueDecisionRequest::new(IssueDecisionId::new(&id).unwrap(), definition).unwrap();
        commit(
            adapter
                .ensure_issue_decision_timed(
                    fixture.cwd(),
                    fence,
                    &assessment,
                    &request,
                    NOW,
                    &timeouts(if index % 2 == 0 { 0 } else { 7200 }),
                )
                .unwrap(),
        );
    }
    assert!(
        adapter
            .issue_decision_deadlines(fixture.cwd(), None, 0)
            .unwrap()
            .is_empty()
    );
    let first = adapter
        .issue_decision_deadlines(fixture.cwd(), None, usize::MAX)
        .unwrap();
    assert_eq!(first.len(), 256);
    assert_eq!(first[0].record.request.id().as_str(), "deadline-000");
    assert!(first[0].remind_at.is_none());
    assert!(first[1].remind_at.is_some());
    assert!(first.iter().all(|deadline| !deadline.reminded));
    let second = adapter
        .issue_decision_deadlines(
            fixture.cwd(),
            first.last().map(|deadline| deadline.record.request.id()),
            256,
        )
        .unwrap();
    assert_eq!(second.len(), 4);
    assert_eq!(second[0].record.request.id().as_str(), "deadline-256");
    assert_eq!(second[3].record.request.id().as_str(), "deadline-259");
    assert!(
        adapter
            .remind_issue_decision(
                fixture.cwd(),
                IssueDecisionFence {
                    triage: fence,
                    decision_revision: first[0].record.revision
                },
                &assessment,
                first[0].record.request.id(),
                NOW + 22 * HOUR,
            )
            .unwrap()
            .is_none()
    );
}

#[test]
fn pending_reminders_are_ordered_by_event_id_and_ack_advances_the_bounded_page() {
    let fixture = Fixture::new("decision-reminder-pages");
    let (fence, assessment, original) = prepared(&fixture);
    let adapter = fixture.adapter();
    let mut events = Vec::new();
    for index in 0..3 {
        let mut definition = original.definition().clone();
        definition.question = shared(&format!("공유된 결정 {index}"));
        let request = IssueDecisionRequest::new(
            IssueDecisionId::new(format!("pending-{index}")).unwrap(),
            definition,
        )
        .unwrap();
        let opened = commit(
            adapter
                .ensure_issue_decision_timed(
                    fixture.cwd(),
                    fence,
                    &assessment,
                    &request,
                    NOW,
                    &timeouts(7200),
                )
                .unwrap(),
        );
        events.push(
            adapter
                .remind_issue_decision(
                    fixture.cwd(),
                    decision_fence(fence, &opened),
                    &assessment,
                    opened.record.request.id(),
                    NOW + 22 * HOUR,
                )
                .unwrap()
                .unwrap(),
        );
    }
    events.sort_by(|left, right| left.event_id.cmp(&right.event_id));
    assert!(
        adapter
            .pending_issue_decision_reminders(fixture.cwd(), 0)
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        adapter
            .pending_issue_decision_reminders(fixture.cwd(), 2)
            .unwrap(),
        events[..2]
    );
    for event in &events[..2] {
        adapter
            .acknowledge_issue_decision_reminder(fixture.cwd(), &event.event_id)
            .unwrap();
    }
    assert_eq!(
        fixture
            .adapter()
            .pending_issue_decision_reminders(fixture.cwd(), usize::MAX)
            .unwrap(),
        events[2..]
    );
}

#[test]
fn version_ten_migration_preserves_definitions_and_does_not_invent_reminder_history() {
    let fixture = Fixture::new("decision-deadline-migration-ten");
    let (fence, assessment, original) = prepared(&fixture);
    let adapter = fixture.adapter();
    let original = commit(
        adapter
            .ensure_issue_decision(fixture.cwd(), fence, &assessment, &original)
            .unwrap(),
    );
    let mut definition = original.record.request.definition().clone();
    definition.due_at = Some("2027-01-16T08:00:00Z".into());
    let dated =
        IssueDecisionRequest::new(IssueDecisionId::new("legacy-dated").unwrap(), definition)
            .unwrap();
    let dated = commit(
        adapter
            .ensure_issue_decision(fixture.cwd(), fence, &assessment, &dated)
            .unwrap(),
    );
    let before = adapter.read_triage_batch(fixture.cwd(), None, 100).unwrap();
    let selection = adapter.issue_autonomy_selection(fixture.cwd()).unwrap();
    adapter
        .with_connection(fixture.cwd(), |connection, _| {
            super::super::super::tests::make_pre_arbiter_schema(connection);
            connection
                .execute_batch("DROP TABLE issue_decision_reminders; PRAGMA user_version=10;")?;
            super::super::super::validate_schema_version(connection, 10)
        })
        .unwrap();
    let reopened = fixture.adapter();
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
    assert_eq!(
        reopened
            .get_issue_decision(fixture.cwd(), original.record.request.id())
            .unwrap(),
        Some(original.record.clone())
    );
    let deadlines = reopened
        .issue_decision_deadlines(fixture.cwd(), None, 10)
        .unwrap();
    assert_eq!(deadlines.len(), 1);
    assert_eq!(deadlines[0].record, dated.record);
    assert!(deadlines[0].remind_at.is_none());
    assert!(!deadlines[0].reminded);
    assert!(
        reopened
            .pending_issue_decision_reminders(fixture.cwd(), 10)
            .unwrap()
            .is_empty()
    );
    reopened
        .with_connection(fixture.cwd(), |connection, _| {
            assert_eq!(
                super::super::super::schema_identity(connection)?,
                (
                    super::super::super::APPLICATION_ID,
                    super::super::super::SCHEMA_VERSION
                )
            );
            super::super::super::validate_schema(connection)
        })
        .unwrap();
}

#[test]
fn pending_reminder_payload_must_match_the_immutable_decision() {
    let fixture = Fixture::new("decision-reminder-corrupt");
    let (fence, assessment, opened) = timed(&fixture);
    let adapter = fixture.adapter();
    let mut event = adapter
        .remind_issue_decision(
            fixture.cwd(),
            decision_fence(fence, &opened),
            &assessment,
            opened.record.request.id(),
            NOW + 22 * HOUR,
        )
        .unwrap()
        .unwrap();
    event.fact.basis.gate_revision += 1;
    adapter
        .with_connection(fixture.cwd(), |connection, _| {
            connection.execute(
                "UPDATE issue_decision_reminders SET event=?1 WHERE event_id=?2",
                params![serde_json::to_string(&event)?, event.event_id],
            )?;
            Ok(())
        })
        .unwrap();
    assert!(
        adapter
            .pending_issue_decision_reminders(fixture.cwd(), 10)
            .is_err()
    );
    assert!(
        adapter
            .issue_decision_deadlines(fixture.cwd(), None, 10)
            .is_err()
    );
}
