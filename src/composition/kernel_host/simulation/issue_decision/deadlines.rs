//! Observer test assembly injects UTC explicitly; host scheduling and durable commits stay real.
use super::*;
use std::sync::atomic::{AtomicBool, AtomicU64};

use crate::application::port::outbound::issue_work_execution_port::{
    IssueWorkExecutionPort, IssueWorkExecutionSnapshot, IssueWorkPermitRequest,
};
use crate::application::service::issue_coordinator::IssueCoordinatorService;
use crate::composition::issue_observer::{IssueObserver, IssueObserverCommittedFact};
use crate::domain::issue_automation::coordinator::IssueWorkOrder;

const START_UTC_MS: u64 = 1_000_000;
const TTL_MS: u64 = 24 * 60 * 60 * 1_000;
const REMINDER_MS: u64 = TTL_MS - 2 * 60 * 60 * 1_000;
const TRACK_MS: u64 = 5_000;
type TrackingCompletion = (u64, std::result::Result<(), String>);
const TIMEOUT_POLICY: &str = r#"
[binding.github]
mode = "gated"
[authority]
autonomy_cap = "gated"
[select]
mode = "all_open"
trusted_actors = [{provider="github",server_id="github.com",actor_id="U_decision"}]
[gates.gated.normal]
start = "timeout_default"
"#;
const SHORT_TIMEOUT_POLICY: &str = r#"
[binding.github]
mode = "gated"
[authority]
autonomy_cap = "gated"
[select]
mode = "all_open"
trusted_actors = [{provider="github",server_id="github.com",actor_id="U_decision"}]
[gates.gated.normal]
start = "timeout_default"
[timeouts]
start = "4s"
remind_before = "2s"
"#;

struct FakeClock {
    base: Instant,
    utc: Arc<AtomicU64>,
}

impl FakeClock {
    fn new() -> Self {
        Self {
            base: Instant::now() + FAKE_CLOCK_OFFSET,
            utc: Arc::new(AtomicU64::new(START_UTC_MS)),
        }
    }

    fn at(&self, elapsed_ms: u64) -> Instant {
        self.utc.store(START_UTC_MS + elapsed_ms, Ordering::SeqCst);
        self.base + Duration::from_millis(elapsed_ms)
    }
}

#[derive(Default)]
struct NoExecution(AtomicUsize);

impl IssueWorkExecutionPort for NoExecution {
    fn snapshot(&self, _: &str, _: &str) -> anyhow::Result<IssueWorkExecutionSnapshot> {
        self.0.fetch_add(1, Ordering::SeqCst);
        anyhow::bail!("deadline-only simulation cannot inspect an executor")
    }

    fn submit(&self, _: &str, _: &IssueWorkOrder) -> anyhow::Result<Vec<String>> {
        self.0.fetch_add(1, Ordering::SeqCst);
        anyhow::bail!("deadline-only simulation cannot submit work")
    }

    fn grant(&self, _: &str, _: &IssueWorkPermitRequest) -> anyhow::Result<()> {
        self.0.fetch_add(1, Ordering::SeqCst);
        anyhow::bail!("deadline-only simulation cannot grant a permit")
    }

    fn revoke(&self, _: &str, _: &str) -> anyhow::Result<()> {
        self.0.fetch_add(1, Ordering::SeqCst);
        anyhow::bail!("deadline-only simulation cannot revoke a permit")
    }
}

fn services(
    fixture: &Fixture,
    source: Arc<Source>,
    execution: Arc<NoExecution>,
) -> (Services, Arc<IssueCoordinatorService>) {
    services_with_policy(fixture, source, execution, TIMEOUT_POLICY)
}

fn services_with_policy(
    fixture: &Fixture,
    source: Arc<Source>,
    execution: Arc<NoExecution>,
    policy: &'static str,
) -> (Services, Arc<IssueCoordinatorService>) {
    let mut services = fixture.services_with_policy(source.clone(), policy);
    let coordinator = Arc::new(IssueCoordinatorService::new(
        fixture.workspace.clone(),
        services.store.clone(),
        source.clone(),
        services.triage.clone(),
        services.decisions.clone(),
        IssuePolicyService::new(Arc::new(DisclosurePolicy)),
        execution,
    ));
    services.issues = Arc::new(
        IssueCollectionService::new(
            fixture.workspace.clone(),
            services.store.clone(),
            source,
            Arc::new(Origin),
            IssuePolicyService::new(Arc::new(DisclosurePolicy)),
        )
        .with_triage(services.triage.clone())
        .with_coordinator(coordinator.clone()),
    );
    (services, coordinator)
}

fn seed_default(services: &Services, id: &str) -> IssueDecisionCommit {
    services.seed();
    let mut definition = services.definition();
    definition.on_timeout = IssueDecisionTimeout::Default;
    services
        .decisions
        .ensure_request_at(IssueDecisionId::new(id).unwrap(), definition, START_UTC_MS)
        .unwrap()
}

fn boot_with_clock(
    fixture: &Fixture,
    services: &Services,
    coordinator: Arc<IssueCoordinatorService>,
    log: Arc<SimKernelLog>,
    clock: &FakeClock,
) -> (SimKernel, Receiver<TrackingCompletion>) {
    let mut sim = fixture.boot(services, log);
    let (completed, completions) = mpsc::channel();
    let utc = clock.utc.clone();
    let deadline_utc = clock.utc.clone();
    let deadline_coordinator = coordinator.clone();
    let observer = IssueObserver::for_test_with_tracking(
        services.issues.projection().unwrap(),
        Arc::new(|_, _| anyhow::bail!("collection is Off in the deadline simulation")),
        Arc::new(move |cancel, _wall_clock, execution_enabled, report| {
            // The observer owns the worker and its monotonic scheduling. Only this test
            // assembly replaces the external UTC reading with the explicit fake clock.
            let at = utc.load(Ordering::SeqCst);
            let result = coordinator.track_execution(cancel, at, execution_enabled, &|facts| {
                report(
                    facts
                        .into_iter()
                        .map(IssueObserverCommittedFact::Coordinated)
                        .collect(),
                );
            });
            completed
                .send((at, result.as_ref().map(|_| ()).map_err(ToString::to_string)))
                .unwrap();
            result
        }),
        sim.host.runtime.host_waker(),
    )
    .with_decision_deadlines_for_test(Arc::new(move |observer_now| {
        let fake_now = deadline_utc.load(Ordering::SeqCst);
        // The application uses fake UTC. Translate only the returned interval to the
        // observer flight's UTC origin so its production Instant mapping stays real.
        deadline_coordinator
            .next_decision_deadline_at(fake_now)
            .map(|next| next.map(|at| observer_now.saturating_add(at - fake_now)))
    }));
    sim.host.runtime.attach_issue_observer_for_test(observer);
    (sim, completions)
}

fn run_tracking(
    sim: &mut SimKernel,
    completions: &Receiver<TrackingCompletion>,
    clock: &FakeClock,
    elapsed_ms: u64,
) {
    run_tracking_until(sim, completions, clock, elapsed_ms, elapsed_ms + TRACK_MS);
}

fn run_tracking_until(
    sim: &mut SimKernel,
    completions: &Receiver<TrackingCompletion>,
    clock: &FakeClock,
    elapsed_ms: u64,
    next_elapsed_ms: u64,
) {
    let now = clock.at(elapsed_ms);
    sim.host.run_cycle(now, Duration::ZERO);
    let (observed_at, result) = completions.recv_timeout(WAIT).unwrap();
    assert_eq!(observed_at, START_UTC_MS + elapsed_ms);
    result.expect("the real deadline coordinator completes successfully");
    let until = Instant::now() + WAIT;
    loop {
        sim.host.run_cycle(now, Duration::from_millis(1));
        if sim.host.runtime.next_deadline(now)
            == Some(clock.base + Duration::from_millis(next_elapsed_ms))
        {
            break;
        }
        assert!(
            Instant::now() < until,
            "tracking completion did not advance the host deadline"
        );
    }
    assert!(
        sim.effects.take().is_empty(),
        "deadline processing must not invoke a model"
    );
}

fn count(log: &SimKernelLog, kind: &str) -> usize {
    log.0
        .lock()
        .unwrap()
        .iter()
        .filter(|record| record.event.kind() == kind)
        .count()
}

fn stored(services: &Services, initial: &IssueDecisionCommit) -> StoredIssueDecision {
    services
        .store
        .get_issue_decision(
            &initial.record.request.issue().project_scope.workspace_dir,
            initial.record.request.id(),
        )
        .unwrap()
        .unwrap()
}

#[test]
fn kernel_simulation_observer_fake_utc_drives_one_reminder_and_one_default_at_the_deadlines() {
    let fixture = Fixture::new();
    let source = Source::new();
    let execution = Arc::new(NoExecution::default());
    let (services, coordinator) = services(&fixture, source.clone(), execution.clone());
    let initial = seed_default(&services, "fake-clock-default");
    let source_calls = source.calls.load(Ordering::SeqCst);
    let log = Arc::new(SimKernelLog::default());
    let clock = FakeClock::new();
    let (mut sim, completions) =
        boot_with_clock(&fixture, &services, coordinator, log.clone(), &clock);
    run_tracking(&mut sim, &completions, &clock, 0);
    run_tracking(&mut sim, &completions, &clock, REMINDER_MS - TRACK_MS);
    sim.host
        .run_cycle(clock.at(REMINDER_MS - 1), Duration::ZERO);
    assert!(completions.try_recv().is_err());
    assert_eq!(count(&log, "issue_decision_reminded"), 0);
    assert_eq!(stored(&services, &initial), initial.record);

    run_tracking(&mut sim, &completions, &clock, REMINDER_MS);
    assert_eq!(count(&log, "issue_decision_reminded"), 1);
    assert_eq!(
        stored(&services, &initial),
        initial.record,
        "reminder must preserve the answer and confirmation revision"
    );
    assert!(
        services
            .store
            .pending_issue_decision_reminders(&fixture.workspace, 256)
            .unwrap()
            .is_empty()
    );
    let records = log.0.lock().unwrap();
    let reminder = records
        .iter()
        .find_map(|record| match &record.event {
            KernelEvent::IssueDecisionReminded {
                due_at,
                reminded_at,
                ..
            } => Some((due_at, reminded_at)),
            _ => None,
        })
        .unwrap();
    assert_eq!(
        chrono::DateTime::parse_from_rfc3339(reminder.0)
            .unwrap()
            .timestamp_millis(),
        (START_UTC_MS + TTL_MS) as i64
    );
    assert_eq!(
        chrono::DateTime::parse_from_rfc3339(reminder.1)
            .unwrap()
            .timestamp_millis(),
        (START_UTC_MS + REMINDER_MS) as i64
    );
    drop(records);

    run_tracking(&mut sim, &completions, &clock, TTL_MS - TRACK_MS);
    sim.host.run_cycle(clock.at(TTL_MS - 1), Duration::ZERO);
    assert!(completions.try_recv().is_err());
    assert_eq!(
        stored(&services, &initial).request.state(),
        IssueDecisionState::Open
    );
    assert_eq!(count(&log, "issue_decision_resolved"), 0);
    run_tracking(&mut sim, &completions, &clock, TTL_MS);
    let resolved = stored(&services, &initial);
    let answer = resolved.request.answer().unwrap();
    assert_eq!(answer.by, IssueDecisionAnswerBy::Default);
    assert_eq!(answer.option, "defer");
    assert_eq!(
        chrono::DateTime::parse_from_rfc3339(&answer.at)
            .unwrap()
            .timestamp_millis(),
        (START_UTC_MS + TTL_MS) as i64
    );
    run_tracking(&mut sim, &completions, &clock, TTL_MS + TRACK_MS);
    assert_eq!(stored(&services, &initial), resolved);
    drop(sim);
    assert_eq!(count(&log, "issue_decision_resolved"), 1);
    assert_eq!(count(&log, "issue_decision_reminded"), 1);
    assert_eq!(source.calls.load(Ordering::SeqCst), source_calls);
    assert_eq!(execution.0.load(Ordering::SeqCst), 0);
}

#[test]
fn kernel_simulation_reassembled_observer_applies_an_overdue_default_once() {
    let fixture = Fixture::new();
    let source = Source::new();
    let execution = Arc::new(NoExecution::default());
    let (initial_services, _) = services(&fixture, source.clone(), execution.clone());
    let initial = seed_default(&initial_services, "overdue-default");
    drop(initial_services);
    let log = Arc::new(SimKernelLog::default());
    let clock = FakeClock::new();
    let mut first = None;
    for elapsed in [TTL_MS + TRACK_MS, TTL_MS + 2 * TRACK_MS] {
        let (services, coordinator) = services(&fixture, source.clone(), execution.clone());
        let (mut sim, completions) =
            boot_with_clock(&fixture, &services, coordinator, log.clone(), &clock);
        run_tracking(&mut sim, &completions, &clock, elapsed);
        let resolved = stored(&services, &initial);
        assert_eq!(
            resolved.request.answer().unwrap().by,
            IssueDecisionAnswerBy::Default
        );
        if let Some(first) = &first {
            assert_eq!(&resolved, first);
        } else {
            first = Some(resolved);
        }
        drop(sim);
        assert_eq!(count(&log, "issue_decision_resolved"), 1);
        assert_eq!(
            count(&log, "issue_decision_reminded"),
            0,
            "an overdue request does not emit a late reminder"
        );
    }
    assert_eq!(execution.0.load(Ordering::SeqCst), 0);
}

struct AppendCommittedButAckLost {
    inner: Arc<SimKernelLog>,
    fail_reminder_once: AtomicBool,
}

impl KernelEventLogPort for AppendCommittedButAckLost {
    fn append_kernel_events(
        &self,
        workspace: &str,
        records: &[RecordedKernelEvent],
    ) -> anyhow::Result<()> {
        self.inner.append_kernel_events(workspace, records)?;
        if records
            .iter()
            .any(|record| record.event.kind() == "issue_decision_reminded")
            && self.fail_reminder_once.swap(false, Ordering::SeqCst)
        {
            anyhow::bail!("simulated lost append acknowledgement");
        }
        Ok(())
    }

    fn last_kernel_event_sequence(&self, workspace: &str) -> anyhow::Result<i64> {
        self.inner.last_kernel_event_sequence(workspace)
    }

    fn load_kernel_events(
        &self,
        workspace: &str,
        kinds: &[&str],
    ) -> anyhow::Result<Vec<KernelEvent>> {
        self.inner.load_kernel_events(workspace, kinds)
    }
}

#[test]
fn kernel_simulation_reminder_append_ack_loss_replays_one_durable_event_after_reassembly() {
    let fixture = Fixture::new();
    let source = Source::new();
    let execution = Arc::new(NoExecution::default());
    let (initial_services, coordinator) = services(&fixture, source.clone(), execution.clone());
    let initial = seed_default(&initial_services, "reminder-ack-loss");
    let log = Arc::new(SimKernelLog::default());
    let clock = FakeClock::new();
    let (mut sim, completions) = boot_with_clock(
        &fixture,
        &initial_services,
        coordinator,
        log.clone(),
        &clock,
    );
    sim.host.runtime.attach_kernel_event_log(
        Arc::new(AppendCommittedButAckLost {
            inner: log.clone(),
            fail_reminder_once: AtomicBool::new(true),
        }),
        &fixture.workspace,
    );
    run_tracking(&mut sim, &completions, &clock, REMINDER_MS);
    assert_eq!(count(&log, "issue_decision_reminded"), 1);
    let pending = initial_services
        .store
        .pending_issue_decision_reminders(&fixture.workspace, 256)
        .unwrap();
    assert_eq!(
        pending.len(),
        1,
        "a failed append acknowledgement keeps the outbox receipt"
    );
    let event_id = pending[0].event_id.clone();
    assert_eq!(stored(&initial_services, &initial), initial.record);
    drop(sim);
    drop(initial_services);

    let (services, coordinator) = services(&fixture, source, execution);
    let (sim, _) = boot_with_clock(&fixture, &services, coordinator, log.clone(), &clock);
    assert!(
        services
            .store
            .pending_issue_decision_reminders(&fixture.workspace, 256)
            .unwrap()
            .is_empty()
    );
    assert_eq!(count(&log, "issue_decision_reminded"), 1);
    assert!(
        log.has_issue_work_event(&fixture.workspace, &event_id)
            .unwrap()
    );
    assert_eq!(stored(&services, &initial), initial.record);
    drop(sim);
}

#[test]
fn kernel_simulation_short_deadlines_wake_before_the_five_second_tracking_interval() {
    let fixture = Fixture::new();
    let source = Source::new();
    let execution = Arc::new(NoExecution::default());
    let (services, coordinator) = services_with_policy(
        &fixture,
        source.clone(),
        execution.clone(),
        SHORT_TIMEOUT_POLICY,
    );
    let initial = seed_default(&services, "four-second-default");
    let source_calls = source.calls.load(Ordering::SeqCst);
    assert_eq!(
        coordinator.next_decision_deadline_at(START_UTC_MS).unwrap(),
        Some(START_UTC_MS + 2_000)
    );
    let log = Arc::new(SimKernelLog::default());
    let clock = FakeClock::new();
    let (mut sim, completions) = boot_with_clock(
        &fixture,
        &services,
        coordinator.clone(),
        log.clone(),
        &clock,
    );

    // The initial worker discovers the persisted timer without request_tracking calls.
    run_tracking_until(&mut sim, &completions, &clock, 0, 2_000);
    sim.host.run_cycle(clock.at(1_999), Duration::ZERO);
    assert!(completions.try_recv().is_err());
    assert_eq!(count(&log, "issue_decision_reminded"), 0);
    run_tracking_until(&mut sim, &completions, &clock, 2_000, 4_000);
    assert_eq!(count(&log, "issue_decision_reminded"), 1);
    assert_eq!(stored(&services, &initial), initial.record);
    assert_eq!(
        coordinator
            .next_decision_deadline_at(START_UTC_MS + 2_000)
            .unwrap(),
        Some(START_UTC_MS + 4_000)
    );

    sim.host.run_cycle(clock.at(3_999), Duration::ZERO);
    assert!(completions.try_recv().is_err());
    assert_eq!(
        stored(&services, &initial).request.state(),
        IssueDecisionState::Open
    );
    run_tracking_until(&mut sim, &completions, &clock, 4_000, 9_000);
    let resolved = stored(&services, &initial);
    assert_eq!(
        resolved.request.answer().unwrap().by,
        IssueDecisionAnswerBy::Default
    );
    assert_eq!(
        coordinator
            .next_decision_deadline_at(START_UTC_MS + 4_000)
            .unwrap(),
        None
    );
    for _ in 0..3 {
        sim.host.run_cycle(clock.at(4_000), Duration::ZERO);
    }
    assert!(
        completions.try_recv().is_err(),
        "a consumed deadline must not keep the host awake"
    );
    run_tracking_until(&mut sim, &completions, &clock, 9_000, 14_000);
    assert_eq!(stored(&services, &initial), resolved);
    drop(sim);
    assert_eq!(count(&log, "issue_decision_reminded"), 1);
    assert_eq!(count(&log, "issue_decision_resolved"), 1);
    assert_eq!(source.calls.load(Ordering::SeqCst), source_calls);
    assert_eq!(execution.0.load(Ordering::SeqCst), 0);
}

#[test]
fn kernel_simulation_overdue_wait_and_legacy_requests_leave_no_immediate_deadline() {
    let fixture = Fixture::new();
    let source = Source::new();
    let execution = Arc::new(NoExecution::default());
    let (services, coordinator) = services_with_policy(
        &fixture,
        source.clone(),
        execution.clone(),
        SHORT_TIMEOUT_POLICY,
    );
    services.seed();
    let waiting = services
        .decisions
        .ensure_request_at(
            IssueDecisionId::new("short-wait").unwrap(),
            services.definition(),
            START_UTC_MS,
        )
        .unwrap();
    let legacy = services
        .decisions
        .ensure_request(
            IssueDecisionId::new("legacy-no-deadline").unwrap(),
            services.definition(),
        )
        .unwrap();
    assert!(legacy.record.request.definition().due_at.is_none());
    let source_calls = source.calls.load(Ordering::SeqCst);
    let log = Arc::new(SimKernelLog::default());
    let clock = FakeClock::new();
    let (mut sim, completions) = boot_with_clock(
        &fixture,
        &services,
        coordinator.clone(),
        log.clone(),
        &clock,
    );
    run_tracking_until(&mut sim, &completions, &clock, 0, 2_000);
    run_tracking_until(&mut sim, &completions, &clock, 2_000, 4_000);
    run_tracking_until(&mut sim, &completions, &clock, 4_000, 9_000);
    assert_eq!(
        coordinator
            .next_decision_deadline_at(START_UTC_MS + 4_000)
            .unwrap(),
        None
    );
    for _ in 0..3 {
        sim.host.run_cycle(clock.at(4_000), Duration::ZERO);
    }
    assert!(completions.try_recv().is_err());
    run_tracking_until(&mut sim, &completions, &clock, 9_000, 14_000);
    assert_eq!(stored(&services, &waiting), waiting.record);
    assert_eq!(stored(&services, &legacy), legacy.record);
    drop(sim);
    assert_eq!(count(&log, "issue_decision_reminded"), 1);
    assert_eq!(count(&log, "issue_decision_resolved"), 0);
    assert_eq!(source.calls.load(Ordering::SeqCst), source_calls);
    assert_eq!(execution.0.load(Ordering::SeqCst), 0);
}
