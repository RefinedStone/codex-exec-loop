//! Fake UTC drives the real observer, SQLite claims and kernel outbox; model turns stay fake.
use super::*;

use crate::adapter::outbound::git::issue_assessment_evidence::fake::FakeIssueAssessmentEvidenceAdapter;
use crate::adapter::outbound::issue_assessment_model::fake::FakeIssueAssessmentModelAdapter;
use crate::domain::issue_automation::arbiter::{
    IssueArbiterFinishedOutcome, IssueArbiterWaitReason,
};

const ARBITER_POLICY: &str = r#"
[binding.github]
mode = "gated"
[authority]
autonomy_cap = "gated"
[select]
mode = "all_open"
trusted_actors = [{provider="github",server_id="github.com",actor_id="U_decision"}]
[gates.gated.normal]
start = "timeout_arbiter"
[timeouts]
start = "4s"
remind_before = "2s"
"#;
const DEFER_REPLY: &str = r#"{"decision":"defer","confidence":9500,"reasons":["공유된 선택지에 따라 보류합니다."],"blockers":[]}"#;

fn arbiter_services(
    fixture: &Fixture,
    source: Arc<Source>,
    model: Arc<FakeIssueAssessmentModelAdapter>,
    evidence: Arc<FakeIssueAssessmentEvidenceAdapter>,
) -> (Services, Arc<IssueCoordinatorService>) {
    let mut services = fixture.services_with_policy(source.clone(), ARBITER_POLICY);
    // UTC is driven by FakeClock in the observer. Individual calls take no simulated
    // time here, so real filesystem/SQLite duration cannot shift exact event timestamps.
    let monotonic = Instant::now();
    let coordinator = Arc::new(
        IssueCoordinatorService::new(
            fixture.workspace.clone(),
            services.store.clone(),
            source.clone(),
            services.triage.clone(),
            services.decisions.clone(),
            IssuePolicyService::new(Arc::new(DisclosurePolicy)),
            Arc::new(NoExecution::default()),
        )
        .with_issue_binding_admission(Arc::new(
            crate::application::service::issue_binding_admission::IssueBindingAdmission::new(),
        ))
        .with_arbiter(model, evidence)
        .with_arbiter_clock(Arc::new(move || monotonic)),
    );
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

fn seed_arbiter(services: &Services, id: &str) -> IssueDecisionCommit {
    services.seed();
    // Arbiter claims require current collection consent. The observer uses the fake source
    // for its independent collection schedule; this fixture does not bypass the store fence.
    services
        .issues
        .set_mode_for_test(IssueCollectionMode::Observe)
        .unwrap();
    services
        .triage
        .triage_all(&IssueCollectionCancellation::new(), &|_| {})
        .unwrap();
    let mut definition = services.definition();
    definition.question = shared("공유 근거를 검토한 선택은 무엇인가요?");
    definition.options[0].effect = shared("진행 의사를 기록한다");
    definition.on_timeout = IssueDecisionTimeout::Arbiter;
    services
        .decisions
        .ensure_request_at(IssueDecisionId::new(id).unwrap(), definition, START_UTC_MS)
        .unwrap()
}

fn boot_arbiter(
    fixture: &Fixture,
    services: &Services,
    coordinator: Arc<IssueCoordinatorService>,
    log: Arc<SimKernelLog>,
    clock: &FakeClock,
) -> (SimKernel, Receiver<TrackingCompletion>) {
    let mut sim = fixture.boot(services, log);
    sim.host
        .runtime
        .force_parallel_mode_for_test(&fixture.workspace, true);
    let (completed, completions) = mpsc::channel();
    let utc = clock.utc.clone();
    let collect_utc = clock.utc.clone();
    let deadline_utc = clock.utc.clone();
    let deadline_coordinator = coordinator.clone();
    let collection = services.issues.clone();
    let observer = IssueObserver::for_test_with_tracking(
        services.issues.projection().unwrap(),
        Arc::new(move |cancel, _| {
            collection.collect_once(cancel, collect_utc.load(Ordering::SeqCst))
        }),
        Arc::new(move |cancel, _, execution_enabled, report| {
            let at = utc.load(Ordering::SeqCst);
            assert!(
                execution_enabled,
                "the fixture explicitly enables its fake executor"
            );
            let result = coordinator.track_execution(cancel, at, execution_enabled, &|facts| {
                report(
                    facts
                        .into_iter()
                        .map(Box::new)
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
        let now = deadline_utc.load(Ordering::SeqCst);
        deadline_coordinator
            .next_decision_deadline_at(now)
            .map(|next| next.map(|at| observer_now.saturating_add(at.saturating_sub(now))))
    }));
    sim.host.runtime.attach_issue_observer_for_test(observer);
    (sim, completions)
}

#[test]
fn kernel_simulation_arbiter_uses_exact_short_deadlines_and_one_durable_resolution_event() {
    let fixture = Fixture::new();
    let source = Source::new();
    let model = Arc::new(FakeIssueAssessmentModelAdapter::default());
    let evidence = Arc::new(FakeIssueAssessmentEvidenceAdapter::default());
    let (services, coordinator) = arbiter_services(&fixture, source, model.clone(), evidence);
    let initial = seed_arbiter(&services, "four-second-arbiter");
    let log = Arc::new(SimKernelLog::default());
    let clock = FakeClock::new();
    let (mut sim, completions) =
        boot_arbiter(&fixture, &services, coordinator, log.clone(), &clock);
    run_tracking_until(&mut sim, &completions, &clock, 0, 2_000);
    sim.host.run_cycle(clock.at(1_999), Duration::ZERO);
    assert!(completions.try_recv().is_err());
    assert!(model.requests().is_empty());
    run_tracking_until(&mut sim, &completions, &clock, 2_000, 4_000);
    assert_eq!(count(&log, "issue_decision_reminded"), 1);
    assert_eq!(stored(&services, &initial), initial.record);
    sim.host.run_cycle(clock.at(3_999), Duration::ZERO);
    assert!(completions.try_recv().is_err());
    assert!(model.requests().is_empty());
    run_tracking_until(&mut sim, &completions, &clock, 4_000, 9_000);
    let requests = model.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(stored(&services, &initial), initial.record);
    model.complete(&requests[0].attempt_id, DEFER_REPLY);
    run_tracking_until(&mut sim, &completions, &clock, 9_000, 14_000);
    let answered = stored(&services, &initial);
    assert_eq!(
        answered.request.answer().unwrap().by,
        IssueDecisionAnswerBy::Arbiter
    );
    assert_eq!(answered.request.answer().unwrap().option, "defer");
    assert_eq!(answered.revision, initial.record.revision + 1);
    assert_eq!(count(&log, "issue_decision_arbiter_finished"), 1);
    assert_eq!(count(&log, "issue_decision_resolved"), 0);
    assert!(
        services
            .store
            .pending_issue_arbiter_events(&fixture.workspace, 256)
            .unwrap()
            .is_empty()
    );
    run_tracking_until(&mut sim, &completions, &clock, 14_000, 19_000);
    assert_eq!(model.requests().len(), 1);
    assert_eq!(stored(&services, &initial), answered);
    assert_eq!(count(&log, "issue_decision_arbiter_finished"), 1);
    assert_eq!(model.forgotten(), vec![requests[0].attempt_id.clone()]);
    let records = log.0.lock().unwrap();
    let (outcome, at) = records
        .iter()
        .find_map(|record| match &record.event {
            KernelEvent::IssueDecisionArbiterFinished { outcome, at, .. } => Some((outcome, at)),
            _ => None,
        })
        .unwrap();
    assert_eq!(*outcome, IssueArbiterFinishedOutcome::Applied {});
    assert_eq!(
        chrono::DateTime::parse_from_rfc3339(at)
            .unwrap()
            .timestamp_millis(),
        (START_UTC_MS + 9_000) as i64
    );
}

#[test]
fn kernel_simulation_arbiter_wait_remains_visible_after_kernel_ack_and_reassembly() {
    let fixture = Fixture::new();
    let source = Source::new();
    let model = Arc::new(FakeIssueAssessmentModelAdapter::default());
    let evidence = Arc::new(FakeIssueAssessmentEvidenceAdapter::default());
    let (services, coordinator) =
        arbiter_services(&fixture, source.clone(), model.clone(), evidence.clone());
    let initial = seed_arbiter(&services, "wait-after-ack");
    let log = Arc::new(SimKernelLog::default());
    let clock = FakeClock::new();
    let (mut sim, completions) =
        boot_arbiter(&fixture, &services, coordinator, log.clone(), &clock);
    run_tracking_until(&mut sim, &completions, &clock, 0, 2_000);
    run_tracking_until(&mut sim, &completions, &clock, 2_000, 4_000);
    run_tracking_until(&mut sim, &completions, &clock, 4_000, 9_000);
    let id = model.requests()[0].attempt_id.clone();
    model.complete(
        &id,
        r#"{"decision":"defer","confidence":4000,"reasons":[],"blockers":[]}"#,
    );
    run_tracking_until(&mut sim, &completions, &clock, 9_000, 14_000);
    assert_eq!(stored(&services, &initial), initial.record);
    assert_eq!(count(&log, "issue_decision_arbiter_finished"), 1);
    assert!(
        services
            .store
            .pending_issue_arbiter_events(&fixture.workspace, 256)
            .unwrap()
            .is_empty()
    );
    let before = services
        .store
        .issue_decision_arbiter_waits(&fixture.workspace, &[initial.record.request.id().clone()])
        .unwrap();
    assert_eq!(before.waits.len(), 1);
    assert_eq!(
        before.waits[0].reason,
        IssueArbiterWaitReason::LowConfidence
    );
    drop(sim);
    drop(services);
    let restarted_model = Arc::new(FakeIssueAssessmentModelAdapter::default());
    let (recovered, coordinator) =
        arbiter_services(&fixture, source, restarted_model.clone(), evidence);
    let (mut sim, completions) =
        boot_arbiter(&fixture, &recovered, coordinator, log.clone(), &clock);
    run_tracking_until(&mut sim, &completions, &clock, 14_000, 19_000);
    assert_eq!(stored(&recovered, &initial), initial.record);
    assert_eq!(
        recovered
            .store
            .issue_decision_arbiter_waits(
                &fixture.workspace,
                &[initial.record.request.id().clone()]
            )
            .unwrap()
            .waits,
        before.waits
    );
    assert!(restarted_model.requests().is_empty());
    assert!(restarted_model.observations().is_empty());
    assert_eq!(count(&log, "issue_decision_arbiter_finished"), 1);
    assert_eq!(count(&log, "issue_decision_resolved"), 0);
}
