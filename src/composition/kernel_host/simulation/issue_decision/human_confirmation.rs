//! 검증된 metadata의 실제 커널 전달과 무증거 receipt 재전송을 검증한다.
use super::*;
use crate::application::port::inbound::stored_command_port::issue_confirmation::{
    IssueConfirmationTarget, VerifiedHumanConfirmation,
};

mod autonomy;
mod delivery;

const REFERENCE: &str = "desktop-confirmation:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const HUMAN_POLICY: &str = r#"
[binding.github]
mode = "propose"
[authority]
autonomy_cap = "auto"
[select]
mode = "all_open"
[[select.trusted_actors]]
provider = "github"
server_id = "github.com"
actor_id = "U_decision"
"#;

fn submit(
    sim: &mut SimKernel,
    workspace: &str,
    command: WireCommand,
    key: &str,
    confirmation: Option<VerifiedHumanConfirmation>,
) -> KernelCommandOutcome {
    let request = CommandRequest {
        command,
        idempotency_key: Some(key.into()),
    };
    let mut descriptor = request.descriptor().unwrap().unwrap();
    descriptor.caller = Some("Desktop".into());
    let KernelCommandInput::Stored(command) = request.command.into_input(workspace).unwrap() else {
        panic!("the fixture must submit a stored command");
    };
    let command = match confirmation {
        Some(confirmation) => KernelClientCommand::ConfirmedStored {
            command: Box::new(command),
            confirmation: Box::new(confirmation),
        },
        None => KernelClientCommand::Stored(Box::new(command)),
    };
    let (reply, response) = mpsc::channel();
    sim.host.handle_input(KernelHostInput::Command {
        command,
        descriptor: Some(descriptor),
        reply,
    });
    response
        .try_recv()
        .expect("human confirmation must not wait for planning")
}

fn answer_command(id: &str) -> WireCommand {
    WireCommand::AnswerIssueDecision {
        decision_id: id.into(),
        option: "proceed".into(),
        note: None,
    }
}

fn verified(target: IssueConfirmationTarget) -> VerifiedHumanConfirmation {
    VerifiedHumanConfirmation::from_verified(target, REFERENCE.into()).unwrap()
}

fn assert_confirmation_rejected(outcome: &KernelCommandOutcome) {
    let KernelCommandOutcome::Handled {
        stored: Some(StoredCommandEnd::Finished(StoredCommandOutcome::Rejected(rejection))),
        operation: Some(receipt),
        ..
    } = outcome
    else {
        panic!("expected a recorded human confirmation rejection: {outcome:?}");
    };
    assert_eq!(rejection.reason, ReasonCode::HumanConfirmationRequired);
    assert_eq!(
        receipt.rejection,
        Some(ReasonCode::HumanConfirmationRequired)
    );
}

#[test]
fn kernel_simulation_human_confirmation_rejects_caller_and_wrong_scope_then_replays_without_proof()
{
    let fixture = Fixture::new();
    let source = Source::new();
    let services = fixture.services_with_policy(source.clone(), HUMAN_POLICY);
    services.seed();
    let log = Arc::new(SimKernelLog::default());
    let mut sim = fixture.boot(&services, log.clone());
    let events = subscribe(&mut sim);
    let initial = services.ensure(&mut sim, "human-required");
    sim.host
        .run_cycle(Instant::now() + FAKE_CLOCK_OFFSET, Duration::ZERO);
    live_decisions(&events);
    let before = query(&mut sim, initial.record.request.issue());
    let reads = source.calls.load(Ordering::SeqCst);

    let spoofed = submit(
        &mut sim,
        &fixture.workspace,
        answer_command("human-required"),
        "human-caller-only",
        None,
    );
    assert_confirmation_rejected(&spoofed);
    assert_eq!(
        query(&mut sim, initial.record.request.issue()).revision,
        before.revision
    );
    assert_eq!(
        services
            .store
            .get_issue_decision(&fixture.workspace, initial.record.request.id())
            .unwrap()
            .unwrap(),
        initial.record
    );
    assert!(live_decisions(&events).is_empty());

    let preview = services
        .decisions
        .prepare_confirmation("human-required", "proceed")
        .unwrap();
    assert!(!preview.display.message.contains(SECRET));
    assert!(!preview.display.detail.contains(SECRET));
    let mut wrong_scope = preview.target.clone();
    let IssueConfirmationTarget::Decision { option, .. } = &mut wrong_scope else {
        panic!()
    };
    *option = "defer".into();
    let rejected = submit(
        &mut sim,
        &fixture.workspace,
        answer_command("human-required"),
        "human-wrong-option",
        Some(verified(wrong_scope)),
    );
    assert_confirmation_rejected(&rejected);
    assert_eq!(
        query(&mut sim, initial.record.request.issue()).revision,
        before.revision
    );

    let accepted = submit(
        &mut sim,
        &fixture.workspace,
        answer_command("human-required"),
        "human-valid",
        Some(verified(preview.target)),
    );
    let (fact, outcome, changed, revision, receipt) = done(&accepted);
    assert_eq!(fact.answer_by, Some(IssueDecisionAnswerBy::Human));
    assert_eq!(outcome, IssueDecisionOutcome::Answered);
    assert!(changed);
    let saved = services
        .store
        .get_issue_decision(&fixture.workspace, initial.record.request.id())
        .unwrap()
        .unwrap();
    assert_eq!(
        saved.request.answer().unwrap().confirmation.as_deref(),
        Some(REFERENCE)
    );
    let delivered = live_decisions(&events);
    assert_eq!(delivered.len(), 1);
    assert_eq!(delivered[0]["revision"], revision);

    let replay = submit(
        &mut sim,
        &fixture.workspace,
        answer_command("human-required"),
        "human-valid",
        None,
    );
    assert!(
        matches!(replay, KernelCommandOutcome::Replayed(other) if other.op_id == receipt.op_id)
    );
    assert_eq!(
        query(&mut sim, initial.record.request.issue()).revision,
        revision
    );
    assert!(live_decisions(&events).is_empty());
    assert_eq!(source.calls.load(Ordering::SeqCst), reads);
    assert_no_execution_effects(&mut sim, &fixture);
    assert!(
        !sim.host.runtime.has_remaining_work(),
        "Propose human decisions do not keep an otherwise idle kernel alive"
    );
    drop(sim);
    let records = serde_json::to_string(&*log.0.lock().unwrap()).unwrap();
    assert!(
        !records.contains(REFERENCE),
        "confirmation references stay out of general records"
    );
    assert!(!records.contains(SECRET));
}

#[test]
fn kernel_simulation_human_confirmation_restart_retains_human_fact_and_ordinary_retry_receipt() {
    let fixture = Fixture::new();
    let source = Source::new();
    let services = fixture.services_with_policy(source.clone(), HUMAN_POLICY);
    services.seed();
    let log = Arc::new(SimKernelLog::default());
    let mut sim = fixture.boot(&services, log.clone());
    let initial = services.ensure(&mut sim, "human-restart");
    let preview = services
        .decisions
        .prepare_confirmation("human-restart", "proceed")
        .unwrap();
    let accepted = submit(
        &mut sim,
        &fixture.workspace,
        answer_command("human-restart"),
        "human-restart-key",
        Some(verified(preview.target)),
    );
    let (_, _, _, _, receipt) = done(&accepted);
    let saved = services
        .store
        .get_issue_decision(&fixture.workspace, initial.record.request.id())
        .unwrap()
        .unwrap();
    drop(sim);
    drop(services);

    let reopened = fixture.services_with_policy(source, HUMAN_POLICY);
    let mut sim = fixture.boot(&reopened, log.clone());
    let replay = submit(
        &mut sim,
        &fixture.workspace,
        answer_command("human-restart"),
        "human-restart-key",
        None,
    );
    assert!(
        matches!(replay, KernelCommandOutcome::Replayed(other) if other.op_id == receipt.op_id)
    );
    let persisted = reopened
        .store
        .get_issue_decision(&fixture.workspace, initial.record.request.id())
        .unwrap()
        .unwrap();
    assert_eq!(persisted, saved);
    assert_eq!(
        persisted.request.answer().unwrap().by,
        IssueDecisionAnswerBy::Human
    );
    assert_no_execution_effects(&mut sim, &fixture);
    assert!(
        !sim.host.runtime.has_remaining_work(),
        "Propose human decisions do not keep an otherwise idle kernel alive"
    );
    drop(sim);
    assert_eq!(
        log.0
            .lock()
            .unwrap()
            .iter()
            .filter(|record| record.event.kind() == "issue_decision_resolved")
            .count(),
        1
    );
}

#[test]
fn kernel_simulation_human_confirmation_mode_raise_requires_proof_and_downward_mode_does_not() {
    let fixture = Fixture::new();
    let services = fixture.services(Source::new());
    services.seed();
    let log = Arc::new(SimKernelLog::default());
    let mut sim = fixture.boot(&services, log.clone());
    let before = services.issues.projection().unwrap();

    let denied = submit(
        &mut sim,
        &fixture.workspace,
        WireCommand::SetIssueAutomationMode {
            mode: IssueCollectionMode::Observe.into(),
        },
        "mode-caller-only",
        None,
    );
    assert_confirmation_rejected(&denied);
    assert_eq!(services.issues.projection().unwrap(), before);
    let preview = services
        .issues
        .prepare_mode_confirmation(IssueCollectionMode::Observe)
        .unwrap();
    let accepted = submit(
        &mut sim,
        &fixture.workspace,
        WireCommand::SetIssueAutomationMode {
            mode: IssueCollectionMode::Observe.into(),
        },
        "mode-confirmed",
        Some(verified(preview.target)),
    );
    let KernelCommandOutcome::Handled {
        stored:
            Some(StoredCommandEnd::Finished(StoredCommandOutcome::Done(
                StoredCommandResult::IssueAutomationModeChanged { changed: true, .. },
            ))),
        operation: Some(receipt),
        ..
    } = accepted
    else {
        panic!("expected confirmed mode change: {accepted:?}");
    };
    let raised = services.issues.projection().unwrap();
    assert_eq!(raised.state.config.mode, IssueCollectionMode::Observe);
    assert_eq!(raised.state.epoch, before.state.epoch + 1);

    let replay = submit(
        &mut sim,
        &fixture.workspace,
        WireCommand::SetIssueAutomationMode {
            mode: IssueCollectionMode::Observe.into(),
        },
        "mode-confirmed",
        None,
    );
    assert!(
        matches!(replay, KernelCommandOutcome::Replayed(other) if other.op_id == receipt.op_id)
    );
    assert_eq!(services.issues.projection().unwrap(), raised);

    let lowered = submit(
        &mut sim,
        &fixture.workspace,
        WireCommand::SetIssueAutomationMode {
            mode: IssueCollectionMode::Off.into(),
        },
        "mode-off",
        None,
    );
    assert!(matches!(
        lowered,
        KernelCommandOutcome::Handled {
            stored: Some(StoredCommandEnd::Finished(StoredCommandOutcome::Done(
                StoredCommandResult::IssueAutomationModeChanged { changed: true, .. }
            ))),
            ..
        }
    ));
    assert_eq!(
        services.issues.projection().unwrap().state.config.mode,
        IssueCollectionMode::Off
    );
    assert!(sim.effects.take().is_empty());
    assert!(sim.host.held_stored_answers.is_empty());
    drop(sim);
    assert!(
        !serde_json::to_string(&*log.0.lock().unwrap())
            .unwrap()
            .contains(REFERENCE)
    );
}
