//! A single stored command, under the repository lock. This graph has no runtime,
//! app-server, scheduler, prompt recovery, or restart-closure path.

use std::path::Path;
use std::sync::Arc;

use anyhow::{Context, Result, bail};
use serde_json::Value;

use crate::adapter::inbound::kernel_protocol::{
    CommandRejection, CommandRequest, CommandResponse, KernelCommand, KernelProtocolError,
    KernelQuery, KernelQueryAnswer, ResetTarget, reason_next,
};
use crate::adapter::outbound::db::SqlitePlanningAuthorityAdapter;
use crate::adapter::outbound::filesystem::{
    FilesystemPlanningWorkspaceAdapter, RepositoryKernelLock, RepositoryKernelLockAdapter,
    RepositoryKernelLockHeld, RepositoryKernelOwner,
};
use crate::adapter::outbound::gateway_kernel_client::GatewayKernelClientAdapter;
use crate::application::port::inbound::kernel_command_port::KernelCommandPort;
use crate::application::port::outbound::kernel_event_log_port::KernelEventLogPort;
use crate::application::port::outbound::planning_reset_route_port::{
    PlanningResetRouteOutcome, PlanningResetRoutePort,
};
use crate::application::service::kernel_command::{KernelCommandService, UnreachableAnswers};
use crate::application::service::planning::PlanningBootstrapService;
use crate::application::service::planning::repair::reset::PlanningResetService;
use crate::application::service::planning::runtime::validation::PlanningValidationService;
use crate::domain::kernel_event::{
    COMMAND_KINDS, CommandMemory, CommandRecall, KernelEvent, MAX_KERNEL_EVENT_PAGE, OperationEnd,
    OperationReceipt, OperationStatus, RecordedKernelEvent, command_failure_detail,
};
use crate::domain::kernel_spec::{
    CommandEffect, KernelCallError, KernelCommandKind, OperationState, ReasonCode,
};
use crate::domain::planning::{
    PlanningAuthorityLocation, PlanningResetRequest, PlanningResetTarget, PriorityQueueService,
};

/// 커널 기록의 호출자 이름이다 (설계 §3.5). 입구마다 하나이다.
pub(crate) const CALLER_CLI: &str = "akra-cli";
pub(crate) const CALLER_ADMIN: &str = "akra-admin";
pub(crate) const CALLER_TELEGRAM: &str = "akra-telegram";

/// `workspace` 가 속한 저장소의 커널에 명령을 보내는 port 이다. `caller` 는 그 명령의 기록에 남는 입구의 이름이다.
pub(crate) fn build(workspace: &Path, caller: &'static str) -> Result<Box<dyn KernelCommandPort>> {
    let workspace = workspace
        .canonicalize()
        .with_context(|| format!("cannot select workspace `{}`", workspace.display()))?;
    if !workspace.is_dir() {
        bail!("workspace must be a directory");
    }
    let target = super::production::repository_kernel_target(&workspace.to_string_lossy())?;
    let local = Arc::new(CommandOnlyKernel {
        workspace: target.workspace_root,
        caller,
    });
    Ok(Box::new(KernelCommandService::new(
        Arc::new(GatewayKernelClientAdapter::new(
            workspace.to_string_lossy().into_owned(),
            caller.to_string(),
        )),
        local,
        UnreachableAnswers {
            no_gateway: unreachable_answer("The Akra Gateway is not running. Start it.")?,
            held_elsewhere: unreachable_answer(
                "Another process (a TUI) runs the kernel of this repository. Use that process, or close it.",
            )?,
            gateway_too_old: unreachable_answer(
                "The running Akra Gateway is an older version than this program. Restart the Gateway.",
            )?,
        },
    )))
}

/// The rejection of a runtime command that no Gateway kernel can take. The reason is the same in
/// both cases; `message` carries the cause, so that the caller is not told to start a Gateway that runs.
fn unreachable_answer(cause: &str) -> Result<Value> {
    let reason = ReasonCode::KernelUnreachable;
    Ok(serde_json::to_value(CommandResponse {
        processed: true,
        rejection: Some(CommandRejection {
            reason,
            message: format!("{} Detail: {cause}", reason.summary()),
            next: reason_next(reason),
        }),
        operation: None,
    })?)
}

struct CommandOnlyKernel {
    workspace: String,
    caller: &'static str,
}

impl CommandOnlyKernel {
    fn reset(&self, target: ResetTarget, confirm: bool) -> Result<()> {
        let database = Arc::new(SqlitePlanningAuthorityAdapter::new());
        let workspace = Arc::new(FilesystemPlanningWorkspaceAdapter::with_repo_scoped_store(
            database.clone(),
        ));
        let reset = PlanningResetService::with_task_repository(
            workspace,
            PlanningBootstrapService::new(),
            database.clone(),
            database,
            PlanningValidationService::new(),
            PriorityQueueService,
        );
        reset.reset_workspace(
            &self.workspace,
            PlanningResetRequest::new(
                match target {
                    ResetTarget::Queue => PlanningResetTarget::Queue,
                    ResetTarget::Directions => PlanningResetTarget::Directions,
                    ResetTarget::All => PlanningResetTarget::All,
                },
                confirm,
            ),
        )?;
        Ok(())
    }

    fn records(&self, log: &dyn KernelEventLogPort) -> Result<Vec<RecordedKernelEvent>> {
        let mut records = Vec::new();
        let mut after = 0;
        loop {
            let page =
                log.load_kernel_events_after(&self.workspace, after, MAX_KERNEL_EVENT_PAGE)?;
            if page.next_after <= after {
                return Ok(records);
            }
            after = page.next_after;
            records.extend(page.records);
        }
    }
}

impl KernelCommandPort for CommandOnlyKernel {
    fn command(
        &self,
        kind: KernelCommandKind,
        input: Value,
        idempotency_key: Option<String>,
    ) -> Result<Value> {
        let request = CommandRequest {
            command: serde_json::from_value(input)?,
            idempotency_key,
        };
        if request.command.kind() != kind {
            bail!("command kind does not match its input");
        }
        request
            .command
            .clone()
            .into_core(&self.workspace)
            .map_err(protocol_error)?;
        let descriptor = request.descriptor().map_err(protocol_error)?;
        if kind.spec().effect == CommandEffect::Destructive && !request.command.confirmed() {
            return Err(protocol_error(KernelProtocolError::confirmation_required(
                kind.name(),
            )));
        }
        let KernelCommand::ResetPlanning { target, confirm } = request.command else {
            bail!("the command-only kernel cannot execute {}", kind.name());
        };
        let location =
            SqlitePlanningAuthorityAdapter::prepare_private_runtime_directory(&self.workspace)?;
        let lock = match acquire_command_lock(location, &self.workspace) {
            Ok(lock) => Some(lock),
            Err(error)
                if error
                    .downcast_ref::<RepositoryKernelLockHeld>()
                    .is_some_and(|held| held.owner == Some(RepositoryKernelOwner::Tui)) =>
            {
                None
            }
            Err(error) => return Err(error),
        };
        // K6 exception: only this pre-existing direct operation is available under a TUI owner.
        if lock.is_none() {
            self.reset(target, confirm)?;
            return Ok(serde_json::to_value(CommandResponse::from_core(Some(&[])))?);
        }
        let mut descriptor = descriptor.context("stored mutation must have a descriptor")?;
        descriptor.caller = Some(self.caller.to_string());
        let log = super::production::kernel_event_log_port();
        let memory =
            CommandMemory::from_events(log.load_kernel_events(&self.workspace, &COMMAND_KINDS)?);
        if let Some(key) = &descriptor.idempotency_key {
            match memory.recall(key, &descriptor.fingerprint) {
                CommandRecall::Same(receipt) => {
                    return Ok(serde_json::to_value(CommandResponse::replayed(receipt))?);
                }
                CommandRecall::Different => {
                    return Err(protocol_error(KernelProtocolError::idempotency_key_reused()));
                }
                CommandRecall::Unknown => {}
            }
        }
        let op_id = log
            .last_kernel_event_sequence(&self.workspace)?
            .checked_add(1)
            .context("kernel event sequence exhausted")?;
        let completed_sequence = op_id
            .checked_add(1)
            .context("kernel event sequence exhausted")?;
        let record = |sequence, event| RecordedKernelEvent {
            sequence,
            recorded_at: chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
            event,
        };
        log.append_kernel_events(
            &self.workspace,
            &[record(
                op_id,
                KernelEvent::CommandReceived {
                    op_id,
                    command: kind.name().to_string(),
                    thread_id: descriptor.thread_id.clone(),
                    fingerprint: descriptor.fingerprint,
                    idempotency_key: descriptor.idempotency_key,
                    completion: Some(kind.spec().completion),
                    caller: descriptor.caller,
                },
            )],
        )?;
        let result = self.reset(target, confirm);
        log.append_kernel_events(
            &self.workspace,
            &[record(
                completed_sequence,
                KernelEvent::CommandCompleted {
                    op_id,
                    thread_id: descriptor.thread_id,
                    result: if result.is_ok() {
                        OperationEnd::Done
                    } else {
                        OperationEnd::Failed
                    },
                    reason: None,
                    detail: result
                        .as_ref()
                        .err()
                        .map(|error| command_failure_detail(&error.to_string())),
                },
            )],
        )?;
        result?;
        Ok(serde_json::to_value(
            CommandResponse::from_core(Some(&[])).with_operation(Some(OperationReceipt {
                op_id,
                rejection: None,
                replayed: false,
            })),
        )?)
    }

    fn query(&self, input: Value, wait: bool) -> Result<Value> {
        let query: KernelQuery = serde_json::from_value(input)?;
        let log = super::production::kernel_event_log_port();
        let answer = match query {
            KernelQuery::History { .. } => {
                let (after, limit) = query
                    .history_range()
                    .context("history query has no range")?;
                let page = log.load_kernel_events_after(&self.workspace, after, limit)?;
                if wait && page.records.is_empty() {
                    bail!("{UNREACHABLE_QUERY}: it cannot wait for new records");
                }
                KernelQueryAnswer::History(page)
            }
            KernelQuery::Operation { op_id } => {
                let status = OperationStatus::from_records(op_id, self.records(log.as_ref())?);
                if wait && !status.state.is_end() {
                    bail!("{UNREACHABLE_QUERY}: it cannot wait for this operation");
                }
                KernelQueryAnswer::Operation(status)
            }
            _ => bail!("{UNREACHABLE_QUERY}: it cannot read live kernel state"),
        };
        Ok(serde_json::to_value(answer)?)
    }
}

/*
 * 입구의 planning 초기화를 저장소 커널의 명령으로 보낸다 (plan 18 K4-9a). 입구마다 하나를 만든다. 명령은 Gateway 의
 * 커널, 명령 전용 조립, 또는 TUI 가 커널을 쥔 저장소의 직접 경로 중 하나로 간다 (설계 §3.7). 어느 길이든 이 값은
 * 초기화가 끝난 뒤에 돌아온다.
 */
pub(crate) struct KernelRoutedPlanningReset {
    caller: &'static str,
}

impl KernelRoutedPlanningReset {
    pub(crate) fn new(caller: &'static str) -> Self {
        Self { caller }
    }
}

impl PlanningResetRoutePort for KernelRoutedPlanningReset {
    fn reset_workspace(
        &self,
        workspace_dir: &str,
        target: PlanningResetTarget,
    ) -> Result<PlanningResetRouteOutcome> {
        // 커널이 있을 수 없는 폴더(대표 checkout 이 없는 저장소, 병렬 slot worktree)는 이전처럼 직접 초기화한다.
        if super::production::repository_kernel_target(workspace_dir).is_err() {
            return Ok(PlanningResetRouteOutcome::NoRepositoryKernel);
        }
        let port = build(Path::new(workspace_dir), self.caller)?;
        reset_through_kernel(port.as_ref(), target, new_idempotency_key())?;
        Ok(PlanningResetRouteOutcome::Done)
    }
}

/// 명령 하나의 멱등 키이다. 무작위 16바이트이다.
fn new_idempotency_key() -> String {
    use rand::RngCore;
    let mut bytes = [0_u8; 16];
    rand::thread_rng().fill_bytes(&mut bytes);
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/*
 * `reset_planning` 을 보내고 그 연산의 끝을 기다린다. 입구의 확인 규칙(큐는 확인이 없어도 된다)은 부른 쪽이 이미
 * 검사했다. 커널은 지우는 명령마다 `confirm: true` 를 요구하므로 여기서 붙인다.
 *
 * 멱등 키를 보내는 이유는 재전송이 아니다. 기록이 없는 Gateway 커널은 명령을 기록 없이 실행하고 영수증을 주지
 * 않는다. 그러면 끝을 기다릴 수 없다. 키가 있으면 그 커널은 실행하지 않고 오류로 답한다 (`idempotency_unavailable`).
 * 그래서 영수증이 없는 답은 TUI 가 커널을 쥔 저장소의 직접 초기화뿐이고, 그 길은 초기화가 끝난 뒤에 답한다.
 */
fn reset_through_kernel(
    port: &dyn KernelCommandPort,
    target: PlanningResetTarget,
    idempotency_key: String,
) -> Result<()> {
    let command = KernelCommand::ResetPlanning {
        target: match target {
            PlanningResetTarget::Queue => ResetTarget::Queue,
            PlanningResetTarget::Directions => ResetTarget::Directions,
            PlanningResetTarget::All => ResetTarget::All,
        },
        confirm: true,
    };
    let answer: CommandResponse = serde_json::from_value(port.command(
        command.kind(),
        serde_json::to_value(&command)?,
        Some(idempotency_key),
    )?)
    .context("the kernel answered the reset with an unknown shape")?;
    if let Some(rejection) = answer.rejection {
        bail!("{}", rejection.message);
    }
    if !answer.processed {
        bail!("the repository kernel stopped before it took the reset; nothing was changed");
    }
    let Some(operation) = answer.operation else {
        return Ok(());
    };
    let status = port.query(
        serde_json::to_value(KernelQuery::Operation {
            op_id: operation.op_id,
        })?,
        true,
    )?;
    let KernelQueryAnswer::Operation(status) = serde_json::from_value(status)
        .context("the kernel answered the operation query with an unknown shape")?
    else {
        bail!("the kernel did not answer the operation query with an operation");
    };
    match status.state {
        OperationState::Done => Ok(()),
        state => Err(reset_failure(state, &status.records)),
    }
}

/*
 * 명령 전용 조립의 잠금이다. 다른 입구의 명령이 같은 잠금을 잠깐 쥐고 있으면 조금 기다린다. 이전의 직접 초기화는
 * 저장소(DB)의 잠금에서 기다렸다. 커널(Gateway, TUI)이 쥔 잠금은 기다려도 풀리지 않으므로 바로 돌려준다.
 */
fn acquire_command_lock(
    location: PlanningAuthorityLocation,
    workspace: &str,
) -> Result<RepositoryKernelLock> {
    const ATTEMPTS: u32 = 50;
    const DELAY: std::time::Duration = std::time::Duration::from_millis(100);
    let mut attempt = 1;
    loop {
        let acquired = RepositoryKernelLockAdapter::acquire_for_owner(
            location.clone(),
            workspace,
            RepositoryKernelOwner::Command,
        );
        let held_by_a_command = acquired.as_ref().err().is_some_and(|error| {
            error
                .downcast_ref::<RepositoryKernelLockHeld>()
                .is_some_and(|held| held.owner == Some(RepositoryKernelOwner::Command))
        });
        if !held_by_a_command || attempt == ATTEMPTS {
            return acquired;
        }
        attempt += 1;
        std::thread::sleep(DELAY);
    }
}

/// 끝난 초기화가 성공이 아닐 때의 오류이다. 커널이 댄 원인(`command_completed` 의 `detail`)이 있으면 그 글이다.
fn reset_failure(state: OperationState, records: &[RecordedKernelEvent]) -> anyhow::Error {
    let detail = records.iter().find_map(|record| match &record.event {
        KernelEvent::CommandCompleted { detail, .. } => detail.clone(),
        _ => None,
    });
    match detail {
        Some(detail) => anyhow::anyhow!(detail),
        None => anyhow::anyhow!(
            "the planning reset did not finish (operation state: {}); read the planning state before sending it again",
            state.name()
        ),
    }
}

/// The start of the error of a query that only a running Gateway kernel can answer.
const UNREACHABLE_QUERY: &str = "kernel_unreachable: no kernel that the Akra Gateway runs answers this query (the Gateway is not running, or a TUI runs the kernel)";

fn protocol_error(error: KernelProtocolError) -> anyhow::Error {
    KernelCallError {
        code: error.code,
        message: error.message,
    }
    .into()
}

#[cfg(test)]
mod g2_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::port::outbound::planning_worker_port::NoopPlanningWorkerPort;
    use crate::application::service::planning::PlanningServices;
    use crate::domain::kernel_event::QueuedThreadPrompt;
    use crate::domain::kernel_spec::OperationState;
    use crate::domain::planning::ManualPromptMainSessionHandoff;
    use serde_json::json;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    struct Workspace(PathBuf);
    impl Drop for Workspace {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn fixture() -> (Workspace, CommandOnlyKernel) {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = crate::test_utils::platform_safe_temp_dir().join(format!(
            "kernel-command-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&path).unwrap();
        let workspace = path.to_string_lossy().into_owned();
        let db = Arc::new(SqlitePlanningAuthorityAdapter::new());
        let services = PlanningServices::from_ports(
            Arc::new(FilesystemPlanningWorkspaceAdapter::with_repo_scoped_store(
                db.clone(),
            )),
            db.clone(),
            db,
            Arc::new(NoopPlanningWorkerPort),
        );
        services
            .workspace
            .initialize_simple_workspace(&workspace)
            .unwrap();
        (
            Workspace(path),
            CommandOnlyKernel {
                workspace,
                caller: CALLER_CLI,
            },
        )
    }

    /// A port that answers with prepared values. It stands for the Gateway kernel.
    struct ScriptedKernel {
        answer: Value,
        operation: Value,
        sent: std::sync::Mutex<Vec<Value>>,
    }

    impl ScriptedKernel {
        fn new(answer: CommandResponse, operation: Option<OperationStatus>) -> Self {
            Self {
                answer: serde_json::to_value(answer).unwrap(),
                operation: operation
                    .map(|status| {
                        serde_json::to_value(KernelQueryAnswer::Operation(status)).unwrap()
                    })
                    .unwrap_or(Value::Null),
                sent: std::sync::Mutex::new(Vec::new()),
            }
        }
    }

    impl KernelCommandPort for ScriptedKernel {
        fn command(
            &self,
            _: KernelCommandKind,
            input: Value,
            key: Option<String>,
        ) -> Result<Value> {
            // With a key, a kernel without a decision log refuses instead of running without a receipt.
            assert_eq!(key.as_deref(), Some("key"));
            self.sent.lock().unwrap().push(input);
            Ok(self.answer.clone())
        }

        fn query(&self, input: Value, wait: bool) -> Result<Value> {
            assert!(wait, "the entrance waits for the end of the reset");
            self.sent.lock().unwrap().push(input);
            Ok(self.operation.clone())
        }
    }

    fn accepted(op_id: i64) -> CommandResponse {
        CommandResponse::from_core(Some(&[])).with_operation(Some(OperationReceipt {
            op_id,
            rejection: None,
            replayed: false,
        }))
    }

    fn ended(op_id: i64, result: OperationEnd, detail: Option<&str>) -> OperationStatus {
        let record = |sequence, event| RecordedKernelEvent {
            sequence,
            recorded_at: "now".to_string(),
            event,
        };
        OperationStatus::from_records(
            op_id,
            [
                record(
                    op_id,
                    KernelEvent::CommandReceived {
                        op_id,
                        command: KernelCommandKind::ResetPlanning.name().to_string(),
                        thread_id: String::new(),
                        fingerprint: "fingerprint".to_string(),
                        idempotency_key: None,
                        completion: Some(KernelCommandKind::ResetPlanning.spec().completion),
                        caller: Some(CALLER_CLI.to_string()),
                    },
                ),
                record(
                    op_id + 1,
                    KernelEvent::CommandCompleted {
                        op_id,
                        thread_id: String::new(),
                        result,
                        reason: None,
                        detail: detail.map(str::to_string),
                    },
                ),
            ],
        )
    }

    #[test]
    fn a_routed_reset_waits_for_the_end_and_reports_the_cause_of_a_failure() {
        // plan 18 K4-9a: the Gateway kernel accepts the command first. The entrance returns after the end.
        let done = ScriptedKernel::new(accepted(5), Some(ended(5, OperationEnd::Done, None)));
        reset_through_kernel(&done, PlanningResetTarget::Queue, "key".to_string())
            .expect("the reset ended");
        assert_eq!(
            *done.sent.lock().unwrap(),
            [
                // The entrance rule lets a queue reset pass without a confirmation. The kernel rule needs one.
                json!({"type":"reset_planning","target":"queue","confirm":true}),
                json!({"type":"operation","op_id":5}),
            ]
        );

        let failed = ScriptedKernel::new(
            accepted(5),
            Some(ended(
                5,
                OperationEnd::Failed,
                Some("live tasks block the directions reset"),
            )),
        );
        assert_eq!(
            reset_through_kernel(&failed, PlanningResetTarget::Directions, "key".to_string())
                .unwrap_err()
                .to_string(),
            "live tasks block the directions reset",
            "the entrance shows the cause that the kernel recorded"
        );

        let lost = ScriptedKernel::new(accepted(5), Some(ended(5, OperationEnd::Lost, None)));
        let error = reset_through_kernel(&lost, PlanningResetTarget::All, "key".to_string())
            .unwrap_err()
            .to_string();
        assert!(error.contains("operation state: lost"), "{error}");
    }

    #[test]
    fn a_routed_reset_reports_a_rejection_and_accepts_the_path_without_records() {
        let reason = ReasonCode::KernelUnreachable;
        let rejected = ScriptedKernel::new(
            CommandResponse {
                processed: true,
                rejection: Some(CommandRejection {
                    reason,
                    message: "the kernel refused".to_string(),
                    next: reason_next(reason),
                }),
                operation: None,
            },
            None,
        );
        assert_eq!(
            reset_through_kernel(&rejected, PlanningResetTarget::Queue, "key".to_string())
                .unwrap_err()
                .to_string(),
            "the kernel refused"
        );
        assert_eq!(rejected.sent.lock().unwrap().len(), 1, "no operation query");

        // A repository whose kernel a TUI holds: the direct reset ends before it answers, without a receipt.
        let direct = ScriptedKernel::new(CommandResponse::from_core(Some(&[])), None);
        reset_through_kernel(&direct, PlanningResetTarget::Queue, "key".to_string())
            .expect("the direct reset ended");
        assert_eq!(direct.sent.lock().unwrap().len(), 1, "no operation query");

        let stopped = ScriptedKernel::new(CommandResponse::from_core(None), None);
        assert!(
            reset_through_kernel(&stopped, PlanningResetTarget::Queue, "key".to_string()).is_err()
        );
    }

    #[test]
    fn an_entrance_reset_without_a_gateway_is_recorded_with_its_caller() {
        // The real command-only kernel: the reset of an entrance leaves `command_received` and `command_completed`.
        let (_directory, mut kernel) = fixture();
        kernel.caller = CALLER_ADMIN;
        reset_through_kernel(&kernel, PlanningResetTarget::Queue, "key".to_string())
            .expect("the reset ended");
        let log = super::super::production::kernel_event_log_port();
        let records = kernel.records(log.as_ref()).unwrap();
        assert!(matches!(
            &records[0].event,
            KernelEvent::CommandReceived { command, caller, .. }
                if command == "reset_planning" && caller.as_deref() == Some("akra-admin")
        ));
        assert!(matches!(
            records[1].event,
            KernelEvent::CommandCompleted {
                result: OperationEnd::Done,
                detail: None,
                ..
            }
        ));
    }

    #[test]
    fn a_failed_reset_without_a_gateway_keeps_its_cause_in_the_record_and_in_the_error() {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        // No planning workspace was initialized here. The reset service refuses.
        let path = crate::test_utils::platform_safe_temp_dir().join(format!(
            "kernel-command-empty-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&path).unwrap();
        let _directory = Workspace(path.clone());
        let kernel = CommandOnlyKernel {
            workspace: path.to_string_lossy().into_owned(),
            caller: CALLER_TELEGRAM,
        };
        let error = reset_through_kernel(&kernel, PlanningResetTarget::Queue, "key".to_string())
            .unwrap_err()
            .to_string();
        assert!(error.contains("initialize planning first"), "{error}");
        let log = super::super::production::kernel_event_log_port();
        let records = kernel.records(log.as_ref()).unwrap();
        let KernelEvent::CommandCompleted { result, detail, .. } = &records[1].event else {
            panic!("the second record is the end of the command: {records:?}");
        };
        assert_eq!(*result, OperationEnd::Failed);
        assert!(
            detail
                .as_deref()
                .is_some_and(|detail| detail.contains("initialize planning first")),
            "{detail:?}"
        );
    }

    fn reset(kernel: &CommandOnlyKernel, key: &str) -> Result<Value> {
        kernel.command(
            KernelCommandKind::ResetPlanning,
            json!({"type":"reset_planning","target":"queue","confirm":true}),
            Some(key.to_string()),
        )
    }

    #[test]
    fn command_only_execution_flushes_logs_replays_and_preserves_open_work() {
        let (_directory, kernel) = fixture();
        let log = super::super::production::kernel_event_log_port();
        let queued = KernelEvent::ThreadPromptQueued(QueuedThreadPrompt {
            thread_id: "thread".to_string(),
            workspace_directory: kernel.workspace.clone(),
            handoff: ManualPromptMainSessionHandoff {
                prompt: "already prepared".to_string(),
                transcript_text: "original".to_string(),
                task: None,
            },
            task_id: "task".to_string(),
            op_id: Some(1),
        });
        let existing = [
            KernelEvent::CommandReceived {
                op_id: 1,
                command: "submit_prompt".to_string(),
                thread_id: "thread".to_string(),
                fingerprint: "old".to_string(),
                idempotency_key: None,
                completion: Some(crate::domain::kernel_spec::CommandCompletion::Records),
                caller: None,
            },
            queued,
        ];
        let seed = existing
            .into_iter()
            .enumerate()
            .map(|(i, event)| RecordedKernelEvent {
                sequence: i as i64 + 1,
                recorded_at: "now".to_string(),
                event,
            })
            .collect::<Vec<_>>();
        log.append_kernel_events(&kernel.workspace, &seed).unwrap();
        let first = reset(&kernel, "reset-key").unwrap();
        assert_eq!(first["operation"]["op_id"], 3);
        let records = kernel.records(log.as_ref()).unwrap();
        assert_eq!(
            records.len(),
            4,
            "only received and completed are added; no recovery, lost closure, or turns"
        );
        assert_eq!(&records[..2], seed.as_slice());
        assert!(matches!(
            records[3].event,
            KernelEvent::CommandCompleted {
                result: OperationEnd::Done,
                ..
            }
        ));
        assert_eq!(
            OperationStatus::from_records(1, records.clone()).state,
            OperationState::Running
        );
        assert_eq!(
            OperationStatus::from_records(3, records.clone()).state,
            OperationState::Done
        );
        assert_eq!(
            reset(&kernel, "reset-key").unwrap()["operation"]["replayed"],
            true
        );
        assert_eq!(kernel.records(log.as_ref()).unwrap(), records);
        let different = kernel.command(
            KernelCommandKind::ResetPlanning,
            json!({"type":"reset_planning","target":"all","confirm":true}),
            Some("reset-key".to_string()),
        );
        assert!(different.unwrap_err().to_string().contains("-32014"));
        let location =
            SqlitePlanningAuthorityAdapter::prepare_private_runtime_directory(&kernel.workspace)
                .unwrap();
        let _next = RepositoryKernelLockAdapter::acquire_for_owner(
            location,
            &kernel.workspace,
            RepositoryKernelOwner::Command,
        )
        .unwrap();
    }

    #[test]
    fn command_owner_blocks_mutation_and_tui_uses_only_legacy_reset() {
        let (_directory, kernel) = fixture();
        let location =
            SqlitePlanningAuthorityAdapter::prepare_private_runtime_directory(&kernel.workspace)
                .unwrap();
        let command = RepositoryKernelLockAdapter::acquire_for_owner(
            location.clone(),
            &kernel.workspace,
            RepositoryKernelOwner::Command,
        )
        .unwrap();
        assert!(reset(&kernel, "blocked").is_err());
        let log = super::super::production::kernel_event_log_port();
        assert!(kernel.records(log.as_ref()).unwrap().is_empty());
        drop(command);
        let _tui = RepositoryKernelLockAdapter::acquire_for_owner(
            location,
            &kernel.workspace,
            RepositoryKernelOwner::Tui,
        )
        .unwrap();
        let answer = reset(&kernel, "legacy").unwrap();
        assert!(answer.get("operation").is_none());
        assert!(kernel.records(log.as_ref()).unwrap().is_empty());
    }

    #[test]
    fn offline_queries_report_persisted_operation_and_do_not_close_it() {
        let (_directory, kernel) = fixture();
        let answer = reset(&kernel, "query").unwrap();
        let op_id = answer["operation"]["op_id"].as_i64().unwrap();
        let status = kernel
            .query(json!({"type":"operation","op_id":op_id}), true)
            .unwrap();
        assert_eq!(status["state"], "done");
        let history = kernel
            .query(json!({"type":"history","after":0}), true)
            .unwrap();
        assert_eq!(history["records"].as_array().unwrap().len(), 2);
        assert!(
            kernel
                .query(json!({"type":"history","after":2}), true)
                .is_err()
        );
        let unknown = kernel
            .query(json!({"type":"operation","op_id":999}), true)
            .unwrap();
        assert_eq!(unknown["state"], "unknown");
    }
}
