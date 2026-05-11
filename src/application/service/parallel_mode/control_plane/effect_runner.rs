use std::sync::Arc;
use std::sync::mpsc;
use std::thread;

use crate::application::port::outbound::parallel_agent_worker_port::ParallelAgentWorkerPort;
use crate::application::service::parallel_mode::turn::ParallelModeTurnService;
use crate::application::service::parallel_mode::{
    ParallelModeDispatchOrchestratorTickRequest, ParallelModeOrchestratorLoopEvent,
    ParallelModeOrchestratorTrigger, ParallelModeService,
};
use crate::application::service::planning::PlanningServices;
use crate::diagnostics::event_log;
use crate::domain::parallel_mode::{
    ParallelModeControlPlaneWorkerEvent, ParallelModeDispatchOutcome,
    ParallelModeOrchestratorStateMachine, ParallelModePoolResetPolicy, ParallelModePoolResetReport,
    ParallelModePoolResetRunId, ParallelModePoolResetScope, ParallelModePostTurnQueueSignal,
    ParallelModeReadinessSnapshot, ParallelModeRuntimeEvent, ParallelModeSupervisorSnapshot,
};

use super::{
    ParallelModeControlPlaneCommand, ParallelModeControlPlaneEffectId, ParallelModeControlPlaneWake,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParallelModeControlPlaneLoadingStage {
    ReconcilingPool,
}

#[derive(Debug, Clone)]
pub enum ParallelModeControlPlaneBackgroundEvent {
    EnterProgress {
        workspace_directory: String,
        readiness_snapshot: Option<ParallelModeReadinessSnapshot>,
        loading_stage: ParallelModeControlPlaneLoadingStage,
        status_text: String,
    },
    Entered {
        workspace_directory: String,
        epoch_id: u64,
        effect_id: ParallelModeControlPlaneEffectId,
        mode_was_enabled: bool,
        readiness_snapshot: ParallelModeReadinessSnapshot,
        supervisor_snapshot: Box<ParallelModeSupervisorSnapshot>,
        status_text: String,
        initial_pool_reset_completed: bool,
        has_actionable_queue_head: bool,
        orchestrator_tick_signature: Option<String>,
    },
    SupervisorSnapshotRefreshed {
        workspace_directory: String,
        epoch_id: u64,
        effect_id: ParallelModeControlPlaneEffectId,
        supervisor_snapshot: Box<ParallelModeSupervisorSnapshot>,
        orchestrator_tick_signature: Option<String>,
    },
    OrchestratorWakeCompleted {
        workspace_directory: String,
        effect_id: ParallelModeControlPlaneEffectId,
        readiness_snapshot: ParallelModeReadinessSnapshot,
        supervisor_snapshot: Box<ParallelModeSupervisorSnapshot>,
        outcome: ParallelModeDispatchOutcome,
        orchestrator_tick_signature: Option<String>,
    },
    WorkerEvent {
        event: ParallelModeControlPlaneWorkerEvent,
        has_actionable_queue_head: bool,
    },
    ConversationRuntimeNotice(String),
    OrchestratorTickCompleted {
        workspace_directory: String,
        epoch_id: u64,
        effect_id: ParallelModeControlPlaneEffectId,
        blocked: bool,
        notices: Vec<String>,
    },
}

pub trait ParallelModeControlPlaneEventSink: Clone + Send + 'static {
    fn send_control_plane_event(&self, event: ParallelModeControlPlaneBackgroundEvent);
}

#[derive(Clone)]
pub struct ParallelModeControlPlaneEffectRunner<S>
where
    S: ParallelModeControlPlaneEventSink,
{
    parallel_mode_service: ParallelModeService,
    planning: PlanningServices,
    worker_port: Arc<dyn ParallelAgentWorkerPort>,
    turn_service: ParallelModeTurnService,
    event_sink: S,
}

impl<S> ParallelModeControlPlaneEffectRunner<S>
where
    S: ParallelModeControlPlaneEventSink,
{
    pub fn new(
        parallel_mode_service: ParallelModeService,
        planning: PlanningServices,
        worker_port: Arc<dyn ParallelAgentWorkerPort>,
        turn_service: ParallelModeTurnService,
        event_sink: S,
    ) -> Self {
        Self {
            parallel_mode_service,
            planning,
            worker_port,
            turn_service,
            event_sink,
        }
    }

    pub fn spawn_supervisor_snapshot_refresh(
        &self,
        workspace_directory: String,
        readiness_snapshot: ParallelModeReadinessSnapshot,
        mode_enabled: bool,
        epoch_id: u64,
        effect_id: ParallelModeControlPlaneEffectId,
    ) {
        let parallel_mode_service = self.parallel_mode_service.clone();
        let event_sink = self.event_sink.clone();

        thread::spawn(move || {
            event_log::emit_lazy("parallel_supervisor_refresh_started", || {
                serde_json::json!({
                    "workspace_directory": &workspace_directory,
                    "mode_enabled": mode_enabled,
                })
            });
            let supervisor_snapshot = parallel_mode_service.build_supervisor_snapshot(
                &workspace_directory,
                mode_enabled,
                Some(&readiness_snapshot),
            );
            event_log::emit_lazy("parallel_supervisor_refresh_completed", || {
                serde_json::json!({
                    "workspace_directory": &workspace_directory,
                    "mode_enabled": mode_enabled,
                    "pool_status": &supervisor_snapshot.pool.reconcile_status,
                    "roster_active_count": supervisor_snapshot.roster.active_count(),
                })
            });
            event_sink.send_control_plane_event(
                ParallelModeControlPlaneBackgroundEvent::SupervisorSnapshotRefreshed {
                    workspace_directory,
                    epoch_id,
                    effect_id,
                    orchestrator_tick_signature: parallel_mode_distributor_tick_signature(
                        &supervisor_snapshot,
                    ),
                    supervisor_snapshot: Box::new(supervisor_snapshot),
                },
            );
        });
    }

    pub fn spawn_orchestrator_tick(
        &self,
        workspace_directory: String,
        signature: String,
        epoch_id: u64,
        effect_id: ParallelModeControlPlaneEffectId,
    ) {
        let parallel_mode_service = self.parallel_mode_service.clone();
        let event_sink = self.event_sink.clone();

        thread::spawn(move || {
            event_log::emit_lazy("parallel_orchestrator_retry_started", || {
                serde_json::json!({
                    "workspace": &workspace_directory,
                    "signature": &signature,
                    "trigger": "supervisor_active_distributor_queue",
                })
            });
            let (blocked, notices) = match parallel_mode_service.run_orchestrator_tick(
                &workspace_directory,
                ParallelModeOrchestratorTrigger::ManualDispatch,
            ) {
                Ok(result) => (result.blocked, result.notices),
                Err(error) => (
                    true,
                    vec![format!("orchestrator retry tick failed: {error}")],
                ),
            };
            event_log::emit_lazy("parallel_orchestrator_retry_completed", || {
                serde_json::json!({
                    "workspace": &workspace_directory,
                    "signature": &signature,
                    "blocked": blocked,
                    "notices_count": notices.len(),
                })
            });
            event_sink.send_control_plane_event(
                ParallelModeControlPlaneBackgroundEvent::OrchestratorTickCompleted {
                    workspace_directory,
                    epoch_id,
                    effect_id,
                    blocked,
                    notices,
                },
            );
        });
    }

    pub fn spawn_entry(
        &self,
        workspace_directory: String,
        epoch_id: u64,
        effect_id: ParallelModeControlPlaneEffectId,
        mode_was_enabled: bool,
        initial_pool_reset_required: bool,
    ) {
        let parallel_mode_service = self.parallel_mode_service.clone();
        let planning = self.planning.clone();
        let event_sink = self.event_sink.clone();

        thread::spawn(move || {
            let planning_projection = planning
                .runtime
                .load_runtime_projection_or_invalid(&workspace_directory);
            let has_actionable_queue_head = planning_projection.has_actionable_queue_head();
            let readiness_snapshot =
                parallel_mode_service.inspect_readiness(&workspace_directory, &planning_projection);
            let entry_decision = ParallelModeOrchestratorStateMachine::decide_parallel_entry(
                mode_was_enabled,
                readiness_snapshot.allows_parallel_mode(),
                initial_pool_reset_required,
            );
            let entry_plan = entry_decision.plan;
            event_log::emit_lazy("parallel_action_planned", || {
                serde_json::json!({
                    "workspace": &workspace_directory,
                    "state": entry_plan.state.label(),
                    "reset_scope": entry_plan.reset_scope.map(|scope| scope.label()),
                    "readiness": readiness_snapshot.readiness_label(),
                    "initial_setup_reset": initial_pool_reset_required,
                })
            });

            let initial_pool_reset_completed = initial_pool_reset_required
                && entry_plan.reset_scope == Some(ParallelModePoolResetScope::PoolOnly);
            let (supervisor_snapshot, status_text) = if readiness_snapshot.allows_parallel_mode() {
                event_sink.send_control_plane_event(
                    ParallelModeControlPlaneBackgroundEvent::EnterProgress {
                        workspace_directory: workspace_directory.clone(),
                        readiness_snapshot: Some(readiness_snapshot.clone()),
                        loading_stage: ParallelModeControlPlaneLoadingStage::ReconcilingPool,
                        status_text:
                            "parallel mode: loading 2/3 / readiness complete; reconciling pool"
                                .to_string(),
                    },
                );
                let reset_result = if entry_plan.reset_scope
                    == Some(ParallelModePoolResetScope::PoolOnly)
                {
                    event_log::emit_lazy("parallel_pool_reset_started", || {
                        serde_json::json!({
                            "workspace": &workspace_directory,
                            "reset_scope": ParallelModePoolResetScope::PoolOnly.label(),
                            "initial_setup_reset": initial_pool_reset_required,
                        })
                    });
                    let reset_report = match entry_decision.reset_policy {
                        Some(ParallelModePoolResetPolicy::ForceDisposable) => parallel_mode_service
                            .reset_pool_on_parallel_initial_setup_report(&workspace_directory),
                        Some(ParallelModePoolResetPolicy::ProtectLive) => parallel_mode_service
                            .reset_pool_on_parallel_enable_report(&workspace_directory),
                        None => Ok(ParallelModePoolResetReport::new(
                            ParallelModePoolResetRunId::new("no-reset"),
                            ParallelModePoolResetPolicy::ProtectLive,
                        )),
                    };
                    reset_report.and_then(|report| {
                            if report.has_live_blockers() {
                                event_log::emit_lazy("parallel_pool_reset_preserved_live", || {
                                    serde_json::json!({
                                        "workspace": &workspace_directory,
                                        "reset_scope": ParallelModePoolResetScope::PoolOnly.label(),
                                        "run_id": report.run_id.as_str(),
                                        "policy": report.policy,
                                        "live_blockers": report.live_blocker_count(),
                                    })
                                });
                            }
                            if report.has_reset_failures() {
                                return Err(format!(
                                    "pool reset partially failed for {} slot(s)",
                                    report.failed_reset_count()
                                ));
                            }
                            let count = report.succeeded_reset_slot_count();
                            event_log::emit_lazy("parallel_pool_reset_completed", || {
                                serde_json::json!({
                                    "workspace": &workspace_directory,
                                    "reset_scope": ParallelModePoolResetScope::PoolOnly.label(),
                                    "run_id": report.run_id.as_str(),
                                    "policy": report.policy,
                                    "slot_count": count,
                                })
                            });
                            let live_suffix = if report.has_live_blockers() {
                                format!(" / preserved {} live slot(s)", report.live_blocker_count())
                            } else {
                                String::new()
                            };
                            let entry_label = if initial_pool_reset_required {
                                "initial setup"
                            } else {
                                "off->on entry"
                            };
                            Ok(format!(
                                "reset {count} pool slot worktree(s) to prerelease after {entry_label}{live_suffix} / {}",
                                ParallelModePoolResetScope::PoolOnly.status_detail()
                            ))
                        })
                } else {
                    Ok(String::new())
                };
                let reset_status = match reset_result {
                    Ok(status) => status,
                    Err(error) => {
                        let supervisor_snapshot = parallel_mode_service.build_supervisor_snapshot(
                            &workspace_directory,
                            true,
                            Some(&readiness_snapshot),
                        );
                        let status_text = format!(
                            "parallel mode: blocked / readiness: {} / pool reset failed: {error}",
                            readiness_snapshot.readiness_label()
                        );
                        event_sink.send_control_plane_event(
                            ParallelModeControlPlaneBackgroundEvent::Entered {
                                workspace_directory,
                                epoch_id,
                                effect_id,
                                mode_was_enabled,
                                readiness_snapshot,
                                supervisor_snapshot: Box::new(supervisor_snapshot),
                                status_text,
                                initial_pool_reset_completed: false,
                                has_actionable_queue_head,
                                orchestrator_tick_signature: None,
                            },
                        );
                        return;
                    }
                };
                let supervisor_snapshot = parallel_mode_service.reconcile_supervisor_snapshot(
                    &workspace_directory,
                    true,
                    Some(&readiness_snapshot),
                );
                let mut status_text = format!(
                    "parallel mode: on / readiness: {} / control tower ready",
                    readiness_snapshot.readiness_label()
                );
                if !reset_status.trim().is_empty() {
                    status_text.push_str(" / ");
                    status_text.push_str(&reset_status);
                }
                (supervisor_snapshot, status_text)
            } else {
                let supervisor_snapshot = parallel_mode_service.build_supervisor_snapshot(
                    &workspace_directory,
                    false,
                    Some(&readiness_snapshot),
                );
                let cause = readiness_snapshot
                    .top_alert
                    .as_deref()
                    .unwrap_or("inspect the readiness panel before retrying");
                let status_text = format!(
                    "parallel mode: blocked / readiness: {} / {cause}",
                    readiness_snapshot.readiness_label()
                );
                (supervisor_snapshot, status_text)
            };

            let orchestrator_tick_signature =
                parallel_mode_distributor_tick_signature(&supervisor_snapshot);
            event_sink.send_control_plane_event(ParallelModeControlPlaneBackgroundEvent::Entered {
                workspace_directory,
                epoch_id,
                effect_id,
                mode_was_enabled,
                readiness_snapshot,
                supervisor_snapshot: Box::new(supervisor_snapshot),
                status_text,
                initial_pool_reset_completed,
                has_actionable_queue_head,
                orchestrator_tick_signature,
            });
        });
    }

    pub fn spawn_orchestrator_wake(
        &self,
        workspace_directory: String,
        trigger: crate::domain::parallel_mode::ParallelModeAutomationTrigger,
        epoch_id: u64,
        enqueue_trigger: Option<crate::domain::parallel_mode::ParallelModeAutomationTrigger>,
        effect_id: ParallelModeControlPlaneEffectId,
    ) {
        let parallel_mode_service = self.parallel_mode_service.clone();
        let parallel_agent_worker_port = self.worker_port.clone();
        let parallel_mode_turn_service = self.turn_service.clone();
        let planning = self.planning.clone();
        let event_sink = self.event_sink.clone();

        thread::spawn(move || {
            let (loop_event_tx, loop_event_rx) = mpsc::channel();
            let loop_event_sink = event_sink.clone();
            let loop_planning = planning.clone();
            thread::spawn(move || {
                while let Ok(event) = loop_event_rx.recv() {
                    loop_event_sink.send_control_plane_event(
                        background_event_from_parallel_loop_event(event, &loop_planning),
                    );
                }
            });
            let result = parallel_mode_service.run_dispatch_orchestrator_tick(
                ParallelModeDispatchOrchestratorTickRequest {
                    workspace_directory: workspace_directory.clone(),
                    trigger,
                    epoch_id,
                    enqueue_trigger,
                    planning,
                    worker_port: parallel_agent_worker_port,
                    turn_service: parallel_mode_turn_service,
                    event_sender: loop_event_tx,
                },
            );

            let orchestrator_tick_signature =
                parallel_mode_distributor_tick_signature(&result.supervisor_snapshot);
            event_sink.send_control_plane_event(
                ParallelModeControlPlaneBackgroundEvent::OrchestratorWakeCompleted {
                    workspace_directory: result.workspace_directory,
                    effect_id,
                    readiness_snapshot: result.readiness_snapshot,
                    supervisor_snapshot: Box::new(result.supervisor_snapshot),
                    outcome: result.outcome,
                    orchestrator_tick_signature,
                },
            );
        });
    }

    pub fn cancel_dispatch_commands(&self, workspace_directory: &str, reason: &str) {
        let _ = self
            .parallel_mode_service
            .cancel_dispatch_commands(workspace_directory, reason);
    }

    pub fn inspect_supervisor(
        &self,
        workspace_directory: &str,
        mode_enabled: bool,
        reconcile_pool: bool,
    ) -> (
        ParallelModeReadinessSnapshot,
        ParallelModeSupervisorSnapshot,
    ) {
        let planning_projection = self
            .planning
            .runtime
            .load_runtime_projection_or_invalid(workspace_directory);
        let readiness_snapshot = self
            .parallel_mode_service
            .inspect_readiness(workspace_directory, &planning_projection);
        let supervisor_snapshot = if reconcile_pool {
            self.parallel_mode_service.reconcile_supervisor_snapshot(
                workspace_directory,
                mode_enabled,
                Some(&readiness_snapshot),
            )
        } else {
            self.parallel_mode_service.build_supervisor_snapshot(
                workspace_directory,
                mode_enabled,
                Some(&readiness_snapshot),
            )
        };

        (readiness_snapshot, supervisor_snapshot)
    }

    pub fn continue_post_turn_queue_command(
        &self,
        workspace_directory: String,
        signal: Option<ParallelModePostTurnQueueSignal>,
        auto_follow_prompt_queued: bool,
    ) -> ParallelModeControlPlaneCommand {
        let has_actionable_queue_head = self
            .planning
            .runtime
            .load_runtime_projection_or_invalid(&workspace_directory)
            .has_actionable_queue_head();
        ParallelModeControlPlaneCommand::ContinuePostTurnQueue {
            workspace_directory,
            signal,
            auto_follow_prompt_queued,
            has_actionable_queue_head,
        }
    }

    pub fn pending_dispatch_wake(
        &self,
        workspace_directory: &str,
        epoch_id: u64,
    ) -> Result<Option<ParallelModeControlPlaneWake>, String> {
        self.parallel_mode_service
            .pending_dispatch_wake(workspace_directory, epoch_id)
    }

    pub fn enqueue_slot_capacity_dispatch(
        &self,
        workspace_directory: &str,
        epoch_id: u64,
    ) -> Result<usize, String> {
        let planning_projection = self
            .planning
            .runtime
            .load_runtime_projection_or_invalid(workspace_directory);
        self.parallel_mode_service
            .enqueue_dispatch_commands_for_event(
                workspace_directory,
                ParallelModeRuntimeEvent::SlotCapacityAvailable,
                &planning_projection,
                Some(epoch_id),
            )
    }

    pub fn enqueue_dispatch_for_trigger(
        &self,
        workspace_directory: &str,
        trigger: crate::domain::parallel_mode::ParallelModeAutomationTrigger,
        epoch_id: u64,
    ) -> Result<usize, String> {
        let planning_projection = self
            .planning
            .runtime
            .load_runtime_projection_or_invalid(workspace_directory);
        self.parallel_mode_service
            .enqueue_dispatch_commands_for_trigger(
                workspace_directory,
                trigger,
                &planning_projection,
                Some(epoch_id),
            )
    }
}

fn background_event_from_parallel_loop_event(
    event: ParallelModeOrchestratorLoopEvent,
    planning: &PlanningServices,
) -> ParallelModeControlPlaneBackgroundEvent {
    match event {
        ParallelModeOrchestratorLoopEvent::ConversationRuntimeNotice(notice) => {
            ParallelModeControlPlaneBackgroundEvent::ConversationRuntimeNotice(notice)
        }
        ParallelModeOrchestratorLoopEvent::WorkerEvent(event) => {
            let has_actionable_queue_head = planning
                .runtime
                .load_runtime_projection_or_invalid(&event.workspace_directory)
                .has_actionable_queue_head();
            ParallelModeControlPlaneBackgroundEvent::WorkerEvent {
                event,
                has_actionable_queue_head,
            }
        }
    }
}

pub(crate) fn parallel_mode_distributor_tick_signature(
    snapshot: &ParallelModeSupervisorSnapshot,
) -> Option<String> {
    let head = snapshot.distributor.queue_items.first()?;
    Some(format!(
        "{}|{}|{}|{}|{}|{}",
        snapshot.workspace_path,
        head.source_agent,
        head.branch_name,
        head.commit_short_sha,
        head.queue_state.label(),
        snapshot
            .distributor
            .orchestrator_status
            .integration_worktree_readiness
    ))
}
