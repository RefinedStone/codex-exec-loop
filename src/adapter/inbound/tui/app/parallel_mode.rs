use crossterm::event::{self, KeyCode, KeyModifiers};
use std::sync::mpsc;
use std::thread;

use crate::adapter::inbound::tui::shell_chrome::{ShellChromeEvent, ShellOverlay};
use crate::application::service::parallel_mode::{
    ParallelModeDispatchOrchestratorTickRequest, ParallelModeOrchestratorLoopEvent,
    ParallelModeOrchestratorTrigger, ParallelModeService,
    control_plane::{
        ParallelModeControlPlaneCommand, ParallelModeControlPlaneEffect,
        ParallelModeControlPlaneEffectId, ParallelModeControlPlaneEvent,
        ParallelModeControlPlaneRuntimeOutcome, ParallelModeControlPlaneWake,
        ParallelModeControlPlaneWorkerEvent,
    },
};
use crate::diagnostics::event_log;
use crate::domain::parallel_mode::{
    ParallelModeAgentRosterSnapshot, ParallelModeAutomationTrigger, ParallelModeDispatchOutcome,
    ParallelModeDistributorSnapshot, ParallelModeOrchestratorStateMachine,
    ParallelModePoolBoardSnapshot, ParallelModePoolResetScope, ParallelModePostTurnQueueSignal,
    ParallelModeReadinessSnapshot, ParallelModeRuntimeEvent, ParallelModeSupervisorDetailSnapshot,
    ParallelModeSupervisorSnapshot, ParallelModeSupervisorState,
};

/*
 * parallel_mode.rs is the TUI adapter for the supersession control tower. The
 * application service owns pool/readiness/lease rules; this file decides when
 * shell commands should refresh snapshots, show overlay chrome, publish status
 * copy, and wake application-owned orchestration work.
 */
use super::parallel_mode_shell_command::{
    PARALLEL_MODE_SHELL_USAGE_TEXT, ParsedParallelModeShellCommand,
    parse_parallel_mode_shell_argument,
};
use super::{
    BackgroundMessage, ConversationInputEvent, ConversationRuntimeEffect, ConversationRuntimeEvent,
    ConversationState, NativeTuiApp, ParallelPanelStateController, ParallelPanelUiEvent,
    ParallelPanelUiState,
};

impl NativeTuiApp {
    pub(crate) fn parallel_mode_enabled(&self) -> bool {
        self.parallel_mode_enabled
    }
    pub(crate) fn parallel_mode_readiness_snapshot(
        &self,
    ) -> Option<&ParallelModeReadinessSnapshot> {
        self.parallel_mode_readiness_snapshot.as_ref()
    }
    pub(crate) fn parallel_mode_service(&self) -> &ParallelModeService {
        &self.parallel_mode_service
    }
    pub(crate) fn parallel_mode_automation_epoch_id(&self) -> Option<u64> {
        self.parallel_mode_control_plane_runtime
            .store()
            .current_epoch_id
    }
    pub(crate) fn parallel_mode_supervisor_refresh_in_flight(&self) -> bool {
        self.parallel_mode_control_plane_runtime
            .store()
            .supervisor_refresh_in_flight
            .is_some()
    }
    pub(crate) fn parallel_mode_orchestrator_wake_in_flight(&self) -> bool {
        self.parallel_mode_control_plane_runtime
            .store()
            .orchestrator_wake_in_flight
            .is_some()
    }
    pub(crate) fn parallel_mode_orchestrator_tick_in_flight(&self) -> bool {
        self.parallel_mode_control_plane_runtime
            .store()
            .orchestrator_tick_in_flight
            .is_some()
    }
    pub(crate) fn parallel_mode_control_effect_in_flight(&self) -> bool {
        self.parallel_mode_supervisor_refresh_in_flight()
            || self.parallel_mode_orchestrator_wake_in_flight()
            || self.parallel_mode_orchestrator_tick_in_flight()
    }
    #[cfg(test)]
    pub(crate) fn set_parallel_mode_automation_epoch_for_test(&mut self, epoch_id: u64) {
        let workspace_directory = self.planning_workspace_directory();
        self.parallel_mode_control_plane_runtime
            .force_epoch_for_test(workspace_directory, epoch_id);
    }
    #[cfg(test)]
    pub(crate) fn mark_parallel_mode_supervisor_refresh_in_flight_for_test(
        &mut self,
    ) -> (u64, ParallelModeControlPlaneEffectId) {
        let workspace_directory = self.planning_workspace_directory();
        let epoch_id = self.parallel_mode_automation_epoch_id().unwrap_or(1);
        let effect_id = self
            .parallel_mode_control_plane_runtime
            .force_supervisor_refresh_in_flight_for_test(workspace_directory, epoch_id);
        (epoch_id, effect_id)
    }
    pub(crate) fn last_parallel_mode_automation_trigger(
        &self,
    ) -> Option<ParallelModeAutomationTrigger> {
        self.last_parallel_mode_automation_trigger
    }
    pub(crate) fn last_parallel_mode_dispatch_withheld_reason(&self) -> Option<&str> {
        self.last_parallel_mode_dispatch_withheld_reason.as_deref()
    }
    pub(crate) fn parallel_mode_supervisor_snapshot(&self) -> ParallelModeSupervisorSnapshot {
        let workspace_directory = self.planning_workspace_directory();
        if let Some(snapshot) = self.parallel_mode_supervisor_snapshot.as_ref()
            && snapshot.workspace_path == workspace_directory
        {
            return snapshot.clone();
        }

        pending_parallel_mode_supervisor_snapshot(
            &workspace_directory,
            self.parallel_mode_enabled(),
            self.parallel_mode_readiness_snapshot(),
            ParallelModeLoadingStage::Entering,
        )
    }

    pub(crate) fn parallel_mode_activity_pulse_visible(&self) -> bool {
        ParallelPanelStateController::activity_pulse_visible(&self.parallel_panel_ui_state())
    }

    pub(crate) fn parallel_mode_loading_prompt_indicator_visible(&self) -> bool {
        ParallelPanelStateController::loading_prompt_indicator_visible(
            &self.parallel_panel_ui_state(),
        )
    }

    pub(crate) fn parallel_mode_prompt_input_locked(&self) -> bool {
        ParallelPanelStateController::prompt_input_locked(&self.parallel_panel_ui_state())
    }

    fn parallel_panel_ui_state(&self) -> ParallelPanelUiState {
        let overlay_event = if self.shell_overlay == ShellOverlay::Supersession {
            ParallelPanelUiEvent::OverlayShown
        } else {
            ParallelPanelUiEvent::OverlayHidden
        };
        let mut events = vec![
            overlay_event,
            ParallelPanelUiEvent::ModeSet(self.parallel_mode_enabled()),
            ParallelPanelUiEvent::SupervisorSnapshotChanged(
                self.parallel_mode_supervisor_snapshot.clone().map(Box::new),
            ),
        ];
        if let Some(reason) = self.last_parallel_mode_dispatch_withheld_reason.as_ref() {
            events.push(ParallelPanelUiEvent::StatusShown(format!(
                "parallel mode: dispatch withheld / {reason}"
            )));
        }
        ParallelPanelStateController::project(events)
    }

    pub(super) fn invalidate_parallel_mode_supervisor_snapshot(&mut self) {
        // Worker dispatch changes leases asynchronously. Keep the last concrete
        // board on screen and refresh a new snapshot off the input/render path.
        let Some(readiness_snapshot) = self.parallel_mode_readiness_snapshot.clone() else {
            if self.parallel_mode_supervisor_snapshot.is_none() {
                let workspace_directory = self.planning_workspace_directory();
                self.parallel_mode_supervisor_snapshot =
                    Some(pending_parallel_mode_supervisor_snapshot(
                        &workspace_directory,
                        self.parallel_mode_enabled(),
                        None,
                        ParallelModeLoadingStage::Entering,
                    ));
            }
            return;
        };

        let workspace_directory = self.planning_workspace_directory();
        if self.parallel_mode_supervisor_snapshot.is_none() {
            self.parallel_mode_supervisor_snapshot =
                Some(pending_parallel_mode_supervisor_snapshot(
                    &workspace_directory,
                    self.parallel_mode_enabled(),
                    Some(&readiness_snapshot),
                    ParallelModeLoadingStage::RefreshingBoard,
                ));
        }
        let outcome = self.parallel_mode_control_plane_runtime.handle(
            ParallelModeControlPlaneCommand::RefreshSupervisor {
                workspace_directory,
            },
        );
        self.apply_parallel_mode_control_plane_outcome(outcome);
    }

    pub(crate) fn refresh_parallel_mode_readiness_snapshot(
        &mut self,
    ) -> ParallelModeReadinessSnapshot {
        // Readiness depends on both repository/runtime checks and planning
        // workspace state. Reload planning first so queue-idle and authority
        // issues are reflected before enabling or dispatching parallel mode.
        let workspace_directory = self.planning_workspace_directory();
        let planning_snapshot = self.load_planning_runtime_snapshot(&workspace_directory);
        let snapshot = self
            .parallel_mode_service()
            .inspect_readiness(&workspace_directory, &planning_snapshot);
        self.parallel_mode_readiness_snapshot = Some(snapshot.clone());
        snapshot
    }

    fn sync_parallel_mode_supervisor_snapshot(
        &mut self,
        execute_pool_actions: bool,
    ) -> ParallelModeSupervisorSnapshot {
        // `build` is a read-only projection for inspection. `reconcile` may
        // create/repair pool worktrees and cleanup reusable slots, so callers opt
        // into it only when the user explicitly enables/refreshes active control.
        let snapshot = if execute_pool_actions {
            self.parallel_mode_service().reconcile_supervisor_snapshot(
                &self.planning_workspace_directory(),
                self.parallel_mode_enabled(),
                self.parallel_mode_readiness_snapshot(),
            )
        } else {
            self.parallel_mode_service().build_supervisor_snapshot(
                &self.planning_workspace_directory(),
                self.parallel_mode_enabled(),
                self.parallel_mode_readiness_snapshot(),
            )
        };
        self.parallel_mode_supervisor_snapshot = Some(snapshot.clone());
        snapshot
    }

    fn apply_parallel_mode_control_plane_outcome(
        &mut self,
        outcome: ParallelModeControlPlaneRuntimeOutcome,
    ) {
        for event in outcome.events {
            match event {
                ParallelModeControlPlaneEvent::StaleCommandDropped {
                    workspace_directory,
                    epoch_id,
                    reason,
                } => {
                    event_log::emit_lazy("parallel_control_plane_stale_command_dropped", || {
                        serde_json::json!({
                            "workspace": workspace_directory,
                            "epoch_id": epoch_id,
                            "reason": reason,
                        })
                    });
                }
                ParallelModeControlPlaneEvent::EffectStarted { effect_id } => {
                    event_log::emit_lazy("parallel_control_plane_effect_started", || {
                        serde_json::json!({
                            "sequence": effect_id.sequence,
                            "kind": effect_id.kind,
                        })
                    });
                }
                ParallelModeControlPlaneEvent::EffectCompleted { effect_id } => {
                    event_log::emit_lazy("parallel_control_plane_effect_completed", || {
                        serde_json::json!({
                            "sequence": effect_id.sequence,
                            "kind": effect_id.kind,
                        })
                    });
                }
                _ => {}
            }
        }
        for effect in outcome.effects {
            self.apply_parallel_mode_control_plane_effect(effect);
        }
    }

    fn apply_parallel_mode_control_plane_effect(&mut self, effect: ParallelModeControlPlaneEffect) {
        match effect {
            ParallelModeControlPlaneEffect::RefreshSupervisor {
                effect_id,
                workspace_directory,
                epoch_id,
            } => {
                let Some(readiness_snapshot) = self.parallel_mode_readiness_snapshot.clone() else {
                    let completion = self.parallel_mode_control_plane_runtime.handle(
                        ParallelModeControlPlaneCommand::EffectCompleted {
                            workspace_directory,
                            epoch_id,
                            effect_id,
                        },
                    );
                    self.apply_parallel_mode_control_plane_outcome(completion);
                    return;
                };
                self.spawn_parallel_mode_supervisor_snapshot_refresh(
                    workspace_directory,
                    readiness_snapshot,
                    epoch_id,
                    effect_id,
                );
            }
            ParallelModeControlPlaneEffect::RunOrchestrator { effect_id, wake } => {
                self.last_parallel_mode_automation_trigger = Some(wake.trigger);
                self.last_parallel_mode_dispatch_withheld_reason = None;
                event_log::emit_lazy("parallel_dispatch_requested", || {
                    serde_json::json!({
                        "trigger": wake.trigger.label(),
                        "workspace": &wake.workspace_directory,
                        "epoch_id": wake.epoch_id,
                        "effect_sequence": effect_id.sequence,
                    })
                });
                self.spawn_parallel_mode_orchestrator_wake_worker(
                    wake.workspace_directory,
                    wake.trigger,
                    wake.epoch_id,
                    wake.enqueue_trigger,
                    effect_id,
                );
            }
            ParallelModeControlPlaneEffect::RunOrchestratorTick {
                effect_id,
                workspace_directory,
                epoch_id,
                signature,
            } => {
                self.spawn_parallel_mode_orchestrator_tick_worker(
                    workspace_directory,
                    signature,
                    epoch_id,
                    effect_id,
                );
            }
            ParallelModeControlPlaneEffect::CancelDispatchCommands {
                workspace_directory,
                reason,
            } => {
                let _ = self
                    .parallel_mode_service
                    .cancel_dispatch_commands(&workspace_directory, &reason);
            }
        }
    }

    fn complete_parallel_mode_control_plane_effect(
        &mut self,
        workspace_directory: &str,
        epoch_id: u64,
        effect_id: ParallelModeControlPlaneEffectId,
    ) -> (bool, ParallelModeControlPlaneRuntimeOutcome) {
        let outcome = self.parallel_mode_control_plane_runtime.handle(
            ParallelModeControlPlaneCommand::EffectCompleted {
                workspace_directory: workspace_directory.to_string(),
                epoch_id,
                effect_id,
            },
        );
        let accepted = outcome.events.iter().any(|event| {
            matches!(
                event,
                ParallelModeControlPlaneEvent::EffectCompleted { effect_id: completed }
                    if *completed == effect_id
            )
        });
        (accepted, outcome)
    }

    pub(super) fn show_supersession_overlay(&mut self) {
        self.dispatch_shell_chrome(ShellChromeEvent::SupersessionOverlayShown);
    }

    pub(super) fn toggle_supersession_overlay(&mut self) {
        self.dispatch_shell_chrome(ShellChromeEvent::SupersessionOverlayToggled);
    }

    pub(super) fn inspect_parallel_mode_shell(&mut self) {
        // Plain inspection is intentionally non-mutating for the pool: refresh
        // readiness and projection, then open the overlay without provisioning or
        // cleaning worktrees.
        self.refresh_parallel_mode_readiness_snapshot();
        self.sync_parallel_mode_supervisor_snapshot(false);
        self.show_supersession_overlay();
    }

    pub(super) fn handle_parallel_shell_command(&mut self, argument: Option<&str>) {
        /*
         * `:parallel` commands are operator controls, not prompt text. Each
         * branch updates the same conversation status line so the inline shell,
         * footer, and popup all report the most recent control action.
         */
        match parse_parallel_mode_shell_argument(argument) {
            Ok(ParsedParallelModeShellCommand::Disable) => {
                // Turning off parallel mode is local UI state. Keep the snapshot
                // read-only and close the control tower so normal shell focus
                // resumes immediately.
                self.close_parallel_mode_automation_epoch();
                self.parallel_mode_enabled = false;
                self.sync_parallel_mode_supervisor_snapshot(false);
                if self.shell_overlay == ShellOverlay::Supersession {
                    self.close_shell_overlay();
                }
                self.dispatch_conversation_input(ConversationInputEvent::StatusMessageShown {
                    status_text: "parallel mode: off / shell returned to normal mode".to_string(),
                });
            }
            Err(error) => {
                // Unsupported arguments still open the control tower. That makes
                // the supported commands and current readiness visible next to
                // the error copy.
                self.inspect_parallel_mode_shell();
                self.dispatch_conversation_input(ConversationInputEvent::StatusMessageShown {
                    status_text: format!(
                        "parallel mode command: unsupported argument `{}` / {}",
                        error.argument(),
                        PARALLEL_MODE_SHELL_USAGE_TEXT
                    ),
                });
            }
            Ok(ParsedParallelModeShellCommand::Enable) => {
                // Bare `:parallel` is the only enable entrypoint. Open the
                // control tower first, then let readiness/reconcile run off the
                // terminal event loop so prompt typing stays responsive.
                // A destructive pool reset belongs to the local off -> on
                // transition. Re-running `:parallel` while already enabled only
                // refreshes/reconciles; after `:parallel off`, the next
                // `:parallel` resets the disposable pool again.
                let workspace_directory = self.planning_workspace_directory();
                let reset_pool_on_off_to_on_entry = !self.parallel_mode_enabled;
                self.parallel_mode_enabled = true;
                self.parallel_mode_readiness_snapshot = None;
                self.parallel_mode_control_plane_runtime
                    .reset_orchestrator_tick_signature();
                self.parallel_mode_supervisor_snapshot =
                    Some(pending_parallel_mode_supervisor_snapshot(
                        &workspace_directory,
                        true,
                        None,
                        ParallelModeLoadingStage::Entering,
                    ));
                self.show_supersession_overlay();
                self.spawn_parallel_mode_enter_worker(
                    workspace_directory,
                    reset_pool_on_off_to_on_entry,
                );
                self.dispatch_conversation_input(ConversationInputEvent::StatusMessageShown {
                    status_text:
                        "parallel mode: loading 1/3 / checking readiness before pool setup"
                            .to_string(),
                });
            }
        }
    }

    fn spawn_parallel_mode_supervisor_snapshot_refresh(
        &self,
        workspace_directory: String,
        readiness_snapshot: ParallelModeReadinessSnapshot,
        epoch_id: u64,
        effect_id: ParallelModeControlPlaneEffectId,
    ) {
        let parallel_mode_service = self.parallel_mode_service.clone();
        let mode_enabled = self.parallel_mode_enabled();
        let tx = self.tx.clone();

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
            let _ = tx.send(BackgroundMessage::ParallelModeSupervisorSnapshotRefreshed {
                workspace_directory,
                epoch_id,
                effect_id,
                supervisor_snapshot: Box::new(supervisor_snapshot),
            });
        });
    }

    fn maybe_spawn_parallel_mode_orchestrator_tick(&mut self, workspace_directory: &str) {
        if !self.parallel_mode_enabled()
            || self.planning_workspace_directory() != workspace_directory
            || self.parallel_mode_control_effect_in_flight()
        {
            return;
        }
        if !self
            .parallel_mode_readiness_snapshot
            .as_ref()
            .is_some_and(ParallelModeReadinessSnapshot::allows_parallel_mode)
        {
            return;
        }
        let Some(snapshot) = self.parallel_mode_supervisor_snapshot.as_ref() else {
            return;
        };
        let Some(signature) = parallel_mode_distributor_tick_signature(snapshot) else {
            return;
        };
        let outcome = self.parallel_mode_control_plane_runtime.handle(
            ParallelModeControlPlaneCommand::RunOrchestratorTick {
                workspace_directory: workspace_directory.to_string(),
                signature,
            },
        );
        self.apply_parallel_mode_control_plane_outcome(outcome);
    }

    fn spawn_parallel_mode_orchestrator_tick_worker(
        &self,
        workspace_directory: String,
        signature: String,
        epoch_id: u64,
        effect_id: ParallelModeControlPlaneEffectId,
    ) {
        let parallel_mode_service = self.parallel_mode_service.clone();
        let tx = self.tx.clone();

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
            let _ = tx.send(BackgroundMessage::ParallelModeOrchestratorTickCompleted {
                workspace_directory,
                epoch_id,
                effect_id,
                blocked,
                notices,
            });
        });
    }

    fn spawn_parallel_mode_enter_worker(
        &self,
        workspace_directory: String,
        reset_pool_on_off_to_on_entry: bool,
    ) {
        let parallel_mode_service = self.parallel_mode_service.clone();
        let planning = self.planning.clone();
        let tx = self.tx.clone();

        thread::spawn(move || {
            let planning_snapshot = planning
                .runtime
                .load_runtime_snapshot_or_invalid(&workspace_directory);
            let readiness_snapshot =
                parallel_mode_service.inspect_readiness(&workspace_directory, &planning_snapshot);
            let entry_plan = ParallelModeOrchestratorStateMachine::plan_parallel_entry(
                !reset_pool_on_off_to_on_entry,
                readiness_snapshot.allows_parallel_mode(),
            );
            event_log::emit_lazy("parallel_action_planned", || {
                serde_json::json!({
                    "workspace": &workspace_directory,
                    "state": entry_plan.state.label(),
                    "reset_scope": entry_plan.reset_scope.map(|scope| scope.label()),
                    "readiness": readiness_snapshot.readiness_label(),
                })
            });

            let (supervisor_snapshot, status_text) = if readiness_snapshot.allows_parallel_mode() {
                let _ = tx.send(BackgroundMessage::ParallelModeEnterProgress {
                    workspace_directory: workspace_directory.clone(),
                    readiness_snapshot: Some(readiness_snapshot.clone()),
                    supervisor_snapshot: Box::new(pending_parallel_mode_supervisor_snapshot(
                        &workspace_directory,
                        true,
                        Some(&readiness_snapshot),
                        ParallelModeLoadingStage::ReconcilingPool,
                    )),
                    status_text:
                        "parallel mode: loading 2/3 / readiness complete; reconciling pool"
                            .to_string(),
                });
                let reset_result = if entry_plan.reset_scope
                    == Some(ParallelModePoolResetScope::PoolOnly)
                {
                    event_log::emit_lazy("parallel_pool_reset_started", || {
                        serde_json::json!({
                            "workspace": &workspace_directory,
                            "reset_scope": ParallelModePoolResetScope::PoolOnly.label(),
                        })
                    });
                    parallel_mode_service
                        .reset_pool_on_parallel_enable_report(&workspace_directory)
                        .and_then(|report| {
                            if report.has_live_blockers() {
                                event_log::emit_lazy("parallel_pool_reset_preserved_live", || {
                                    serde_json::json!({
                                        "workspace": &workspace_directory,
                                        "reset_scope": ParallelModePoolResetScope::PoolOnly.label(),
                                        "run_id": report.run_id.as_str(),
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
                                    "slot_count": count,
                                })
                            });
                            let live_suffix = if report.has_live_blockers() {
                                format!(" / preserved {} live slot(s)", report.live_blocker_count())
                            } else {
                                String::new()
                            };
                            Ok(format!(
                                "reset {count} pool slot worktree(s) to prerelease after off->on entry{live_suffix} / {}",
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
                        return {
                            let _ = tx.send(BackgroundMessage::ParallelModeEntered {
                                workspace_directory,
                                readiness_snapshot,
                                supervisor_snapshot: Box::new(supervisor_snapshot),
                                status_text,
                            });
                        };
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

            let _ = tx.send(BackgroundMessage::ParallelModeEntered {
                workspace_directory,
                readiness_snapshot,
                supervisor_snapshot: Box::new(supervisor_snapshot),
                status_text,
            });
        });
    }

    pub(super) fn refresh_parallel_mode_dispatch_after_task_update(&mut self, task_id: &str) {
        if !self.parallel_mode_enabled() {
            return;
        }

        if self.parallel_mode_automation_epoch_id().is_none() {
            let reason = format!(
                "task update `{task_id}` accepted before a parallel automation epoch opened"
            );
            self.record_parallel_mode_dispatch_withheld(
                Some(ParallelModeAutomationTrigger::TaskIntakeAfterEpoch),
                &reason,
            );
            self.dispatch_conversation_input(ConversationInputEvent::StatusMessageShown {
                status_text: format!("parallel mode: dispatch withheld / {reason}"),
            });
            return;
        }

        let workspace_directory = self.planning_workspace_directory();
        self.wake_parallel_mode_orchestrator(
            workspace_directory,
            ParallelModeAutomationTrigger::TaskIntakeAfterEpoch,
        );
    }

    fn parallel_mode_orchestrator_wake_should_defer(&self) -> bool {
        if self.parallel_mode_readiness_snapshot.is_none() {
            return true;
        }

        match self.parallel_mode_supervisor_snapshot.as_ref() {
            Some(snapshot) => ParallelPanelStateController::snapshot_is_loading(snapshot),
            None => true,
        }
    }

    pub(super) fn parallel_mode_post_turn_queue_signal(
        &self,
        event: &ConversationRuntimeEvent,
    ) -> Option<ParallelModePostTurnQueueSignal> {
        let ConversationRuntimeEvent::PostTurnAutomationEvaluated { evaluation } = event else {
            return None;
        };
        evaluation.provenance.parallel_queue_signal
    }

    pub(super) fn apply_parallel_mode_post_turn_queue_continuation(
        &mut self,
        effects: &mut Vec<ConversationRuntimeEffect>,
        event_signal: Option<ParallelModePostTurnQueueSignal>,
    ) {
        let effect_signal = effects
            .iter()
            .any(|effect| matches!(effect, ConversationRuntimeEffect::QueueAutoPrompt { .. }))
            .then_some(ParallelModePostTurnQueueSignal::AutoFollowQueued);
        let signal = effect_signal.or(event_signal);
        let has_actionable_queue_head = match &self.conversation_state {
            ConversationState::Ready(conversation) => conversation
                .planning_runtime_snapshot
                .has_actionable_queue_head(),
            ConversationState::Loading | ConversationState::Failed(_) => false,
        };
        let decision = ParallelModeOrchestratorStateMachine::post_turn_queue_continuation(
            self.parallel_mode_enabled(),
            signal,
            has_actionable_queue_head,
        );
        let Some(trigger) = decision.dispatch_trigger() else {
            return;
        };

        if decision.should_consume_auto_follow_prompt() {
            effects.retain(|effect| {
                !matches!(effect, ConversationRuntimeEffect::QueueAutoPrompt { .. })
            });
        }
        if let ConversationState::Ready(conversation) = &mut self.conversation_state
            && decision.should_consume_auto_follow_prompt()
        {
            conversation.record_auto_follow_parallel_dispatch();
        }
        let epoch_id = self.open_parallel_mode_automation_epoch();
        let workspace_directory = self.planning_workspace_directory();
        self.dispatch_conversation_input(ConversationInputEvent::StatusMessageShown {
            status_text: format!(
                "parallel mode: automation epoch {epoch_id} opened / dispatching accepted queue"
            ),
        });
        self.wake_parallel_mode_orchestrator(workspace_directory, trigger);
    }

    fn open_parallel_mode_automation_epoch(&mut self) -> u64 {
        if let Some(epoch_id) = self.parallel_mode_automation_epoch_id() {
            return epoch_id;
        }

        let workspace_directory = self.planning_workspace_directory();
        let outcome = self.parallel_mode_control_plane_runtime.handle(
            ParallelModeControlPlaneCommand::OpenEpoch {
                workspace_directory: workspace_directory.clone(),
            },
        );
        let epoch_id = self
            .parallel_mode_automation_epoch_id()
            .expect("open epoch command should create a current epoch");
        self.last_parallel_mode_dispatch_withheld_reason = None;
        self.apply_parallel_mode_control_plane_outcome(outcome);
        event_log::emit_lazy("parallel_automation_epoch_opened", || {
            serde_json::json!({
                "workspace": workspace_directory,
                "epoch_id": epoch_id,
            })
        });
        epoch_id
    }

    fn open_parallel_mode_automation_epoch_for_entry(&mut self) -> (u64, bool) {
        if let Some(epoch_id) = self.parallel_mode_automation_epoch_id() {
            return (epoch_id, false);
        }

        (self.open_parallel_mode_automation_epoch(), true)
    }

    fn close_parallel_mode_automation_epoch(&mut self) {
        let workspace_directory = self.planning_workspace_directory();
        let epoch_id = self.parallel_mode_automation_epoch_id();
        let outcome = self.parallel_mode_control_plane_runtime.handle(
            ParallelModeControlPlaneCommand::Disable {
                workspace_directory: workspace_directory.clone(),
            },
        );
        self.apply_parallel_mode_control_plane_outcome(outcome);
        self.last_parallel_mode_dispatch_withheld_reason = None;
        if let Some(epoch_id) = epoch_id {
            event_log::emit_lazy("parallel_automation_epoch_closed", || {
                serde_json::json!({
                    "workspace": workspace_directory,
                    "epoch_id": epoch_id,
                })
            });
        }
    }

    fn wake_parallel_mode_orchestrator(
        &mut self,
        workspace_directory: String,
        trigger: ParallelModeAutomationTrigger,
    ) {
        let Some(epoch_id) = self.parallel_mode_automation_epoch_id() else {
            self.record_parallel_mode_dispatch_withheld(
                Some(trigger),
                "automation epoch is not open",
            );
            return;
        };

        if self.parallel_mode_orchestrator_wake_should_defer() {
            let queue_detail = match self.enqueue_parallel_mode_orchestrator_command(
                &workspace_directory,
                trigger,
                epoch_id,
                "entry loading or supervisor refresh is still in progress",
            ) {
                Ok(0) => "orchestrator wake already queued".to_string(),
                Ok(_) => {
                    "entry loading or supervisor refresh is still in progress; orchestrator wake queued"
                        .to_string()
                }
                Err(error) => format!("orchestrator wake queue failed: {error}"),
            };
            self.record_parallel_mode_dispatch_withheld(Some(trigger), &queue_detail);
            self.dispatch_conversation_input(ConversationInputEvent::StatusMessageShown {
                status_text:
                    "parallel mode: dispatch deferred until control-plane refresh finishes"
                        .to_string(),
            });
            return;
        }

        if self.parallel_mode_orchestrator_wake_in_flight() {
            let queue_detail = match self.enqueue_parallel_mode_orchestrator_command(
                &workspace_directory,
                trigger,
                epoch_id,
                "orchestrator wake already in flight",
            ) {
                Ok(0) => "orchestrator wake already queued".to_string(),
                Ok(_) => "orchestrator wake already in flight; next wake queued".to_string(),
                Err(error) => format!("orchestrator wake queue failed: {error}"),
            };
            self.record_parallel_mode_dispatch_withheld(Some(trigger), &queue_detail);
            return;
        }

        let outcome = self.parallel_mode_control_plane_runtime.handle(
            ParallelModeControlPlaneCommand::WakeOrchestrator(ParallelModeControlPlaneWake::new(
                workspace_directory,
                trigger,
                epoch_id,
                Some(trigger),
            )),
        );
        self.apply_parallel_mode_control_plane_outcome(outcome);
    }

    fn maybe_start_parallel_mode_entry_dispatch(&mut self, workspace_directory: &str) -> bool {
        let planning_snapshot = self
            .planning
            .runtime
            .load_runtime_snapshot_or_invalid(workspace_directory);
        if !planning_snapshot.has_actionable_queue_head() {
            return false;
        }

        let Some(epoch_id) = self.parallel_mode_automation_epoch_id() else {
            return false;
        };
        self.dispatch_conversation_input(ConversationInputEvent::StatusMessageShown {
            status_text: format!(
                "parallel mode: automation epoch {epoch_id} opened / dispatching ready queue"
            ),
        });
        self.wake_parallel_mode_orchestrator(
            workspace_directory.to_string(),
            ParallelModeAutomationTrigger::TaskIntakeAfterEpoch,
        );
        true
    }

    pub(super) fn apply_parallel_mode_orchestrator_wake_request(
        &mut self,
        workspace_directory: String,
        trigger: ParallelModeAutomationTrigger,
        epoch_id: u64,
    ) {
        if self.parallel_mode_automation_epoch_id() != Some(epoch_id) {
            event_log::emit_lazy("parallel_dispatch_blocked", || {
                serde_json::json!({
                    "trigger": trigger.label(),
                    "workspace": workspace_directory,
                    "epoch_id": epoch_id,
                    "blocked_reason": "stale automation epoch",
                })
            });
            return;
        }
        self.wake_parallel_mode_orchestrator(workspace_directory, trigger);
    }

    fn enqueue_parallel_mode_orchestrator_command(
        &mut self,
        workspace_directory: &str,
        trigger: ParallelModeAutomationTrigger,
        epoch_id: u64,
        reason: &str,
    ) -> Result<usize, String> {
        let planning_snapshot = self
            .planning
            .runtime
            .load_runtime_snapshot_or_invalid(workspace_directory);
        let inserted_count = self
            .parallel_mode_service
            .enqueue_dispatch_commands_for_trigger(
                workspace_directory,
                trigger,
                &planning_snapshot,
                Some(epoch_id),
            )?;
        self.last_parallel_mode_automation_trigger = Some(trigger);
        event_log::emit_lazy("parallel_orchestrator_wake_queued", || {
            serde_json::json!({
                "trigger": trigger.label(),
                "workspace": workspace_directory,
                "epoch_id": epoch_id,
                "inserted_count": inserted_count,
                "reason": reason,
            })
        });
        Ok(inserted_count)
    }

    fn enqueue_parallel_mode_slot_capacity_command(
        &mut self,
        workspace_directory: &str,
    ) -> Result<usize, String> {
        let Some(epoch_id) = self.parallel_mode_automation_epoch_id() else {
            return Ok(0);
        };
        let planning_snapshot = self
            .planning
            .runtime
            .load_runtime_snapshot_or_invalid(workspace_directory);
        self.parallel_mode_service
            .enqueue_dispatch_commands_for_event(
                workspace_directory,
                ParallelModeRuntimeEvent::SlotCapacityAvailable,
                &planning_snapshot,
                Some(epoch_id),
            )
    }

    pub(super) fn maybe_wake_parallel_mode_orchestrator_for_pending_command(&mut self) -> bool {
        if !self.parallel_mode_enabled() || self.parallel_mode_automation_epoch_id().is_none() {
            return false;
        }
        if self.parallel_mode_orchestrator_wake_should_defer()
            || self.parallel_mode_control_effect_in_flight()
        {
            return false;
        }
        let workspace_directory = self.planning_workspace_directory();
        let epoch_id = self
            .parallel_mode_automation_epoch_id()
            .expect("checked automation epoch should exist");
        match self
            .parallel_mode_service
            .pending_dispatch_wake(&workspace_directory, epoch_id)
        {
            Ok(None) => false,
            Ok(Some(wake)) => {
                let outcome = self
                    .parallel_mode_control_plane_runtime
                    .handle(ParallelModeControlPlaneCommand::WakeOrchestrator(wake));
                self.apply_parallel_mode_control_plane_outcome(outcome);
                true
            }
            Err(error) => {
                self.record_parallel_mode_dispatch_withheld(
                    Some(ParallelModeAutomationTrigger::TaskIntakeAfterEpoch),
                    &format!("pending dispatch command poll failed: {error}"),
                );
                false
            }
        }
    }

    fn record_parallel_mode_dispatch_withheld(
        &mut self,
        trigger: Option<ParallelModeAutomationTrigger>,
        reason: &str,
    ) {
        self.last_parallel_mode_automation_trigger = trigger;
        self.last_parallel_mode_dispatch_withheld_reason = Some(reason.to_string());
        event_log::emit_lazy("parallel_dispatch_blocked", || {
            serde_json::json!({
                "trigger": trigger.map(|value| value.label()),
                "workspace": self.planning_workspace_directory(),
                "epoch_id": self.parallel_mode_automation_epoch_id(),
                "blocked_reason": reason,
            })
        });
    }

    fn spawn_parallel_mode_orchestrator_wake_worker(
        &self,
        workspace_directory: String,
        trigger: ParallelModeAutomationTrigger,
        epoch_id: u64,
        enqueue_trigger: Option<ParallelModeAutomationTrigger>,
        effect_id: ParallelModeControlPlaneEffectId,
    ) {
        let parallel_mode_service = self.parallel_mode_service.clone();
        let parallel_agent_worker_port = self.parallel_agent_worker_port.clone();
        let parallel_mode_turn_service = self.parallel_mode_turn_service();
        let planning = self.planning.clone();
        let tx = self.tx.clone();

        thread::spawn(move || {
            let (loop_event_tx, loop_event_rx) = mpsc::channel();
            let loop_bridge_tx = tx.clone();
            thread::spawn(move || {
                while let Ok(event) = loop_event_rx.recv() {
                    let _ = loop_bridge_tx.send(background_message_from_parallel_loop_event(event));
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

            let _ = tx.send(BackgroundMessage::ParallelModeOrchestratorWakeCompleted {
                workspace_directory: result.workspace_directory,
                effect_id,
                readiness_snapshot: result.readiness_snapshot,
                supervisor_snapshot: Box::new(result.supervisor_snapshot),
                outcome: result.outcome,
            });
        });
    }

    pub(super) fn apply_parallel_mode_enter_progress(
        &mut self,
        workspace_directory: &str,
        readiness_snapshot: Option<ParallelModeReadinessSnapshot>,
        supervisor_snapshot: ParallelModeSupervisorSnapshot,
        status_text: String,
    ) {
        if !self.parallel_mode_enabled()
            || self.planning_workspace_directory() != workspace_directory
        {
            return;
        }

        if let Some(readiness_snapshot) = readiness_snapshot {
            self.parallel_mode_readiness_snapshot = Some(readiness_snapshot);
        }
        self.parallel_mode_supervisor_snapshot = Some(supervisor_snapshot);
        self.dispatch_conversation_input(ConversationInputEvent::StatusMessageShown {
            status_text,
        });
    }

    pub(super) fn apply_parallel_mode_entered(
        &mut self,
        workspace_directory: &str,
        readiness_snapshot: ParallelModeReadinessSnapshot,
        supervisor_snapshot: ParallelModeSupervisorSnapshot,
        status_text: String,
    ) {
        // A delayed enter result should not reopen parallel mode after the user
        // has already switched it off or moved to another workspace.
        if !self.parallel_mode_enabled()
            || self.planning_workspace_directory() != workspace_directory
        {
            return;
        }

        self.parallel_mode_enabled = readiness_snapshot.allows_parallel_mode();
        self.parallel_mode_readiness_snapshot = Some(readiness_snapshot);
        self.parallel_mode_supervisor_snapshot = Some(supervisor_snapshot);
        self.refresh_ready_conversation_planning_runtime_snapshot_for_workspace(
            workspace_directory,
        );
        self.dispatch_conversation_input(ConversationInputEvent::StatusMessageShown {
            status_text,
        });
        if self.parallel_mode_enabled {
            let (_, opened_epoch) = self.open_parallel_mode_automation_epoch_for_entry();
            let entry_dispatch_started =
                opened_epoch && self.maybe_start_parallel_mode_entry_dispatch(workspace_directory);
            if !entry_dispatch_started {
                self.maybe_wake_parallel_mode_orchestrator_for_pending_command();
            }
            self.maybe_spawn_parallel_mode_orchestrator_tick(workspace_directory);
        }
    }

    pub(super) fn apply_parallel_mode_supervisor_snapshot_refreshed(
        &mut self,
        workspace_directory: &str,
        epoch_id: u64,
        effect_id: ParallelModeControlPlaneEffectId,
        supervisor_snapshot: ParallelModeSupervisorSnapshot,
    ) {
        if !self.parallel_mode_enabled()
            || self.planning_workspace_directory() != workspace_directory
        {
            let outcome = self.parallel_mode_control_plane_runtime.handle(
                ParallelModeControlPlaneCommand::Disable {
                    workspace_directory: workspace_directory.to_string(),
                },
            );
            self.apply_parallel_mode_control_plane_outcome(outcome);
            return;
        }

        let (effect_completed, outcome) = self.complete_parallel_mode_control_plane_effect(
            workspace_directory,
            epoch_id,
            effect_id,
        );
        if !effect_completed {
            self.apply_parallel_mode_control_plane_outcome(outcome);
            return;
        }

        self.parallel_mode_supervisor_snapshot = Some(supervisor_snapshot);
        self.apply_parallel_mode_control_plane_outcome(outcome);
        self.maybe_wake_parallel_mode_orchestrator_for_pending_command();
        self.maybe_spawn_parallel_mode_orchestrator_tick(workspace_directory);
    }

    pub(super) fn apply_parallel_mode_orchestrator_wake_completed(
        &mut self,
        workspace_directory: &str,
        effect_id: ParallelModeControlPlaneEffectId,
        readiness_snapshot: ParallelModeReadinessSnapshot,
        supervisor_snapshot: ParallelModeSupervisorSnapshot,
        outcome: ParallelModeDispatchOutcome,
    ) {
        if !self.parallel_mode_enabled()
            || self.planning_workspace_directory() != workspace_directory
        {
            let runtime_outcome = self.parallel_mode_control_plane_runtime.handle(
                ParallelModeControlPlaneCommand::Disable {
                    workspace_directory: workspace_directory.to_string(),
                },
            );
            self.apply_parallel_mode_control_plane_outcome(runtime_outcome);
            return;
        }

        let (effect_completed, runtime_outcome) = self.complete_parallel_mode_control_plane_effect(
            workspace_directory,
            outcome.epoch_id,
            effect_id,
        );
        if self.parallel_mode_automation_epoch_id() != Some(outcome.epoch_id) || !effect_completed {
            self.apply_parallel_mode_control_plane_outcome(runtime_outcome);
            return;
        }

        self.parallel_mode_enabled = readiness_snapshot.allows_parallel_mode();
        self.parallel_mode_readiness_snapshot = Some(readiness_snapshot);
        self.parallel_mode_supervisor_snapshot = Some(supervisor_snapshot);
        self.refresh_ready_conversation_planning_runtime_snapshot_for_workspace(
            workspace_directory,
        );
        self.last_parallel_mode_automation_trigger = Some(outcome.trigger);
        self.last_parallel_mode_dispatch_withheld_reason = outcome.blocked_reason.clone();
        let status_text = format!(
            "parallel mode: dispatch refreshed / trigger: {} / {}",
            outcome.trigger.label(),
            outcome.status_detail()
        );
        self.dispatch_conversation_input(ConversationInputEvent::StatusMessageShown {
            status_text,
        });
        event_log::emit_lazy("parallel_dispatch_completed", || {
            serde_json::json!({
                "trigger": outcome.trigger.label(),
                "workspace": workspace_directory,
                "epoch_id": outcome.epoch_id,
                "idle_slot_count": outcome.idle_slot_count,
                "task_ids": outcome.candidate_task_ids,
                "launched_count": outcome.launched_task_ids.len(),
                "blocked_reason": outcome.blocked_reason,
            })
        });
        self.apply_parallel_mode_control_plane_outcome(runtime_outcome);
        self.maybe_wake_parallel_mode_orchestrator_for_pending_command();
        self.maybe_spawn_parallel_mode_orchestrator_tick(workspace_directory);
    }

    pub(super) fn apply_parallel_mode_worker_event(
        &mut self,
        event: ParallelModeControlPlaneWorkerEvent,
    ) {
        let current_workspace_directory = self.planning_workspace_directory();
        let has_actionable_queue_head = if current_workspace_directory == event.workspace_directory
        {
            self.planning
                .runtime
                .load_runtime_snapshot_or_invalid(&event.workspace_directory)
                .has_actionable_queue_head()
        } else {
            false
        };
        let outcome = event.reduce(
            &current_workspace_directory,
            self.parallel_mode_automation_epoch_id(),
            has_actionable_queue_head,
        );
        if let Some(reason) = outcome.stale_drop_reason {
            event_log::emit_lazy("parallel_worker_event_dropped", || {
                serde_json::json!({
                    "workspace": current_workspace_directory,
                    "epoch_id": self.parallel_mode_automation_epoch_id(),
                    "reason": reason,
                })
            });
            return;
        }

        for notice in outcome.notices {
            self.dispatch_conversation_runtime(ConversationRuntimeEvent::StreamExecutionObserved {
                notice,
            });
        }
        if outcome.refresh_supervisor {
            self.invalidate_parallel_mode_supervisor_snapshot();
        }
        if let Some(wake) = outcome.wake {
            self.apply_parallel_mode_orchestrator_wake_request(
                wake.workspace_directory,
                wake.trigger,
                wake.epoch_id,
            );
        }
    }

    pub(super) fn apply_parallel_mode_orchestrator_tick_completed(
        &mut self,
        workspace_directory: &str,
        epoch_id: u64,
        effect_id: ParallelModeControlPlaneEffectId,
        blocked: bool,
        notices: Vec<String>,
    ) {
        if !self.parallel_mode_enabled()
            || self.planning_workspace_directory() != workspace_directory
        {
            let outcome = self.parallel_mode_control_plane_runtime.handle(
                ParallelModeControlPlaneCommand::Disable {
                    workspace_directory: workspace_directory.to_string(),
                },
            );
            self.apply_parallel_mode_control_plane_outcome(outcome);
            return;
        }

        let (effect_completed, outcome) = self.complete_parallel_mode_control_plane_effect(
            workspace_directory,
            epoch_id,
            effect_id,
        );
        if !effect_completed {
            self.apply_parallel_mode_control_plane_outcome(outcome);
            return;
        }

        let notice_count = notices.len();
        for notice in notices {
            self.dispatch_conversation_runtime(ConversationRuntimeEvent::StreamExecutionObserved {
                notice,
            });
        }
        self.refresh_ready_conversation_planning_runtime_snapshot_for_workspace(
            workspace_directory,
        );
        self.invalidate_parallel_mode_supervisor_snapshot();
        if !blocked {
            match self.enqueue_parallel_mode_slot_capacity_command(workspace_directory) {
                Ok(0) => {}
                Ok(_) => {
                    self.last_parallel_mode_automation_trigger =
                        Some(ParallelModeAutomationTrigger::TaskIntakeAfterEpoch);
                    self.last_parallel_mode_dispatch_withheld_reason = None;
                }
                Err(error) => {
                    self.record_parallel_mode_dispatch_withheld(
                        Some(ParallelModeAutomationTrigger::TaskIntakeAfterEpoch),
                        &format!("slot-capacity dispatch queue failed: {error}"),
                    );
                }
            }
        }
        let status_text = if blocked {
            format!("parallel mode: distributor retry blocked / notices: {notice_count}")
        } else {
            format!("parallel mode: distributor retry completed / notices: {notice_count}")
        };
        self.dispatch_conversation_input(ConversationInputEvent::StatusMessageShown {
            status_text,
        });
        self.apply_parallel_mode_control_plane_outcome(outcome);
    }
}

fn parallel_mode_distributor_tick_signature(
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

fn background_message_from_parallel_loop_event(
    event: ParallelModeOrchestratorLoopEvent,
) -> BackgroundMessage {
    match event {
        ParallelModeOrchestratorLoopEvent::ConversationRuntimeNotice(notice) => {
            BackgroundMessage::ConversationRuntimeNotice(notice)
        }
        ParallelModeOrchestratorLoopEvent::WorkerEvent(event) => {
            BackgroundMessage::ParallelModeWorkerEvent(event)
        }
    }
}

#[derive(Clone, Copy)]
enum ParallelModeLoadingStage {
    Entering,
    ReconcilingPool,
    RefreshingBoard,
}

impl ParallelModeLoadingStage {
    fn pool_root_label(self) -> &'static str {
        match self {
            Self::Entering => "loading: readiness checks",
            Self::ReconcilingPool => "loading: pool reconcile",
            Self::RefreshingBoard => "loading: supervisor refresh",
        }
    }

    fn pool_status(self) -> &'static str {
        match self {
            Self::Entering => "1/3 readiness checks running",
            Self::ReconcilingPool => "2/3 pool reconcile running",
            Self::RefreshingBoard => "3/3 refreshing supervisor board",
        }
    }

    fn roster_empty_state(self) -> &'static str {
        match self {
            Self::Entering => "waiting for readiness before slots can be assigned",
            Self::ReconcilingPool => "waiting for pool reset and reconcile results",
            Self::RefreshingBoard => "refreshing active agent roster",
        }
    }

    fn detail_empty_state(self) -> &'static str {
        match self {
            Self::Entering => "loading 1/3: readiness checks",
            Self::ReconcilingPool => "loading 2/3: pool reconcile",
            Self::RefreshingBoard => "loading 3/3: board refresh",
        }
    }

    fn distributor_head(self) -> &'static str {
        match self {
            Self::Entering => "waiting for readiness",
            Self::ReconcilingPool => "pool reconcile in progress",
            Self::RefreshingBoard => "refreshing distributor state",
        }
    }

    fn distributor_note(self) -> &'static str {
        match self {
            Self::Entering => "pipeline: [running] readiness -> [next] pool -> [next] board",
            Self::ReconcilingPool => "pipeline: [done] readiness -> [running] pool -> [next] board",
            Self::RefreshingBoard => "pipeline: [done] readiness -> [done] pool -> [running] board",
        }
    }

    fn top_notice(self) -> &'static str {
        match self {
            Self::Entering => {
                "loading 1/3: checking repository, planning, branch, pool, and GitHub readiness"
            }
            Self::ReconcilingPool => "loading 2/3: readiness passed; reconciling pool",
            Self::RefreshingBoard => {
                "loading 3/3: pool state changed; refreshing the supervisor board"
            }
        }
    }
}

fn pending_parallel_mode_supervisor_snapshot(
    workspace_directory: &str,
    mode_enabled: bool,
    readiness_snapshot: Option<&ParallelModeReadinessSnapshot>,
    stage: ParallelModeLoadingStage,
) -> ParallelModeSupervisorSnapshot {
    ParallelModeSupervisorSnapshot::new(
        ParallelModeSupervisorState::derive(mode_enabled, readiness_snapshot),
        workspace_directory,
        ParallelModePoolBoardSnapshot::new(
            0,
            stage.pool_root_label(),
            stage.pool_status(),
            Vec::new(),
        ),
        ParallelModeAgentRosterSnapshot::new(Vec::new(), stage.roster_empty_state()),
        ParallelModeSupervisorDetailSnapshot::new(None, stage.detail_empty_state()),
        ParallelModeDistributorSnapshot::new(
            Vec::new(),
            Vec::new(),
            stage.distributor_head(),
            stage.distributor_note(),
        ),
        Some(stage.top_notice().to_string()),
    )
}

impl NativeTuiApp {
    pub(super) fn handle_supersession_overlay_key(&mut self, key: event::KeyEvent) -> bool {
        // Return false outside the overlay so the normal shell keymap can handle
        // the event. Supersession shortcuts are scoped to the control tower.
        if self.shell_overlay != ShellOverlay::Supersession {
            return false;
        }
        match key.code {
            KeyCode::Char('r') if key.modifiers == KeyModifiers::CONTROL => {
                // Ctrl+R is the operator's explicit "re-read the world" command:
                // readiness is refreshed and supervisor projection is synced
                // using the current enabled state.
                let snapshot = self.refresh_parallel_mode_readiness_snapshot();
                self.sync_parallel_mode_supervisor_snapshot(self.parallel_mode_enabled());
                self.dispatch_conversation_input(ConversationInputEvent::StatusMessageShown {
                    status_text: format!(
                        "parallel readiness refreshed / state: {}",
                        snapshot.readiness_label()
                    ),
                });
            }
            KeyCode::Char('o') if key.modifiers == KeyModifiers::CONTROL => {
                // Ctrl+O hides the tower without changing mode. Active workers
                // continue and can be inspected later.
                self.close_shell_overlay();
            }
            KeyCode::Char('p') if key.modifiers == KeyModifiers::CONTROL => {
                // Ctrl+P is the emergency local off-switch and reuses the same
                // command path as `:parallel off` so status copy stays identical.
                self.handle_parallel_shell_command(Some("off"));
            }
            KeyCode::Tab if key.modifiers.is_empty() => {
                self.supersession_mud_ui_state.focus_next_zone();
                let snapshot = self.parallel_mode_supervisor_snapshot();
                self.supersession_mud_ui_state.clamp_to_snapshot(&snapshot);
            }
            KeyCode::BackTab => {
                self.supersession_mud_ui_state.focus_previous_zone();
                let snapshot = self.parallel_mode_supervisor_snapshot();
                self.supersession_mud_ui_state.clamp_to_snapshot(&snapshot);
            }
            KeyCode::Left | KeyCode::Up if key.modifiers.is_empty() => {
                let snapshot = self.parallel_mode_supervisor_snapshot();
                self.supersession_mud_ui_state.move_selection(&snapshot, -1);
            }
            KeyCode::Right | KeyCode::Down if key.modifiers.is_empty() => {
                let snapshot = self.parallel_mode_supervisor_snapshot();
                self.supersession_mud_ui_state.move_selection(&snapshot, 1);
            }
            KeyCode::Enter
                if key.modifiers.is_empty() && self.parallel_mode_prompt_input_locked() =>
            {
                let snapshot = self.parallel_mode_supervisor_snapshot();
                self.supersession_mud_ui_state.inspect_focused(&snapshot);
            }
            KeyCode::Char(' ')
                if key.modifiers.is_empty() && self.parallel_mode_prompt_input_locked() =>
            {
                let snapshot = self.parallel_mode_supervisor_snapshot();
                self.supersession_mud_ui_state.inspect_focused(&snapshot);
            }
            _ => return false,
        }

        true
    }
}

#[cfg(test)]
mod orchestrator_retry_tests {
    use super::*;
    use crate::domain::parallel_mode::{
        ParallelModeDistributorQueueItem, ParallelModeOrchestratorStatus,
        ParallelModeQueueItemState,
    };

    #[test]
    fn distributor_tick_signature_changes_when_integration_worktree_recovers() {
        let blocked = supervisor_with_distributor_readiness(
            "blocked: expected `prerelease` but checked out `feature`",
        );
        let ready = supervisor_with_distributor_readiness("ready: prerelease worktree clean");

        let blocked_signature = parallel_mode_distributor_tick_signature(&blocked)
            .expect("active queue should produce retry signature");
        let ready_signature = parallel_mode_distributor_tick_signature(&ready)
            .expect("active queue should produce retry signature");

        assert_ne!(
            blocked_signature, ready_signature,
            "integration readiness must be part of the retry signature so a fixed worktree retries the same queued head"
        );
        assert!(ParallelPanelStateController::snapshot_has_active_distributor_queue(&ready));
    }

    #[test]
    fn distributor_tick_signature_ignores_idle_distributor() {
        let snapshot = ParallelModeSupervisorSnapshot::new(
            ParallelModeSupervisorState::Supervise,
            "/tmp/workspace",
            ParallelModePoolBoardSnapshot::new(3, "/tmp/pool", "idle", Vec::new()),
            ParallelModeAgentRosterSnapshot::new(Vec::new(), "no active agents"),
            ParallelModeSupervisorDetailSnapshot::new(None, "no detail"),
            ParallelModeDistributorSnapshot::new(Vec::new(), Vec::new(), "idle", "queue idle"),
            None,
        );

        assert!(parallel_mode_distributor_tick_signature(&snapshot).is_none());
        assert!(!ParallelPanelStateController::snapshot_has_active_distributor_queue(&snapshot));
    }

    fn supervisor_with_distributor_readiness(readiness: &str) -> ParallelModeSupervisorSnapshot {
        let distributor = ParallelModeDistributorSnapshot::new(
            vec![ParallelModeDistributorQueueItem::new(
                "agent-1",
                "Task One",
                ParallelModeQueueItemState::Queued,
                "akra-agent/slot-1/task-one",
                "abc1234",
                "commit-ready result accepted into distributor queue",
            )],
            Vec::new(),
            "queued",
            "commit-ready result accepted into distributor queue",
        )
        .with_orchestrator_status(ParallelModeOrchestratorStatus {
            queue_head: "agent-1 / task-1 / queued".to_string(),
            barrier_state: "head queued".to_string(),
            blocked_reason: None,
            conflict_files: Vec::new(),
            held_queue_count: 0,
            integration_worktree_readiness: readiness.to_string(),
            slot_return_wait_reason: None,
        });

        ParallelModeSupervisorSnapshot::new(
            ParallelModeSupervisorState::Supervise,
            "/tmp/workspace",
            ParallelModePoolBoardSnapshot::new(3, "/tmp/pool", "idle", Vec::new()),
            ParallelModeAgentRosterSnapshot::new(Vec::new(), "no active agents"),
            ParallelModeSupervisorDetailSnapshot::new(None, "no detail"),
            distributor,
            None,
        )
    }
}
