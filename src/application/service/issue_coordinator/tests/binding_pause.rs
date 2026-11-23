use super::*;
use crate::domain::issue_automation::binding_pause::IssueBindingPauseIntent;
use crate::domain::issue_automation::execution::*;

fn begin_pause(h: &Harness) {
    let item = page(&h.triage).items.remove(0);
    let store = h.fixture.store();
    let state = store
        .issue_binding_pause_state(&h.fixture.workspace, &item.binding_id)
        .unwrap();
    store
        .begin_issue_binding_pause(
            &h.fixture.workspace,
            &IssueBindingPauseIntent {
                command_id: "pause-coordinator".into(),
                binding_id: item.binding_id,
                paused: true,
                expected_revision: state.revision,
                resume_basis: None,
            },
        )
        .unwrap();
}

#[test]
fn pending_pause_does_not_create_a_new_start_decision_or_fetch_the_provider() {
    let h = Harness::new();
    begin_pause(&h);
    let before = h.source.calls.load(Ordering::SeqCst);
    let facts = Mutex::new(Vec::new());
    h.coordinator()
        .tick(&IssueCollectionCancellation::new(), 123, true, &|values| {
            facts.lock().unwrap().extend(values)
        })
        .unwrap();
    assert!(page(&h.triage).items[0].binding_run_blocked);
    assert!(page(&h.triage).items[0].open_decisions.is_empty());
    assert_eq!(h.source.calls.load(Ordering::SeqCst), before);
    assert_eq!(h.execution.state.lock().unwrap().registrations, 0);
    assert!(
        !facts
            .lock()
            .unwrap()
            .iter()
            .any(|fact| matches!(fact, IssueCoordinatorFact::Decision { .. }))
    );
}

#[test]
fn pending_pause_keeps_terminal_unknown_facts_and_existing_human_answers_without_new_recovery() {
    let h = Harness::new();
    h.tick(true).unwrap();
    h.answer("start");
    h.tick(true).unwrap();
    let work = h.work().unwrap();
    let approved = h
        .fixture
        .store()
        .get_issue_decision(&h.fixture.workspace, work.decision_id.as_ref().unwrap())
        .unwrap();
    begin_pause(&h);
    h.execution.state.lock().unwrap().observation = Some(IssueWorkExecutionObservation {
        phase: IssueExecutionPhase::Terminal,
        task_ids: work.task_ids.clone(),
        active_task_id: work.task_ids.first().cloned(),
        execution_ids: work.task_ids.clone(),
        terminals: vec![IssueExecutionTerminalRecord {
            record_id: "unknown-during-pause".into(),
            classification: IssueExecutionTerminalClass::ContractRecovery,
        }],
        current_terminal_id: Some("unknown-during-pause".into()),
        ..Default::default()
    });
    let limits_before = h.execution.state.lock().unwrap().execution_limits.len();
    let facts = Mutex::new(Vec::new());
    h.coordinator()
        .track_execution(&IssueCollectionCancellation::new(), 124, true, &|values| {
            facts.lock().unwrap().extend(values)
        })
        .unwrap();
    let saved = h
        .fixture
        .store()
        .get_issue_execution(&h.fixture.workspace, &work.order.issue)
        .unwrap()
        .unwrap();
    assert!(
        saved
            .progress
            .terminal
            .as_ref()
            .is_some_and(|terminal| terminal.record_id == "unknown-during-pause")
    );
    assert_eq!(
        h.fixture
            .store()
            .get_issue_decision(&h.fixture.workspace, work.decision_id.as_ref().unwrap())
            .unwrap(),
        approved
    );
    assert!(
        page(&h.triage).items[0]
            .open_decisions
            .iter()
            .all(
                |decision| decision.request.definition().kind != IssueDecisionKind::FailureRecovery
            )
    );
    let execution = h.execution.state.lock().unwrap();
    assert!(execution.recoveries.is_empty());
    assert!(
        execution.execution_limits[limits_before..]
            .iter()
            .all(|(_, limit)| *limit == 0)
    );
    assert!(
        facts
            .lock()
            .unwrap()
            .iter()
            .any(|fact| matches!(fact, IssueCoordinatorFact::ExecutionProgressed { .. }))
    );
}
