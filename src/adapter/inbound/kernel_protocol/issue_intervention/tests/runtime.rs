//! 가짜 저장 포트·모델 런타임으로 커널의 확인 및 재전송 게이트를 검증한다.
use super::*;
use crate::application::port::inbound::stored_command_port::{
    StoredCommandOutcome, StoredCommandPort,
};
use crate::application::port::outbound::kernel_event_log_port::KernelEventLogPort;
use crate::composition::native_client_runtime::tests::recording_runtime_for_test;
use crate::composition::native_client_runtime::{
    ClientCommandDispatch, KernelClientCommand, StoredCommandEnd,
};
use crate::domain::kernel_event::{KernelEvent, RecordedKernelEvent};
use std::sync::{Arc, Mutex};

#[derive(Default)]
struct MemoryLog(Mutex<Vec<RecordedKernelEvent>>);
impl KernelEventLogPort for MemoryLog {
    fn append_kernel_events(&self, _: &str, records: &[RecordedKernelEvent]) -> anyhow::Result<()> {
        self.0.lock().unwrap().extend_from_slice(records);
        Ok(())
    }
    fn last_kernel_event_sequence(&self, _: &str) -> anyhow::Result<i64> {
        Ok(self
            .0
            .lock()
            .unwrap()
            .last()
            .map_or(0, |record| record.sequence))
    }
    fn load_kernel_events(&self, _: &str, kinds: &[&str]) -> anyhow::Result<Vec<KernelEvent>> {
        Ok(self
            .0
            .lock()
            .unwrap()
            .iter()
            .filter(|record| kinds.contains(&record.event.kind()))
            .map(|record| record.event.clone())
            .collect())
    }
}

#[derive(Default)]
struct Executor(Mutex<Vec<StoredCommand>>);
impl StoredCommandPort for Executor {
    fn execute(&self, command: StoredCommand) -> anyhow::Result<StoredCommandOutcome> {
        self.0.lock().unwrap().push(command.clone());
        let StoredCommand::InterveneIssue { request } = command else {
            panic!("only intervention commands should reach this executor");
        };
        let mut receipt = receipt(request.action.label());
        receipt.command_id = request.command_id;
        if matches!(request.action, IssueInterventionAction::Steer { .. }) {
            receipt.delivery = Some(IssueSteerDelivery::Unknown);
        }
        Ok(StoredCommandOutcome::Done(
            StoredCommandResult::IssueIntervened {
                receipt: Box::new(receipt),
            },
        ))
    }
}

fn encoded(
    action: IssueInterventionAction,
    key: &str,
) -> (
    KernelClientCommand,
    crate::domain::kernel_event::KernelCommandDescriptor,
) {
    let request = CommandRequest {
        command: command_from_request(request(action)),
        idempotency_key: Some(key.to_owned()),
    };
    let descriptor = request.descriptor().unwrap().unwrap();
    let KernelCommandInput::Stored(stored) = request.command.into_input("/workspace").unwrap()
    else {
        panic!("interventions route to storage");
    };
    (KernelClientCommand::Stored(Box::new(stored)), descriptor)
}

#[test]
fn intervention_cancel_confirmation_gate_never_invokes_the_executor_without_confirm() {
    let (mut runtime, turns) = recording_runtime_for_test();
    let log = Arc::new(MemoryLog::default());
    let executor = Arc::new(Executor::default());
    runtime.attach_kernel_event_log(log.clone(), "/workspace");
    runtime.attach_stored_commands(executor.clone(), "/workspace");
    let (command, descriptor) = encoded(
        IssueInterventionAction::Cancel { confirm: false },
        "cancel-key",
    );
    assert!(matches!(
        runtime.dispatch_client_command(command, Some(&descriptor)),
        ClientCommandDispatch::ConfirmationRequired
    ));
    assert!(executor.0.lock().unwrap().is_empty());
    assert!(log.0.lock().unwrap().is_empty());
    let (command, descriptor) = encoded(
        IssueInterventionAction::Cancel { confirm: true },
        "confirmed-key",
    );
    assert!(matches!(
        runtime.dispatch_client_command(command, Some(&descriptor)),
        ClientCommandDispatch::Ran {
            stored: Some(StoredCommandEnd::Finished(StoredCommandOutcome::Done(_))),
            ..
        }
    ));
    assert_eq!(executor.0.lock().unwrap().len(), 1);
    assert!(turns.lock().unwrap().is_empty());
}

#[test]
fn intervention_steer_unknown_receipt_is_not_resent_on_retry_or_kernel_restart() {
    let (mut runtime, turns) = recording_runtime_for_test();
    let log = Arc::new(MemoryLog::default());
    let executor = Arc::new(Executor::default());
    runtime.attach_kernel_event_log(log.clone(), "/workspace");
    runtime.attach_stored_commands(executor.clone(), "/workspace");
    let (command, descriptor) = encoded(
        IssueInterventionAction::Steer {
            message: "raw-runtime-steer-canary".to_owned(),
        },
        "steer-key",
    );
    let ClientCommandDispatch::Ran {
        stored: Some(StoredCommandEnd::Finished(StoredCommandOutcome::Done(result))),
        ..
    } = runtime.dispatch_client_command(command.clone(), Some(&descriptor))
    else {
        panic!("first steer reaches storage");
    };
    let wire = serde_json::to_value(CommandResult::from_stored(result)).unwrap();
    assert_eq!(wire["receipt"]["delivery"], "unknown");
    assert!(matches!(
        runtime.dispatch_client_command(command.clone(), Some(&descriptor)),
        ClientCommandDispatch::Replayed(_)
    ));
    drop(runtime);
    let (mut reopened, restarted_turns) = recording_runtime_for_test();
    reopened.attach_kernel_event_log(log.clone(), "/workspace");
    reopened.attach_stored_commands(executor.clone(), "/workspace");
    assert!(matches!(
        reopened.dispatch_client_command(command, Some(&descriptor)),
        ClientCommandDispatch::Replayed(_)
    ));
    assert_eq!(executor.0.lock().unwrap().len(), 1);
    assert!(turns.lock().unwrap().is_empty());
    assert!(restarted_turns.lock().unwrap().is_empty());
    assert!(
        !serde_json::to_string(&*log.0.lock().unwrap())
            .unwrap()
            .contains("raw-runtime-steer-canary")
    );
}
