use super::*;
use crate::domain::issue_automation::coordinator::{
    IssueWorkCheckpoint, IssueWorkEvent, IssueWorkEventKind, IssueWorkOrder, issue_work_order_id,
};
use crate::domain::issue_automation::decision::{IssueDecisionBasis, IssueDecisionId};
use rusqlite::params;

fn discover(fixture: &Fixture, closed: bool) -> StoredIssueCollection {
    let adapter = fixture.adapter();
    let initial = adapter.ensure_collection(fixture.cwd()).unwrap();
    let request = request(fixture, &initial);
    let mut observation = observed(fixture, "I_close", "shared");
    if closed {
        observation.remote_state = IssueRemoteState::Closed;
    }
    commit(
        adapter
            .apply_discovery_page(
                fixture.cwd(),
                initial.fence(),
                &request,
                &page(fixture, &request, vec![observation], None),
            )
            .unwrap(),
    )
    .state
}

fn work(fixture: &Fixture, submitted: bool) -> IssueWorkCheckpoint {
    let adapter = fixture.adapter();
    let issue = reference(fixture.cwd(), "I_close");
    let tracked = adapter.get_issue(fixture.cwd(), &issue).unwrap().unwrap();
    let ready = transition_and_save(
        &adapter,
        fixture.cwd(),
        &tracked,
        IssueTransition::Assess(IssueTriageResult::Ready),
    );
    let queued = transition_and_save(&adapter, fixture.cwd(), &ready, IssueTransition::Queue);
    let shared = &queued.snapshot().shared_copy;
    let field_hash = queued.snapshot().basis.field_hash.clone();
    let work = IssueWorkCheckpoint {
        delivery_assessment: None,
        delivery_revision: 0,
        authorization: None,
        permit_authorization_epoch: 0,
        order: IssueWorkOrder {
            work_order_id: issue_work_order_id(&issue),
            issue: issue.clone(),
            field_hash: field_hash.clone(),
            title: shared.title.clone(),
            handoff: shared.clone(),
            acceptance: vec!["통합 사실과 외부 상태를 구분한다.".into()],
            scope: fixture.cwd().into(),
            branch_hint: None,
            link_reference: None,
            provider_link: false,
            profile: None,
            priority: 0,
            attempt: 1,
        },
        start_authority:
            crate::domain::issue_automation::coordinator::IssueWorkStartAuthority::Human,
        decision_id: Some(IssueDecisionId::new("fixture-close-start").unwrap()),
        approved_basis: IssueDecisionBasis {
            delivery: None,
            authorization_epoch: 0,
            policy_version: "a".repeat(64),
            field_hash: Some(field_hash),
            work_order_id: None,
            gate_revision: 1,
        },
        task_ids: if submitted {
            vec!["task-close".into()]
        } else {
            Vec::new()
        },
        permit_source: None,
        permit_revision: 0,
        delivery_granted: false,
        blocked_reason: None,
        revision: 1,
    };
    work.validate().unwrap();
    adapter.with_connection(fixture.cwd(), |connection, guard| {
        super::super::super::transaction(connection, guard, |tx| {
            tx.execute(
                "INSERT INTO issue_work_checkpoints
                 (workspace_dir,provider,server_id,repository_id,issue_id,work_order_id,checkpoint,revision)
                 VALUES (?1,?2,?3,?4,?5,?6,?7,1)",
                params![
                    fixture.cwd(), super::super::super::enum_name(&issue.provider)?,
                    issue.server_id, issue.repository_id, issue.issue_id,
                    work.order.work_order_id, serde_json::to_string(&work)?
                ],
            )?;
            Ok(())
        })
    }).unwrap();
    work
}

fn closed(fixture: &Fixture, body: &str) -> IssueSourceRead {
    let mut observation = observed(fixture, "I_close", body);
    observation.remote_state = IssueRemoteState::Closed;
    IssueSourceRead::Observed(Box::new(observation))
}

fn pending(fixture: &Fixture) -> Vec<IssueWorkEvent> {
    fixture
        .adapter()
        .pending_issue_work_events(fixture.cwd())
        .unwrap()
}

#[test]
fn external_close_is_atomic_with_the_observation_and_replays_the_same_durable_outbox_fact() {
    let fixture = Fixture::new("external-close-replay");
    let initial = discover(&fixture, false);
    let work = work(&fixture, true);
    let first = resync_one(&fixture, &initial, closed(&fixture, "shared"));
    let events = pending(&fixture);
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].issue, work.order.issue);
    assert_eq!(events[0].work_order_id, work.order.work_order_id);
    let page = fixture
        .adapter()
        .query_collection_issues(fixture.cwd(), &IssueCollectionQuery::default())
        .unwrap();
    let observation_revision = page.items[0].observation_revision.unwrap();
    assert_eq!(events[0].revision, observation_revision);
    assert_eq!(
        events[0].kind,
        IssueWorkEventKind::ExternalClosed {
            observation_revision
        }
    );
    // A new adapter instance reads the same durable fact after a process-style reopen.
    assert_eq!(pending(&fixture), events);
    assert_eq!(
        page.items[0].tracked.as_ref().unwrap().state,
        IssueAutomationState::Queued
    );
    assert!(
        fixture
            .adapter()
            .get_issue_execution(fixture.cwd(), &work.order.issue)
            .unwrap()
            .is_none()
    );

    let repeated = resync_one(&fixture, &first.state, closed(&fixture, "shared"));
    assert_eq!(pending(&fixture), events);
    let edited = resync_one(
        &fixture,
        &repeated.state,
        closed(&fixture, "changed while closed"),
    );
    assert_eq!(pending(&fixture), events);

    fixture
        .adapter()
        .acknowledge_issue_work_event(fixture.cwd(), &events[0].event_id)
        .unwrap();
    assert!(pending(&fixture).is_empty());
    let reopened = resync_one(
        &fixture,
        &edited.state,
        IssueSourceRead::Observed(Box::new(observed(&fixture, "I_close", "shared"))),
    );
    let closed_again = resync_one(&fixture, &reopened.state, closed(&fixture, "shared"));
    let next = pending(&fixture);
    assert_eq!(next.len(), 1);
    assert_ne!(next[0].event_id, events[0].event_id);
    assert!(next[0].revision > events[0].revision);
    assert_eq!(next[0].revision, closed_again.state.revision);
}

#[test]
fn a_close_without_submitted_work_never_fabricates_an_execution_fact() {
    for prepared in [false, true] {
        let fixture = Fixture::new(if prepared {
            "external-close-unsubmitted"
        } else {
            "external-close-no-work"
        });
        let initial = discover(&fixture, false);
        if prepared {
            work(&fixture, false);
        }
        resync_one(&fixture, &initial, closed(&fixture, "shared"));
        assert!(pending(&fixture).is_empty());
    }
}

#[test]
fn first_observation_already_closed_does_not_invent_an_open_to_closed_transition() {
    let fixture = Fixture::new("external-close-first-observation");
    let initial = discover(&fixture, true);
    assert!(pending(&fixture).is_empty());
    work(&fixture, true);
    resync_one(&fixture, &initial, closed(&fixture, "shared"));
    assert!(pending(&fixture).is_empty());
}

#[test]
fn recovering_from_unknown_to_closed_is_not_evidence_of_an_observed_transition() {
    let fixture = Fixture::new("external-close-unknown");
    let initial = discover(&fixture, false);
    let work = work(&fixture, true);
    let unknown = resync_one(&fixture, &initial, unknown(&work.order.issue));
    resync_one(&fixture, &unknown.state, closed(&fixture, "shared"));
    assert!(pending(&fixture).is_empty());
}
