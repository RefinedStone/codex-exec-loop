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
    FilesystemPlanningWorkspaceAdapter, RepositoryKernelLockAdapter, RepositoryKernelLockHeld,
    RepositoryKernelOwner,
};
use crate::adapter::outbound::gateway_kernel_client::GatewayKernelClientAdapter;
use crate::application::port::inbound::kernel_command_port::KernelCommandPort;
use crate::application::port::outbound::kernel_event_log_port::KernelEventLogPort;
use crate::application::service::kernel_command::KernelCommandService;
use crate::application::service::planning::PlanningBootstrapService;
use crate::application::service::planning::repair::reset::PlanningResetService;
use crate::application::service::planning::runtime::validation::PlanningValidationService;
use crate::domain::kernel_event::{
    COMMAND_KINDS, CommandMemory, CommandRecall, KernelEvent, MAX_KERNEL_EVENT_PAGE, OperationEnd,
    OperationReceipt, OperationStatus, RecordedKernelEvent,
};
use crate::domain::kernel_spec::{CommandEffect, KernelCommandKind, ReasonCode};
use crate::domain::planning::{PlanningResetRequest, PlanningResetTarget, PriorityQueueService};

pub(crate) fn build(workspace: &Path) -> Result<Box<dyn KernelCommandPort>> {
    let workspace = workspace
        .canonicalize()
        .with_context(|| format!("cannot select workspace `{}`", workspace.display()))?;
    if !workspace.is_dir() {
        bail!("workspace must be a directory");
    }
    let target = super::production::repository_kernel_target(&workspace.to_string_lossy())?;
    let local = Arc::new(CommandOnlyKernel {
        workspace: target.workspace_root,
    });
    let reason = ReasonCode::KernelNotRunning;
    let not_running = serde_json::to_value(CommandResponse {
        processed: true,
        rejection: Some(CommandRejection {
            reason,
            message: reason.summary().to_string(),
            next: reason_next(reason),
        }),
        operation: None,
    })?;
    Ok(Box::new(KernelCommandService::new(
        Arc::new(GatewayKernelClientAdapter::new(
            workspace.to_string_lossy().into_owned(),
            "akra-cli".to_string(),
        )),
        local,
        not_running,
    )))
}

struct CommandOnlyKernel {
    workspace: String,
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
        let lock = match RepositoryKernelLockAdapter::acquire_for_owner(
            location,
            &self.workspace,
            RepositoryKernelOwner::Command,
        ) {
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
        descriptor.caller = Some("akra-cli".to_string());
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
                    bail!("kernel_not_running: start the Akra Gateway to wait for new records");
                }
                KernelQueryAnswer::History(page)
            }
            KernelQuery::Operation { op_id } => {
                let status = OperationStatus::from_records(op_id, self.records(log.as_ref())?);
                if wait && !status.state.is_end() {
                    bail!("kernel_not_running: start the Akra Gateway to wait for this operation");
                }
                KernelQueryAnswer::Operation(status)
            }
            _ => bail!("kernel_not_running: start the Akra Gateway to query live kernel state"),
        };
        Ok(serde_json::to_value(answer)?)
    }
}

fn protocol_error(error: KernelProtocolError) -> anyhow::Error {
    anyhow::anyhow!("kernel protocol error {}: {}", error.code, error.message)
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
        (Workspace(path), CommandOnlyKernel { workspace })
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
