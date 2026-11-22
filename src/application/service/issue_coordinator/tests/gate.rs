use super::*;
use crate::domain::issue_automation::intervention::{
    IssueInterventionAction, IssueInterventionRequest,
};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread::{self, JoinHandle};
use std::time::Duration;

// Only bounds a missing positive handshake; no assertion depends on elapsed time.
const DEADLOCK_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Clone, Copy)]
enum Entry {
    Tick,
    Tracking,
}

impl Entry {
    fn run(
        self,
        coordinator: &IssueCoordinatorService,
        cancel: &IssueCollectionCancellation,
        committed: &Reporter<'_>,
    ) -> Result<()> {
        match self {
            Self::Tick => coordinator.tick(cancel, 123, false, committed),
            Self::Tracking => coordinator.track_execution(cancel, 123, false, committed),
        }
    }
}

fn coordinator_with_gate_events(
    h: &Harness,
) -> (
    Arc<IssueCoordinatorService>,
    Sender<IssueCoordinatorGateEvent>,
    Receiver<IssueCoordinatorGateEvent>,
) {
    let (events, receiver) = mpsc::channel();
    let mut coordinator = h.coordinator();
    coordinator.tick_gate_events = Some(events.clone());
    (Arc::new(coordinator), events, receiver)
}

fn spawn_entry(
    fixture: Arc<Harness>,
    coordinator: Arc<IssueCoordinatorService>,
    cancel: Arc<IssueCollectionCancellation>,
    entry: Entry,
    events: Sender<IssueCoordinatorGateEvent>,
    facts: Arc<Mutex<Vec<IssueCoordinatorFact>>>,
) -> JoinHandle<Result<()>> {
    thread::spawn(move || {
        // Keep the temporary store alive even if a missing handshake fails the test.
        let _fixture = fixture;
        let result = entry.run(&coordinator, &cancel, &|values| {
            facts.lock().unwrap().extend(values);
        });
        let _ = events.send(IssueCoordinatorGateEvent::Finished);
        result
    })
}

#[test]
fn collector_tick_waits_for_tracking_and_opens_one_start_decision() {
    let h = Arc::new(Harness::new());
    h.tick(false).unwrap();
    h.answer("start");
    h.tick(false).unwrap();
    let next_issue = h.fixture.reference("I_002");
    h.fixture
        .seed(vec![h.fixture.observation("I_002")], &disclosure_version());
    run(&h.triage);
    let (coordinator, events, receiver) = coordinator_with_gate_events(&h);
    let (tracking_entered, tracking_started) = mpsc::channel();
    let (release_tracking, tracking_release) = mpsc::channel();
    let tracking_release = Mutex::new(tracking_release);
    h.execution.state.lock().unwrap().after_observation = Some(Box::new(move || {
        tracking_entered.send(()).unwrap();
        tracking_release
            .lock()
            .unwrap()
            .recv_timeout(DEADLOCK_TIMEOUT)
            .expect("tracking holder was not released");
    }));
    let tracking_coordinator = coordinator.clone();
    let tracking_fixture = h.clone();
    let tracker = thread::spawn(move || {
        let _fixture = tracking_fixture;
        Entry::Tracking.run(
            &tracking_coordinator,
            &IssueCollectionCancellation::new(),
            &|_| {},
        )
    });
    tracking_started
        .recv_timeout(DEADLOCK_TIMEOUT)
        .expect("tracking never acquired the coordinator gate");
    let collector = spawn_entry(
        h.clone(),
        coordinator.clone(),
        Arc::new(IssueCollectionCancellation::new()),
        Entry::Tick,
        events,
        Arc::new(Mutex::new(Vec::new())),
    );
    assert_eq!(
        receiver.recv_timeout(DEADLOCK_TIMEOUT).unwrap(),
        IssueCoordinatorGateEvent::Contended,
        "the collection tick must reach the occupied gate before tracking is released",
    );
    release_tracking.send(()).unwrap();
    assert_eq!(
        receiver.recv_timeout(DEADLOCK_TIMEOUT).unwrap(),
        IssueCoordinatorGateEvent::Finished,
    );
    collector.join().unwrap().unwrap();
    tracker.join().unwrap().unwrap();

    let current = page(&h.triage);
    let item = current
        .items
        .iter()
        .find(|item| item.issue == next_issue)
        .unwrap();
    assert_eq!(item.open_decisions.len(), 1);
    let first = item.open_decisions[0].clone();
    assert_eq!(
        first.request.definition().kind,
        IssueDecisionKind::StartApproval
    );
    coordinator
        .tick(&IssueCollectionCancellation::new(), 124, false, &|_| {})
        .unwrap();
    let current = page(&h.triage);
    let item = current
        .items
        .iter()
        .find(|item| item.issue == next_issue)
        .unwrap();
    assert_eq!(item.open_decisions, vec![first]);
    assert_eq!(h.execution.state.lock().unwrap().registrations, 1);
}

fn assert_cancelled_wait_has_no_effect(entry: Entry) {
    let h = Arc::new(Harness::new());
    let store = h.fixture.store();
    let pending = store
        .begin_issue_intervention(
            &h.fixture.workspace,
            &IssueInterventionRequest {
                command_id: "cancelled-coordinator-wait".into(),
                issue: h.fixture.reference("I_001"),
                action: IssueInterventionAction::Hold,
            },
        )
        .unwrap();
    assert!(pending.receipt.is_none());
    let (coordinator, events, receiver) = coordinator_with_gate_events(&h);
    let held_gate = coordinator.tick_gate.lock().unwrap();
    let cancel = Arc::new(IssueCollectionCancellation::new());
    let facts = Arc::new(Mutex::new(Vec::new()));
    let worker = spawn_entry(
        h.clone(),
        coordinator.clone(),
        cancel.clone(),
        entry,
        events,
        facts.clone(),
    );
    assert_eq!(
        receiver.recv_timeout(DEADLOCK_TIMEOUT).unwrap(),
        IssueCoordinatorGateEvent::Contended,
    );
    // The worker has observed contention and cannot acquire this gate before cancellation.
    cancel.cancel();
    drop(held_gate);
    assert_eq!(
        receiver.recv_timeout(DEADLOCK_TIMEOUT).unwrap(),
        IssueCoordinatorGateEvent::Finished,
    );
    worker.join().unwrap().unwrap();

    assert_eq!(
        store
            .pending_issue_interventions(&h.fixture.workspace)
            .unwrap(),
        vec![pending],
    );
    assert!(h.work().is_none());
    assert!(page(&h.triage).items[0].open_decisions.is_empty());
    assert!(facts.lock().unwrap().is_empty());
    assert_eq!(h.source.calls.load(Ordering::SeqCst), 0);
    {
        let state = h.execution.state.lock().unwrap();
        assert!(state.control_commands.is_empty());
        assert!(state.observed_work_orders.is_empty());
        assert!(state.execution_limits.is_empty());
        assert!(state.revoked_work_orders.is_empty());
        assert!(state.grants.is_empty());
        assert_eq!(state.registrations, 0);
    }

    // This pending control is actionable: an uncancelled entry must recover it.
    entry
        .run(&coordinator, &IssueCollectionCancellation::new(), &|_| {})
        .unwrap();
    assert!(
        store
            .pending_issue_interventions(&h.fixture.workspace)
            .unwrap()
            .is_empty()
    );
    assert_eq!(h.execution.state.lock().unwrap().control_commands.len(), 1);
}

#[test]
fn cancelled_tick_wait_does_not_recover_pending_controls() {
    assert_cancelled_wait_has_no_effect(Entry::Tick);
}

#[test]
fn cancelled_tracking_wait_does_not_recover_pending_controls() {
    assert_cancelled_wait_has_no_effect(Entry::Tracking);
}

#[test]
fn poisoned_gate_is_an_error_for_tick_and_tracking() {
    let h = Harness::new();
    let coordinator = h.coordinator();
    let gate = &coordinator.tick_gate;
    assert!(
        std::panic::catch_unwind(move || {
            let _guard = gate.lock().unwrap();
            panic!("poison the coordinator gate");
        })
        .is_err()
    );
    for entry in [Entry::Tick, Entry::Tracking] {
        let error = entry
            .run(&coordinator, &IssueCollectionCancellation::new(), &|_| {})
            .unwrap_err();
        assert_eq!(
            error.to_string(),
            "the issue coordinator tick gate is poisoned"
        );
    }
    assert!(page(&h.triage).items[0].open_decisions.is_empty());
    assert_eq!(h.source.calls.load(Ordering::SeqCst), 0);
    assert_eq!(h.execution.state.lock().unwrap().registrations, 0);
}
