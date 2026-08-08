use std::sync::Arc;
use std::sync::mpsc;
use std::thread;
use std::thread::JoinHandle;

use serde_json::Value;

use crate::application::port::outbound::parallel_agent_worker_port::ParallelAgentWorkerPort;
use crate::application::service::parallel_mode::turn::ParallelModeTurnService;
use crate::application::service::parallel_mode::{
    ParallelModeAutomationGuard, ParallelModeDispatchOrchestratorTickRequest,
    ParallelModeOrchestratorLoopEvent, ParallelModeOrchestratorTrigger, ParallelModeService,
    distributor_integration_branch_for_repo,
};
use crate::application::service::planning::PlanningServices;
use crate::diagnostics::event_log;
use crate::domain::parallel_mode::{
    ParallelModeControlPlaneWorkerEvent, ParallelModeDispatchOutcome,
    ParallelModeOrchestratorStateMachine, ParallelModePoolResetPolicy, ParallelModePoolResetReport,
    ParallelModePoolResetRunId, ParallelModePoolResetScope, ParallelModeReadinessSnapshot,
    ParallelModeRuntimeEvent, ParallelModeSupervisorSnapshot,
};
use crate::panic_observation::catch_redacted_worker_unwind;

use super::{
    ParallelModeControlPlaneEffectId, ParallelModeControlPlaneWake, ParallelModeDispatchMutation,
    ParallelModeDispatchMutationCorrelation, ParallelModePendingDispatchPollCorrelation,
    ParallelModeSupervisorInspectionCorrelation,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParallelModeControlPlaneLoadingStage {
    ReconcilingPool,
}

#[derive(Debug, Clone)]
pub struct ParallelModeSupervisorInspectionSnapshot {
    pub readiness_snapshot: ParallelModeReadinessSnapshot,
    pub supervisor_snapshot: Box<ParallelModeSupervisorSnapshot>,
}

#[derive(Debug, Clone)]
pub enum ParallelModeControlPlaneBackgroundEvent {
    EnterProgress {
        workspace_directory: String,
        epoch_id: u64,
        effect_id: ParallelModeControlPlaneEffectId,
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
    SupervisorInspectionCompleted {
        correlation: ParallelModeSupervisorInspectionCorrelation,
        result: Result<ParallelModeSupervisorInspectionSnapshot, String>,
    },
    PendingDispatchWakePolled {
        correlation: ParallelModePendingDispatchPollCorrelation,
        result: Result<Option<ParallelModeControlPlaneWake>, String>,
    },
    DispatchMutationCompleted {
        correlation: ParallelModeDispatchMutationCorrelation,
        result: Result<usize, String>,
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
    ConversationRuntimeNotice {
        workspace_directory: String,
        epoch_id: u64,
        effect_id: ParallelModeControlPlaneEffectId,
        notice: String,
    },
    OrchestratorTickCompleted {
        workspace_directory: String,
        epoch_id: u64,
        effect_id: ParallelModeControlPlaneEffectId,
        blocked: bool,
        notices: Vec<String>,
    },
    EffectFailed {
        workspace_directory: String,
        epoch_id: u64,
        effect_id: ParallelModeControlPlaneEffectId,
        error: String,
    },
}

pub trait ParallelModeControlPlaneEventSink: Clone + Send + 'static {
    fn send_control_plane_event(&self, event: ParallelModeControlPlaneBackgroundEvent);
}

fn spawn_parallel_effect_completion_worker<S, Work>(
    event_sink: S,
    panic_completion: ParallelModeControlPlaneBackgroundEvent,
    work: Work,
) -> JoinHandle<()>
where
    S: ParallelModeControlPlaneEventSink,
    Work: FnOnce() -> ParallelModeControlPlaneBackgroundEvent + Send + 'static,
{
    thread::spawn(move || {
        let completion = catch_redacted_worker_unwind(work).unwrap_or(panic_completion);
        let _ = catch_redacted_worker_unwind(|| {
            event_sink.send_control_plane_event(completion);
        });
    })
}

fn effect_failed(
    workspace_directory: impl Into<String>,
    epoch_id: u64,
    effect_id: ParallelModeControlPlaneEffectId,
    error: impl Into<String>,
) -> ParallelModeControlPlaneBackgroundEvent {
    ParallelModeControlPlaneBackgroundEvent::EffectFailed {
        workspace_directory: workspace_directory.into(),
        epoch_id,
        effect_id,
        error: error.into(),
    }
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
    automation_guard: ParallelModeAutomationGuard,
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
        let automation_guard = ParallelModeAutomationGuard::default();
        Self::new_with_automation_guard(
            parallel_mode_service,
            planning,
            worker_port,
            turn_service,
            automation_guard,
            event_sink,
        )
    }

    pub(crate) fn new_with_automation_guard(
        parallel_mode_service: ParallelModeService,
        planning: PlanningServices,
        worker_port: Arc<dyn ParallelAgentWorkerPort>,
        turn_service: ParallelModeTurnService,
        automation_guard: ParallelModeAutomationGuard,
        event_sink: S,
    ) -> Self {
        Self {
            parallel_mode_service,
            planning,
            worker_port,
            turn_service: turn_service.with_automation_guard(automation_guard.clone()),
            event_sink,
            automation_guard,
        }
    }

    pub(crate) fn activate_epoch(&self, workspace_directory: &str, epoch_id: u64) {
        self.automation_guard
            .activate(workspace_directory.to_string(), epoch_id);
    }

    pub(crate) fn cancel_epoch(&self, workspace_directory: &str) {
        self.automation_guard.cancel(workspace_directory);
    }

    #[cfg(test)]
    pub(crate) fn automation_epoch_is_active(
        &self,
        workspace_directory: &str,
        epoch_id: u64,
    ) -> bool {
        self.automation_guard
            .is_active(workspace_directory, epoch_id)
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
        let automation_guard = self.automation_guard.clone();
        let panic_completion = effect_failed(
            workspace_directory.clone(),
            epoch_id,
            effect_id,
            "parallel supervisor refresh failed unexpectedly",
        );

        spawn_parallel_effect_completion_worker(event_sink, panic_completion, move || {
            if !automation_guard.is_active(&workspace_directory, epoch_id) {
                return effect_failed(
                    workspace_directory,
                    epoch_id,
                    effect_id,
                    "parallel supervisor refresh belongs to an inactive epoch",
                );
            }
            event_log::emit_lazy("parallel_supervisor_refresh_started", || {
                supervisor_refresh_started_payload(&workspace_directory, mode_enabled)
            });
            let supervisor_snapshot = parallel_mode_service.build_supervisor_snapshot(
                &workspace_directory,
                mode_enabled,
                Some(&readiness_snapshot),
            );
            event_log::emit_lazy("parallel_supervisor_refresh_completed", || {
                supervisor_refresh_completed_payload(
                    &workspace_directory,
                    mode_enabled,
                    &supervisor_snapshot,
                )
            });
            let orchestrator_tick_signature = parallel_mode_or_recovery_tick_signature(
                &parallel_mode_service,
                &workspace_directory,
                &supervisor_snapshot,
            );
            ParallelModeControlPlaneBackgroundEvent::SupervisorSnapshotRefreshed {
                workspace_directory,
                epoch_id,
                effect_id,
                orchestrator_tick_signature,
                supervisor_snapshot: Box::new(supervisor_snapshot),
            }
        });
    }

    pub fn spawn_supervisor_inspection(
        &self,
        correlation: ParallelModeSupervisorInspectionCorrelation,
        mode_enabled: bool,
        reconcile_pool: bool,
    ) {
        let parallel_mode_service = self.parallel_mode_service.clone();
        let planning = self.planning.clone();
        let event_sink = self.event_sink.clone();
        let automation_guard = self.automation_guard.clone();
        let panic_completion =
            ParallelModeControlPlaneBackgroundEvent::SupervisorInspectionCompleted {
                correlation: correlation.clone(),
                result: Err("parallel supervisor inspection failed unexpectedly".to_string()),
            };

        spawn_parallel_effect_completion_worker(event_sink, panic_completion, move || {
            let workspace_directory = correlation.workspace_directory.clone();
            let automation_permit = correlation
                .epoch_id
                .map(|epoch_id| automation_guard.permit(&workspace_directory, epoch_id));
            let result = (|| -> Result<ParallelModeSupervisorInspectionSnapshot, String> {
                if automation_permit
                    .as_ref()
                    .is_some_and(|permit| !permit.is_active())
                {
                    return Err(
                        "parallel supervisor inspection belongs to an inactive epoch".to_string(),
                    );
                }
                let planning_projection = planning
                    .runtime
                    .load_runtime_projection_or_invalid(&workspace_directory);
                let readiness_snapshot = parallel_mode_service
                    .inspect_readiness(&workspace_directory, &planning_projection);
                let supervisor_snapshot = if reconcile_pool {
                    let permit = automation_permit.as_ref().ok_or_else(|| {
                        "parallel supervisor reconciliation has no automation epoch".to_string()
                    })?;
                    parallel_mode_service.reconcile_supervisor_snapshot_guarded(
                        &workspace_directory,
                        mode_enabled,
                        Some(&readiness_snapshot),
                        permit,
                    )?
                } else {
                    parallel_mode_service.build_supervisor_snapshot(
                        &workspace_directory,
                        mode_enabled,
                        Some(&readiness_snapshot),
                    )
                };
                Ok(ParallelModeSupervisorInspectionSnapshot {
                    readiness_snapshot,
                    supervisor_snapshot: Box::new(supervisor_snapshot),
                })
            })();
            ParallelModeControlPlaneBackgroundEvent::SupervisorInspectionCompleted {
                correlation,
                result,
            }
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
        let planning = self.planning.clone();
        let event_sink = self.event_sink.clone();
        let automation_guard = self.automation_guard.clone();
        let panic_completion = effect_failed(
            workspace_directory.clone(),
            epoch_id,
            effect_id,
            "parallel orchestrator tick failed unexpectedly",
        );

        spawn_parallel_effect_completion_worker(event_sink, panic_completion, move || {
            if !automation_guard.is_active(&workspace_directory, epoch_id) {
                return effect_failed(
                    workspace_directory,
                    epoch_id,
                    effect_id,
                    "parallel orchestrator tick belongs to an inactive epoch",
                );
            }
            event_log::emit_lazy("parallel_orchestrator_retry_started", || {
                orchestrator_retry_started_payload(&workspace_directory, &signature)
            });
            let validation_error = parallel_mode_service
                .poll_pr_validations_for_runtime_tick(&workspace_directory, &planning.queue)
                .err();
            let permit = automation_guard.permit(&workspace_directory, epoch_id);
            let (blocked, mut notices) = match parallel_mode_service.run_orchestrator_tick_guarded(
                &workspace_directory,
                ParallelModeOrchestratorTrigger::ManualDispatch,
                &permit,
            ) {
                Ok(result) => (result.blocked, result.notices),
                Err(error) => (
                    true,
                    vec![format!("orchestrator retry tick failed: {error}")],
                ),
            };
            if let Some(error) = validation_error {
                notices.push(format!("PR validation poll failed: {error}"));
            }
            event_log::emit_lazy("parallel_orchestrator_retry_completed", || {
                orchestrator_retry_completed_payload(
                    &workspace_directory,
                    &signature,
                    blocked,
                    notices.len(),
                )
            });
            ParallelModeControlPlaneBackgroundEvent::OrchestratorTickCompleted {
                workspace_directory,
                epoch_id,
                effect_id,
                blocked,
                notices,
            }
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
        let automation_guard = self.automation_guard.clone();
        let panic_completion = effect_failed(
            workspace_directory.clone(),
            epoch_id,
            effect_id,
            "parallel mode entry failed unexpectedly",
        );

        spawn_parallel_effect_completion_worker(event_sink.clone(), panic_completion, move || {
            if !automation_guard.is_active(&workspace_directory, epoch_id) {
                return effect_failed(
                    workspace_directory,
                    epoch_id,
                    effect_id,
                    "parallel mode entry belongs to an inactive epoch",
                );
            }
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
                parallel_action_planned_payload(
                    &workspace_directory,
                    entry_plan.state.label(),
                    entry_plan.reset_scope.map(|scope| scope.label()),
                    readiness_snapshot.readiness_label(),
                    initial_pool_reset_required,
                )
            });

            let initial_pool_reset_completed = initial_pool_reset_required
                && entry_plan.reset_scope == Some(ParallelModePoolResetScope::PoolOnly);
            let (supervisor_snapshot, status_text) = if readiness_snapshot.allows_parallel_mode() {
                if !automation_guard.is_active(&workspace_directory, epoch_id) {
                    return effect_failed(
                        workspace_directory,
                        epoch_id,
                        effect_id,
                        "parallel mode entry belongs to an inactive epoch",
                    );
                }
                event_sink.send_control_plane_event(
                    ParallelModeControlPlaneBackgroundEvent::EnterProgress {
                        workspace_directory: workspace_directory.clone(),
                        epoch_id,
                        effect_id,
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
                    if !automation_guard.is_active(&workspace_directory, epoch_id) {
                        return effect_failed(
                            workspace_directory,
                            epoch_id,
                            effect_id,
                            "parallel mode entry belongs to an inactive epoch",
                        );
                    }
                    event_log::emit_lazy("parallel_pool_reset_started", || {
                        parallel_pool_reset_started_payload(
                            &workspace_directory,
                            initial_pool_reset_required,
                        )
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
                                    parallel_pool_reset_preserved_live_payload(
                                        &workspace_directory,
                                        &report,
                                    )
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
                                parallel_pool_reset_completed_payload(
                                    &workspace_directory,
                                    &report,
                                    count,
                                )
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
                                "reset {count} pool slot worktree(s) to {} after {entry_label}{live_suffix} / {}",
                                distributor_integration_branch_for_repo(
                                    parallel_mode_service.parallel_runtime.as_ref(),
                                    &workspace_directory,
                                ),
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
                        return ParallelModeControlPlaneBackgroundEvent::Entered {
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
                        };
                    }
                };
                if !automation_guard.is_active(&workspace_directory, epoch_id) {
                    return effect_failed(
                        workspace_directory,
                        epoch_id,
                        effect_id,
                        "parallel mode entry belongs to an inactive epoch",
                    );
                }
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

            let orchestrator_tick_signature = parallel_mode_or_recovery_tick_signature(
                &parallel_mode_service,
                &workspace_directory,
                &supervisor_snapshot,
            );
            if !automation_guard.is_active(&workspace_directory, epoch_id) {
                return effect_failed(
                    workspace_directory,
                    epoch_id,
                    effect_id,
                    "parallel mode entry belongs to an inactive epoch",
                );
            }
            ParallelModeControlPlaneBackgroundEvent::Entered {
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
            }
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
        let automation_guard = self.automation_guard.clone();
        let panic_completion = effect_failed(
            workspace_directory.clone(),
            epoch_id,
            effect_id,
            "parallel orchestrator wake failed unexpectedly",
        );

        spawn_parallel_effect_completion_worker(event_sink.clone(), panic_completion, move || {
            if !automation_guard.is_active(&workspace_directory, epoch_id) {
                return effect_failed(
                    workspace_directory,
                    epoch_id,
                    effect_id,
                    "parallel orchestrator wake belongs to an inactive epoch",
                );
            }
            let (loop_event_tx, loop_event_rx) = mpsc::channel();
            let loop_event_sink = event_sink.clone();
            let loop_planning = planning.clone();
            let loop_workspace_directory = workspace_directory.clone();
            thread::spawn(move || {
                let _ = catch_redacted_worker_unwind(|| {
                    while let Ok(event) = loop_event_rx.recv() {
                        let fallback_event = match &event {
                            ParallelModeOrchestratorLoopEvent::ConversationRuntimeNotice(_) => {
                                ParallelModeControlPlaneBackgroundEvent::ConversationRuntimeNotice {
                                    workspace_directory: loop_workspace_directory.clone(),
                                    epoch_id,
                                    effect_id,
                                    notice: "parallel runtime notice could not be projected"
                                        .to_string(),
                                }
                            }
                            ParallelModeOrchestratorLoopEvent::WorkerEvent(event) => {
                                ParallelModeControlPlaneBackgroundEvent::WorkerEvent {
                                    event: event.clone(),
                                    has_actionable_queue_head: false,
                                }
                            }
                        };
                        let background_event = catch_redacted_worker_unwind(|| {
                            background_event_from_parallel_loop_event(
                                event,
                                &loop_planning,
                                &loop_workspace_directory,
                                epoch_id,
                                effect_id,
                            )
                        })
                        .unwrap_or(fallback_event);
                        loop_event_sink.send_control_plane_event(background_event);
                    }
                });
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

            let orchestrator_tick_signature = parallel_mode_or_recovery_tick_signature(
                &parallel_mode_service,
                &workspace_directory,
                &result.supervisor_snapshot,
            );
            ParallelModeControlPlaneBackgroundEvent::OrchestratorWakeCompleted {
                workspace_directory: result.workspace_directory,
                effect_id,
                readiness_snapshot: result.readiness_snapshot,
                supervisor_snapshot: Box::new(result.supervisor_snapshot),
                outcome: result.outcome,
                orchestrator_tick_signature,
            }
        });
    }

    pub fn spawn_pending_dispatch_wake_poll(
        &self,
        correlation: ParallelModePendingDispatchPollCorrelation,
    ) {
        let parallel_mode_service = self.parallel_mode_service.clone();
        let planning = self.planning.clone();
        let event_sink = self.event_sink.clone();
        let automation_guard = self.automation_guard.clone();
        let panic_completion = ParallelModeControlPlaneBackgroundEvent::PendingDispatchWakePolled {
            correlation: correlation.clone(),
            result: Err("pending dispatch poll failed unexpectedly".to_string()),
        };

        spawn_parallel_effect_completion_worker(event_sink, panic_completion, move || {
            let result = if !automation_guard
                .is_active(&correlation.workspace_directory, correlation.epoch_id)
            {
                Err("pending dispatch poll belongs to an inactive automation epoch".to_string())
            } else {
                if let Err(error) = parallel_mode_service.poll_pr_validations_for_runtime_tick(
                    &correlation.workspace_directory,
                    &planning.queue,
                ) {
                    event_log::emit_lazy("parallel_pr_validation_poll_failed", || {
                        serde_json::json!({
                            "workspace": &correlation.workspace_directory,
                            "epoch_id": correlation.epoch_id,
                            "error": error,
                        })
                    });
                }
                parallel_mode_service
                    .pending_dispatch_wake(&correlation.workspace_directory, correlation.epoch_id)
            };
            ParallelModeControlPlaneBackgroundEvent::PendingDispatchWakePolled {
                correlation,
                result,
            }
        });
    }

    pub fn spawn_dispatch_command_mutation(
        &self,
        correlation: ParallelModeDispatchMutationCorrelation,
        mutation: ParallelModeDispatchMutation,
    ) {
        let parallel_mode_service = self.parallel_mode_service.clone();
        let planning = self.planning.clone();
        let event_sink = self.event_sink.clone();
        let panic_completion = ParallelModeControlPlaneBackgroundEvent::DispatchMutationCompleted {
            correlation: correlation.clone(),
            result: Err("parallel dispatch mutation failed unexpectedly".to_string()),
        };

        spawn_parallel_effect_completion_worker(event_sink, panic_completion, move || {
            let result = match &mutation {
                ParallelModeDispatchMutation::EnqueueSlotCapacity => {
                    let planning_projection = planning
                        .runtime
                        .load_runtime_projection_or_invalid(&correlation.workspace_directory);
                    parallel_mode_service.enqueue_dispatch_commands_for_event(
                        &correlation.workspace_directory,
                        ParallelModeRuntimeEvent::SlotCapacityAvailable,
                        &planning_projection,
                        Some(correlation.epoch_id),
                    )
                }
                ParallelModeDispatchMutation::EnqueueForTrigger { trigger, .. } => {
                    let planning_projection = planning
                        .runtime
                        .load_runtime_projection_or_invalid(&correlation.workspace_directory);
                    parallel_mode_service.enqueue_dispatch_commands_for_trigger(
                        &correlation.workspace_directory,
                        *trigger,
                        &planning_projection,
                        Some(correlation.epoch_id),
                    )
                }
                ParallelModeDispatchMutation::Cancel { reason } => parallel_mode_service
                    .cancel_dispatch_commands(&correlation.workspace_directory, reason),
                ParallelModeDispatchMutation::RetryCancel { original_cleanup } => {
                    parallel_mode_service.cancel_dispatch_commands(
                        &original_cleanup.workspace_directory,
                        &format!(
                            "retry unsettled cleanup operation {}",
                            original_cleanup.operation_id
                        ),
                    )
                }
            };
            ParallelModeControlPlaneBackgroundEvent::DispatchMutationCompleted {
                correlation,
                result,
            }
        });
    }
}

fn supervisor_refresh_started_payload(workspace_directory: &str, mode_enabled: bool) -> Value {
    serde_json::json!({
        "workspace_directory": workspace_directory,
        "mode_enabled": mode_enabled,
    })
}

fn supervisor_refresh_completed_payload(
    workspace_directory: &str,
    mode_enabled: bool,
    supervisor_snapshot: &ParallelModeSupervisorSnapshot,
) -> Value {
    serde_json::json!({
        "workspace_directory": workspace_directory,
        "mode_enabled": mode_enabled,
        "pool_status": &supervisor_snapshot.pool.reconcile_status,
        "roster_active_count": supervisor_snapshot.roster.active_count(),
    })
}

fn orchestrator_retry_started_payload(workspace_directory: &str, signature: &str) -> Value {
    serde_json::json!({
        "workspace": workspace_directory,
        "signature": signature,
        "trigger": "supervisor_active_distributor_queue",
    })
}

fn orchestrator_retry_completed_payload(
    workspace_directory: &str,
    signature: &str,
    blocked: bool,
    notices_count: usize,
) -> Value {
    serde_json::json!({
        "workspace": workspace_directory,
        "signature": signature,
        "blocked": blocked,
        "notices_count": notices_count,
    })
}

fn parallel_action_planned_payload(
    workspace_directory: &str,
    state: &str,
    reset_scope: Option<&str>,
    readiness: &str,
    initial_pool_reset_required: bool,
) -> Value {
    serde_json::json!({
        "workspace": workspace_directory,
        "state": state,
        "reset_scope": reset_scope,
        "readiness": readiness,
        "initial_setup_reset": initial_pool_reset_required,
    })
}

fn parallel_pool_reset_started_payload(
    workspace_directory: &str,
    initial_pool_reset_required: bool,
) -> Value {
    serde_json::json!({
        "workspace": workspace_directory,
        "reset_scope": ParallelModePoolResetScope::PoolOnly.label(),
        "initial_setup_reset": initial_pool_reset_required,
    })
}

fn parallel_pool_reset_preserved_live_payload(
    workspace_directory: &str,
    report: &ParallelModePoolResetReport,
) -> Value {
    serde_json::json!({
        "workspace": workspace_directory,
        "reset_scope": ParallelModePoolResetScope::PoolOnly.label(),
        "run_id": report.run_id.as_str(),
        "policy": report.policy,
        "live_blockers": report.live_blocker_count(),
    })
}

fn parallel_pool_reset_completed_payload(
    workspace_directory: &str,
    report: &ParallelModePoolResetReport,
    slot_count: usize,
) -> Value {
    serde_json::json!({
        "workspace": workspace_directory,
        "reset_scope": ParallelModePoolResetScope::PoolOnly.label(),
        "run_id": report.run_id.as_str(),
        "policy": report.policy,
        "slot_count": slot_count,
    })
}

fn background_event_from_parallel_loop_event(
    event: ParallelModeOrchestratorLoopEvent,
    planning: &PlanningServices,
    workspace_directory: &str,
    epoch_id: u64,
    effect_id: ParallelModeControlPlaneEffectId,
) -> ParallelModeControlPlaneBackgroundEvent {
    match event {
        ParallelModeOrchestratorLoopEvent::ConversationRuntimeNotice(notice) => {
            ParallelModeControlPlaneBackgroundEvent::ConversationRuntimeNotice {
                workspace_directory: workspace_directory.to_string(),
                epoch_id,
                effect_id,
                notice,
            }
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

fn parallel_mode_or_recovery_tick_signature(
    parallel_mode_service: &ParallelModeService,
    workspace_directory: &str,
    snapshot: &ParallelModeSupervisorSnapshot,
) -> Option<String> {
    let pending_commit_ready = parallel_mode_service
        .pending_commit_ready_recovery_signature(workspace_directory)
        .ok()
        .flatten();
    combine_parallel_mode_tick_signature(
        workspace_directory,
        pending_commit_ready.as_deref(),
        snapshot,
    )
}

fn combine_parallel_mode_tick_signature(
    workspace_directory: &str,
    pending_commit_ready: Option<&str>,
    snapshot: &ParallelModeSupervisorSnapshot,
) -> Option<String> {
    let distributor_head = parallel_mode_distributor_tick_signature(snapshot);
    let pending_commit_ready =
        pending_commit_ready.map(|signature| format!("{workspace_directory}|{signature}"));
    match (distributor_head, pending_commit_ready) {
        (Some(head), Some(recovery)) => Some(format!("{head}|recovery:{recovery}")),
        (Some(head), None) => Some(head),
        (None, Some(recovery)) => Some(recovery),
        (None, None) => None,
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use std::sync::mpsc;
    use std::time::Duration;

    use crate::domain::parallel_mode::{
        ParallelModeAgentRosterSnapshot, ParallelModeDistributorQueueItem,
        ParallelModeDistributorSnapshot, ParallelModePoolBoardSnapshot,
        ParallelModePoolResetSlotAction, ParallelModePoolResetSlotOutcome,
        ParallelModePoolResetSlotReport, ParallelModeQueueItemState,
        ParallelModeSupervisorDetailSnapshot, ParallelModeSupervisorState,
    };

    use super::*;

    #[derive(Clone)]
    struct CapturingEventSink {
        tx: mpsc::Sender<ParallelModeControlPlaneBackgroundEvent>,
    }

    impl ParallelModeControlPlaneEventSink for CapturingEventSink {
        fn send_control_plane_event(&self, event: ParallelModeControlPlaneBackgroundEvent) {
            let _ = self.tx.send(event);
        }
    }

    fn supervisor_snapshot() -> ParallelModeSupervisorSnapshot {
        ParallelModeSupervisorSnapshot::new(
            ParallelModeSupervisorState::Supervise,
            "/repo",
            ParallelModePoolBoardSnapshot::new(0, "pool", "ready", Vec::new()),
            ParallelModeAgentRosterSnapshot::new(Vec::new(), "no agents"),
            ParallelModeSupervisorDetailSnapshot::new(None, "no detail"),
            ParallelModeDistributorSnapshot::new(Vec::new(), Vec::new(), "idle", "none"),
            None,
        )
    }

    fn assert_exactly_one_worker_completion(
        work: impl FnOnce() -> ParallelModeControlPlaneBackgroundEvent + Send + 'static,
        panic_completion: ParallelModeControlPlaneBackgroundEvent,
    ) -> ParallelModeControlPlaneBackgroundEvent {
        let (tx, rx) = mpsc::channel();
        spawn_parallel_effect_completion_worker(CapturingEventSink { tx }, panic_completion, work)
            .join()
            .expect("completion worker should settle its own panic");
        let completion = rx
            .recv_timeout(Duration::from_secs(1))
            .expect("completion worker should publish one terminal event");
        assert!(
            matches!(rx.try_recv(), Err(mpsc::TryRecvError::Disconnected)),
            "completion worker must publish exactly one terminal event"
        );
        completion
    }

    #[test]
    fn completion_worker_is_total_for_success_failure_and_panic() {
        let effect_id = ParallelModeControlPlaneEffectId {
            sequence: 11,
            kind: super::super::ParallelModeControlPlaneEffectKind::RunOrchestratorTick,
        };
        let panic_fallback =
            || effect_failed("/repo", 7, effect_id, "parallel effect failed unexpectedly");

        let success = assert_exactly_one_worker_completion(
            move || ParallelModeControlPlaneBackgroundEvent::OrchestratorTickCompleted {
                workspace_directory: "/repo".to_string(),
                epoch_id: 7,
                effect_id,
                blocked: false,
                notices: Vec::new(),
            },
            panic_fallback(),
        );
        assert!(matches!(
            success,
            ParallelModeControlPlaneBackgroundEvent::OrchestratorTickCompleted {
                effect_id: completed,
                ..
            } if completed == effect_id
        ));

        let failure = assert_exactly_one_worker_completion(
            move || effect_failed("/repo", 7, effect_id, "service returned an error"),
            panic_fallback(),
        );
        assert!(matches!(
            failure,
            ParallelModeControlPlaneBackgroundEvent::EffectFailed {
                effect_id: completed,
                error,
                ..
            } if completed == effect_id && error == "service returned an error"
        ));

        let panic = assert_exactly_one_worker_completion(
            || panic!("SECRET-PANIC-PAYLOAD"),
            panic_fallback(),
        );
        assert!(matches!(
            panic,
            ParallelModeControlPlaneBackgroundEvent::EffectFailed {
                effect_id: completed,
                error,
                ..
            } if completed == effect_id
                && error == "parallel effect failed unexpectedly"
                && !error.contains("SECRET-PANIC-PAYLOAD")
        ));
    }

    #[test]
    fn commit_ready_only_authority_state_produces_a_control_plane_wake_signature() {
        assert_eq!(
            combine_parallel_mode_tick_signature(
                "/repo",
                Some("commit-ready|slot-1@generation-1|2026-07-12T00:00:00Z"),
                &supervisor_snapshot(),
            ),
            Some("/repo|commit-ready|slot-1@generation-1|2026-07-12T00:00:00Z".to_string())
        );
    }

    #[test]
    fn active_head_and_recovery_state_share_one_stable_wake_signature() {
        let mut snapshot = supervisor_snapshot();
        snapshot.distributor = ParallelModeDistributorSnapshot::new(
            vec![ParallelModeDistributorQueueItem::new(
                "agent-a",
                "Queued A",
                ParallelModeQueueItemState::Queued,
                "akra-agent/slot-1/a",
                "abc1234",
                "queued",
            )],
            Vec::new(),
            "queued",
            "active",
        );
        let dirty = combine_parallel_mode_tick_signature(
            "/repo",
            Some("commit-ready|slot-2@generation-2|updated|source:untracked files"),
            &snapshot,
        )
        .expect("active head and recovery should be wakeable");
        let unchanged = combine_parallel_mode_tick_signature(
            "/repo",
            Some("commit-ready|slot-2@generation-2|updated|source:untracked files"),
            &snapshot,
        )
        .expect("unchanged state should retain its signature");
        let clean = combine_parallel_mode_tick_signature(
            "/repo",
            Some("commit-ready|slot-2@generation-2|updated|source:clean"),
            &snapshot,
        )
        .expect("externally repaired recovery should remain wakeable");

        assert_eq!(dirty, unchanged);
        assert_ne!(dirty, clean);
        assert!(dirty.starts_with("/repo|agent-a|akra-agent/slot-1/a|abc1234|queued|"));
        assert!(dirty.contains("|recovery:/repo|commit-ready|slot-2@generation-2|"));
    }

    #[test]
    fn trace_payload_helpers_render_supervisor_and_retry_shapes() {
        assert_eq!(
            supervisor_refresh_started_payload("/repo", true),
            json!({
                "workspace_directory": "/repo",
                "mode_enabled": true,
            })
        );

        assert_eq!(
            supervisor_refresh_completed_payload("/repo", true, &supervisor_snapshot()),
            json!({
                "workspace_directory": "/repo",
                "mode_enabled": true,
                "pool_status": "ready",
                "roster_active_count": 0,
            })
        );

        assert_eq!(
            orchestrator_retry_started_payload("/repo", "tick-1"),
            json!({
                "workspace": "/repo",
                "signature": "tick-1",
                "trigger": "supervisor_active_distributor_queue",
            })
        );

        assert_eq!(
            orchestrator_retry_completed_payload("/repo", "tick-1", true, 2),
            json!({
                "workspace": "/repo",
                "signature": "tick-1",
                "blocked": true,
                "notices_count": 2,
            })
        );
    }

    #[test]
    fn trace_payload_helpers_render_entry_and_pool_reset_shapes() {
        assert_eq!(
            parallel_action_planned_payload(
                "/repo",
                "pool_resetting",
                Some(ParallelModePoolResetScope::PoolOnly.label()),
                "ready",
                true,
            ),
            json!({
                "workspace": "/repo",
                "state": "pool_resetting",
                "reset_scope": "pool_only",
                "readiness": "ready",
                "initial_setup_reset": true,
            })
        );

        let mut report = ParallelModePoolResetReport::new(
            ParallelModePoolResetRunId::new("run-1"),
            ParallelModePoolResetPolicy::ProtectLive,
        );
        report
            .slot_reports
            .push(ParallelModePoolResetSlotReport::new(
                "slot-1",
                ParallelModePoolResetSlotAction::PreserveLive,
                ParallelModePoolResetSlotOutcome::Blocked,
                "busy",
            ));
        report
            .slot_reports
            .push(ParallelModePoolResetSlotReport::new(
                "slot-2",
                ParallelModePoolResetSlotAction::Reset,
                ParallelModePoolResetSlotOutcome::Succeeded,
                "clean",
            ));

        assert_eq!(
            parallel_pool_reset_started_payload("/repo", true),
            json!({
                "workspace": "/repo",
                "reset_scope": "pool_only",
                "initial_setup_reset": true,
            })
        );
        assert_eq!(
            parallel_pool_reset_preserved_live_payload("/repo", &report),
            json!({
                "workspace": "/repo",
                "reset_scope": "pool_only",
                "run_id": "run-1",
                "policy": "protect_live",
                "live_blockers": 1,
            })
        );
        assert_eq!(
            parallel_pool_reset_completed_payload(
                "/repo",
                &report,
                report.succeeded_reset_slot_count(),
            ),
            json!({
                "workspace": "/repo",
                "reset_scope": "pool_only",
                "run_id": "run-1",
                "policy": "protect_live",
                "slot_count": 1,
            })
        );
    }
}
