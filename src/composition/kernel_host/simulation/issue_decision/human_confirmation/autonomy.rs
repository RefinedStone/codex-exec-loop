//! Auto의 Native 확인이 실제 저장 명령·커널 조회·재시작 영수증까지 이어지는지 검증한다.
use super::*;

use crate::application::port::outbound::issue_work_execution_port::{
    IssueWorkExecutionPort, IssueWorkExecutionSnapshot, IssueWorkPermitRequest,
};
use crate::application::service::issue_autonomy::IssueAutonomyService;
use crate::domain::issue_automation::IssueAutomationAutonomy;
use crate::domain::issue_automation::autonomy::{
    IssueAutonomyLevel, IssueAutonomyProjection, IssueAutonomyReady,
};
use crate::domain::issue_automation::coordinator::IssueWorkOrder;

const AUTO_POLICY: &str = r#"
[binding.github]
mode = "auto"
[authority]
autonomy_cap = "auto"
[select]
mode = "all_open"
"#;

struct NoExecution;

impl IssueWorkExecutionPort for NoExecution {
    fn snapshot(&self, _: &str, _: &str) -> anyhow::Result<IssueWorkExecutionSnapshot> {
        panic!("an autonomy choice must not inspect an executor")
    }

    fn submit(&self, _: &str, _: &IssueWorkOrder) -> anyhow::Result<Vec<String>> {
        panic!("an autonomy choice must not submit work")
    }

    fn grant(&self, _: &str, _: &IssueWorkPermitRequest) -> anyhow::Result<()> {
        panic!("an autonomy choice must not grant delivery")
    }

    fn revoke(&self, _: &str, _: &str) -> anyhow::Result<()> {
        panic!("the empty work fixture has no permit to revoke")
    }
}

fn services(fixture: &Fixture, source: Arc<Source>) -> Services {
    let mut services = fixture.services_with_policy(source.clone(), AUTO_POLICY);
    let autonomy = Arc::new(IssueAutonomyService::new(
        fixture.workspace.clone(),
        services.store.clone(),
        IssueAutomationPolicyService::new(Arc::new(AutomationPolicy(AUTO_POLICY))),
        Arc::new(NoExecution),
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
        .with_autonomy(autonomy),
    );
    services
}

fn command() -> WireCommand {
    WireCommand::SetIssueAutonomy {
        command_id: "native-auto".into(),
        level: IssueAutonomyLevel::Auto,
    }
}

fn current(sim: &mut SimKernel) -> IssueAutonomyReady {
    let (reply, response) = mpsc::channel();
    sim.host
        .handle_input(KernelHostInput::IssueAutomation { reply });
    let view = response.try_recv().unwrap().unwrap();
    assert_eq!(view.projection.state.config.mode, IssueCollectionMode::Off);
    let IssueAutonomyProjection::Ready(autonomy) = view.autonomy else {
        panic!("the attached kernel must expose the stored autonomy choice");
    };
    autonomy
}

#[test]
fn kernel_simulation_auto_requires_exact_native_consent_and_replays_after_restart() {
    let fixture = Fixture::new();
    let source = Source::new();
    let configured = services(&fixture, source.clone());
    configured
        .issues
        .set_mode(IssueCollectionMode::Off)
        .unwrap();
    let log = Arc::new(SimKernelLog::default());
    let mut sim = fixture.boot(&configured, log.clone());
    let initial = current(&mut sim);
    assert_eq!(initial.requested_level, None);
    assert_eq!(initial.effective_level, IssueAutomationAutonomy::Propose);

    let caller_only = submit(
        &mut sim,
        &fixture.workspace,
        command(),
        "auto-caller-only",
        None,
    );
    assert_confirmation_rejected(&caller_only);
    assert_eq!(current(&mut sim), initial);

    let preview = configured
        .issues
        .autonomy_service()
        .unwrap()
        .prepare_confirmation("native-auto", IssueAutonomyLevel::Auto)
        .unwrap();
    let mut wrong = preview.target.clone();
    let IssueConfirmationTarget::Autonomy { command_id, .. } = &mut wrong else {
        panic!("Auto must use an exact autonomy confirmation target");
    };
    *command_id = "another-auto-command".into();
    let wrong_command = submit(
        &mut sim,
        &fixture.workspace,
        command(),
        "auto-wrong-command",
        Some(verified(wrong)),
    );
    assert_confirmation_rejected(&wrong_command);
    assert_eq!(current(&mut sim), initial);
    assert!(
        configured
            .store
            .get_issue_autonomy_command(&fixture.workspace, "native-auto")
            .unwrap()
            .is_none()
    );

    let accepted = submit(
        &mut sim,
        &fixture.workspace,
        command(),
        "auto-confirmed",
        Some(verified(preview.target)),
    );
    let KernelCommandOutcome::Handled {
        stored:
            Some(StoredCommandEnd::Finished(StoredCommandOutcome::Done(
                StoredCommandResult::IssueAutonomyChanged {
                    command_id,
                    changed: true,
                    autonomy,
                },
            ))),
        operation: Some(receipt),
        ..
    } = &accepted
    else {
        panic!("expected the kernel's confirmed Auto receipt: {accepted:?}");
    };
    assert_eq!(command_id, "native-auto");
    let IssueAutonomyProjection::Ready(returned) = autonomy.as_ref() else {
        panic!("confirmed Auto must return its stored projection");
    };
    let selected = current(&mut sim);
    assert_eq!(&selected, returned);
    assert_eq!(selected.requested_level, Some(IssueAutonomyLevel::Auto));
    assert_eq!(selected.effective_level, IssueAutomationAutonomy::Auto);
    assert_eq!(selected.revision, initial.revision + 1);
    assert!(sim.host.runtime.has_remaining_work());
    assert!(sim.host.held_stored_answers.is_empty());
    assert!(sim.effects.take().is_empty());
    let op_id = receipt.op_id;
    drop(sim);
    drop(configured);

    let reopened = services(&fixture, source.clone());
    let mut sim = fixture.boot(&reopened, log.clone());
    assert_eq!(current(&mut sim), selected);
    let replay = submit(
        &mut sim,
        &fixture.workspace,
        command(),
        "auto-confirmed",
        None,
    );
    assert!(matches!(replay, KernelCommandOutcome::Replayed(receipt) if receipt.op_id == op_id));
    assert_eq!(current(&mut sim), selected);
    assert_eq!(
        reopened
            .store
            .get_issue_autonomy_command(&fixture.workspace, "native-auto")
            .unwrap()
            .unwrap()
            .completed_revision,
        Some(selected.revision)
    );
    assert!(
        reopened
            .store
            .list_issue_work(&fixture.workspace)
            .unwrap()
            .is_empty()
    );
    assert_eq!(source.calls.load(Ordering::SeqCst), 0);
    assert!(sim.effects.take().is_empty());
    drop(sim);
    let recorded = log.0.lock().unwrap();
    assert_eq!(
        recorded
            .iter()
            .filter(|record| matches!(
                &record.event,
                KernelEvent::CommandCompleted { op_id: recorded_op, .. } if *recorded_op == op_id
            ))
            .count(),
        1
    );
    assert!(
        !serde_json::to_string(&*recorded)
            .unwrap()
            .contains(REFERENCE)
    );
}
