//! Fake provider/model-free kernel flow: acceptance, drain, native resume, restart and replay.
use super::*;
use crate::application::service::issue_binding_admission::IssueBindingAdmission;
use crate::application::service::issue_binding_pause::IssueBindingPauseService;
use crate::domain::issue_automation::binding_pause::IssueBindingPauseOutcome;

fn command(binding: &str, id: &str, paused: bool, revision: u64) -> WireCommand {
    WireCommand::SetIssueBindingPaused {
        command_id: id.into(),
        binding_id: binding.into(),
        paused,
        expected_revision: revision,
    }
}

fn boot(
    fixture: &Fixture,
    services: &Services,
    log: Arc<SimKernelLog>,
    pause: Arc<IssueBindingPauseService>,
    admission: Arc<IssueBindingAdmission>,
) -> SimKernel {
    pause.recover_before_runtime().unwrap();
    let mut sim = fixture.boot(services, log);
    let workspace = Arc::new(FilesystemPlanningWorkspaceAdapter::new());
    let admin = PlanningAdminFacadeService::from_planning_with_authority(
        &fixture.workspace,
        test_planning_services(workspace.clone()),
        workspace,
        Arc::new(NoopPlanningAuthorityPort::default()),
        Arc::new(NoopPlanningTaskRepositoryPort),
    );
    sim.host.runtime.attach_stored_commands(
        Arc::new(
            StoredCommandService::new(admin)
                .with_issue_collection(services.issues.clone())
                .with_binding_pause(pause.clone()),
        ),
        &fixture.workspace,
    );
    sim.host
        .runtime
        .attach_issue_binding_pause(pause, admission)
        .unwrap();
    sim
}

fn state(
    sim: &mut SimKernel,
) -> crate::domain::issue_automation::binding_pause::IssueBindingPauseState {
    let (reply, response) = mpsc::channel();
    sim.host
        .handle_input(KernelHostInput::IssueAutomation { reply });
    response.try_recv().unwrap().unwrap().binding_pause.unwrap()
}

#[test]
fn pause_waits_for_inflight_and_exact_native_resume_survives_restart_without_replaying_effects() {
    let fixture = Fixture::new();
    let source = Source::new();
    let mut services = fixture.services_with_policy(source.clone(), HUMAN_POLICY);
    services.seed();
    let binding = services
        .issues
        .projection()
        .unwrap()
        .state
        .binding_id
        .unwrap();
    let admission = Arc::new(IssueBindingAdmission::new());
    let pause = Arc::new(IssueBindingPauseService::new(
        fixture.workspace.clone(),
        services.store.clone(),
        IssueAutomationPolicyService::new(Arc::new(AutomationPolicy(HUMAN_POLICY))),
        Arc::new(super::autonomy::NoExecution),
        admission.clone(),
    ));
    services.issues = Arc::new(
        IssueCollectionService::new(
            fixture.workspace.clone(),
            services.store.clone(),
            source.clone(),
            Arc::new(Origin),
            IssuePolicyService::new(Arc::new(DisclosurePolicy)),
        )
        .with_triage(services.triage.clone())
        .with_binding_pause(pause.clone()),
    );
    let log = Arc::new(SimKernelLog::default());
    let mut sim = boot(
        &fixture,
        &services,
        log.clone(),
        pause.clone(),
        admission.clone(),
    );
    let initial = state(&mut sim);
    let guard = admission
        .acquire(&binding, admission.capture(&binding).unwrap())
        .unwrap();
    let accepted = submit(
        &mut sim,
        &fixture.workspace,
        command(binding.as_str(), "pause-binding", true, initial.revision),
        "pause-binding",
        None,
    );
    let KernelCommandOutcome::Handled {
        stored:
            Some(StoredCommandEnd::Finished(StoredCommandOutcome::Done(
                StoredCommandResult::IssueBindingPauseRequested { command: accepted },
            ))),
        ..
    } = accepted
    else {
        panic!("expected accepted binding pause")
    };
    assert!(accepted.receipt.is_none());
    assert!(state(&mut sim).pending.is_some());
    assert!(admission.capture(&binding).is_err());
    let _ = sim.host.runtime.run_host_cycle(
        Instant::now() + Duration::from_secs(2),
        Duration::ZERO,
        32,
    );
    assert!(pause.pending_events(64).unwrap().is_empty());
    assert!(sim.host.runtime.has_remaining_work());
    drop(guard);
    let _ = sim.host.runtime.run_host_cycle(
        Instant::now() + Duration::from_secs(3),
        Duration::ZERO,
        32,
    );
    let paused = state(&mut sim);
    assert!(paused.paused && paused.pending.is_none());
    assert_confirmation_rejected(&submit(
        &mut sim,
        &fixture.workspace,
        command(binding.as_str(), "resume-binding", false, paused.revision),
        "resume-no-proof",
        None,
    ));
    let preview = pause
        .prepare_confirmation("resume-binding", &binding, false, paused.revision)
        .unwrap();
    let mut wrong = preview.target.clone();
    let IssueConfirmationTarget::BindingResume {
        expected_revision, ..
    } = &mut wrong
    else {
        panic!("exact resume target")
    };
    *expected_revision += 1;
    assert_confirmation_rejected(&submit(
        &mut sim,
        &fixture.workspace,
        command(binding.as_str(), "resume-binding", false, paused.revision),
        "resume-wrong-proof",
        Some(verified(wrong)),
    ));
    let resumed = submit(
        &mut sim,
        &fixture.workspace,
        command(binding.as_str(), "resume-binding", false, paused.revision),
        "resume-binding",
        Some(verified(preview.target)),
    );
    assert!(matches!(
        resumed,
        KernelCommandOutcome::Handled {
            stored: Some(StoredCommandEnd::Finished(StoredCommandOutcome::Done(
                StoredCommandResult::IssueBindingPauseRequested { .. }
            ))),
            ..
        }
    ));
    let _ = sim.host.runtime.run_host_cycle(
        Instant::now() + Duration::from_secs(4),
        Duration::ZERO,
        32,
    );
    assert!(state(&mut sim).allows_automation());
    drop(sim);
    // Even an external strong Arc cannot start another action after this owner's Drop.
    assert!(admission.capture(&binding).is_err());
    let fresh_admission = Arc::new(IssueBindingAdmission::new());
    let reopened = Arc::new(IssueBindingPauseService::new(
        fixture.workspace.clone(),
        services.store.clone(),
        IssueAutomationPolicyService::new(Arc::new(AutomationPolicy(HUMAN_POLICY))),
        Arc::new(super::autonomy::NoExecution),
        fresh_admission.clone(),
    ));
    let mut sim = boot(
        &fixture,
        &services,
        log.clone(),
        reopened.clone(),
        fresh_admission,
    );
    let replay = reopened
        .set(
            "resume-binding".into(),
            binding.clone(),
            false,
            paused.revision,
            None,
        )
        .unwrap();
    assert_eq!(
        replay.receipt.unwrap().outcome,
        IssueBindingPauseOutcome::Resumed
    );
    assert!(state(&mut sim).allows_automation());
    assert!(sim.effects.take().is_empty());
    drop(sim);
    let records = log.0.lock().unwrap();
    let ids: Vec<_> = records
        .iter()
        .filter_map(|record| record.event.issue_binding_pause_event_id())
        .collect();
    assert_eq!(
        ids.len(),
        2,
        "one immutable completion per pause and resume"
    );
    assert_ne!(ids[0], ids[1]);
    assert!(
        !serde_json::to_string(&*records)
            .unwrap()
            .contains(REFERENCE)
    );
}
