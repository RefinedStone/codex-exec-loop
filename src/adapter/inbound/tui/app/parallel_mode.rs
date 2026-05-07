use chrono::Utc;
use crossterm::event::{self, KeyCode, KeyModifiers};
use std::sync::Arc;
use std::sync::mpsc::Sender;
use std::thread;

use crate::adapter::inbound::tui::shell_chrome::{ShellChromeEvent, ShellOverlay};
use crate::application::port::outbound::parallel_agent_worker_port::ParallelAgentWorkerPort;
use crate::application::service::parallel_agent_persona::{
    ParallelAgentPersona, load_parallel_agent_persona_config,
};
use crate::application::service::parallel_mode::turn::ParallelModeTurnService;
use crate::application::service::parallel_mode::{
    ParallelModeOrchestratorTrigger, ParallelModeService,
};
use crate::application::service::planning::{PlanningRuntimeSnapshot, PlanningServices};
use crate::diagnostics::event_log;
use crate::domain::parallel_mode::{
    ParallelModeAgentRosterSnapshot, ParallelModeAutomationTrigger,
    ParallelModeDispatchCommandSnapshot, ParallelModeDispatchOutcome,
    ParallelModeDistributorSnapshot, ParallelModeOrchestratorStateMachine,
    ParallelModePoolBoardSnapshot, ParallelModePoolResetScope, ParallelModePostTurnQueueSignal,
    ParallelModeReadinessSnapshot, ParallelModeRuntimeEvent, ParallelModeSupervisorDetailSnapshot,
    ParallelModeSupervisorSnapshot, ParallelModeSupervisorState,
};

/*
 * parallel_mode.rs is the TUI adapter for the supersession control tower. The
 * application service owns pool/readiness/lease rules; this file decides when
 * shell commands should refresh snapshots, show overlay chrome, publish status
 * copy, and spawn background workers for dispatchable planning queue tasks.
 */
#[path = "parallel_mode/dispatch_worker.rs"]
mod dispatch_worker;

use self::dispatch_worker::{ParallelDispatchWorkerRequest, spawn_parallel_dispatch_worker};
use super::turn_submission_runtime::parallel_mode_slot_lease_request;
use super::{
    AutoFollowSkipReason, BackgroundMessage, ConversationInputEvent, ConversationRuntimeEffect,
    ConversationRuntimeEvent, ConversationState, NativeTuiApp,
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
        if self.shell_overlay != ShellOverlay::Supersession || !self.parallel_mode_enabled() {
            return false;
        }

        let Some(snapshot) = self.parallel_mode_supervisor_snapshot.as_ref() else {
            return true;
        };

        parallel_mode_supervisor_snapshot_is_loading(snapshot)
            || parallel_mode_supervisor_snapshot_has_running_slot(snapshot)
            || parallel_mode_supervisor_snapshot_has_active_distributor_queue(snapshot)
            || parallel_mode_supervisor_snapshot_has_recoverable_pool_issue(snapshot)
    }

    pub(crate) fn parallel_mode_loading_prompt_indicator_visible(&self) -> bool {
        if self.shell_overlay != ShellOverlay::Supersession || !self.parallel_mode_enabled() {
            return false;
        }

        let Some(snapshot) = self.parallel_mode_supervisor_snapshot.as_ref() else {
            return true;
        };

        parallel_mode_supervisor_snapshot_is_loading(snapshot)
    }

    pub(crate) fn parallel_mode_prompt_input_locked(&self) -> bool {
        self.shell_overlay == ShellOverlay::Supersession
            && self.parallel_mode_loading_prompt_indicator_visible()
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
        if self.parallel_mode_supervisor_refresh_in_flight {
            return;
        }
        self.spawn_parallel_mode_supervisor_snapshot_refresh(
            workspace_directory,
            readiness_snapshot,
        );
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
        match argument {
            Some(value) if value.eq_ignore_ascii_case("off") => {
                // Turning off parallel mode is local UI state. Keep the snapshot
                // read-only and close the control tower so normal shell focus
                // resumes immediately.
                self.close_parallel_mode_automation_epoch();
                self.parallel_mode_enabled = false;
                self.parallel_mode_supervisor_refresh_in_flight = false;
                self.parallel_mode_dispatch_refresh_in_flight = false;
                self.parallel_mode_orchestrator_tick_in_flight = false;
                self.last_parallel_mode_orchestrator_tick_signature = None;
                self.pending_parallel_mode_dispatch_trigger = None;
                self.sync_parallel_mode_supervisor_snapshot(false);
                if self.shell_overlay == ShellOverlay::Supersession {
                    self.close_shell_overlay();
                }
                self.dispatch_conversation_input(ConversationInputEvent::StatusMessageShown {
                    status_text: "parallel mode: off / shell returned to normal mode".to_string(),
                });
            }
            Some(value) => {
                // Unsupported arguments still open the control tower. That makes
                // the supported commands and current readiness visible next to
                // the error copy.
                self.inspect_parallel_mode_shell();
                self.dispatch_conversation_input(ConversationInputEvent::StatusMessageShown {
                    status_text: format!(
                        "parallel mode command: unsupported argument `{value}` / supported: :parallel, :parallel off"
                    ),
                });
            }
            None => {
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
                self.parallel_mode_supervisor_refresh_in_flight = false;
                self.last_parallel_mode_orchestrator_tick_signature = None;
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
        &mut self,
        workspace_directory: String,
        readiness_snapshot: ParallelModeReadinessSnapshot,
    ) {
        let parallel_mode_service = self.parallel_mode_service.clone();
        let mode_enabled = self.parallel_mode_enabled();
        let tx = self.tx.clone();

        self.parallel_mode_supervisor_refresh_in_flight = true;
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
                supervisor_snapshot: Box::new(supervisor_snapshot),
            });
        });
    }

    fn maybe_spawn_parallel_mode_orchestrator_tick(&mut self, workspace_directory: &str) {
        if !self.parallel_mode_enabled()
            || self.planning_workspace_directory() != workspace_directory
            || self.parallel_mode_orchestrator_tick_in_flight
            || self.parallel_mode_supervisor_refresh_in_flight
            || self.parallel_mode_dispatch_refresh_in_flight
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
        if self
            .last_parallel_mode_orchestrator_tick_signature
            .as_deref()
            == Some(signature.as_str())
        {
            return;
        }

        self.last_parallel_mode_orchestrator_tick_signature = Some(signature.clone());
        self.parallel_mode_orchestrator_tick_in_flight = true;
        self.spawn_parallel_mode_orchestrator_tick_worker(
            workspace_directory.to_string(),
            signature,
        );
    }

    fn spawn_parallel_mode_orchestrator_tick_worker(
        &self,
        workspace_directory: String,
        signature: String,
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

        if self.parallel_mode_automation_epoch_id.is_none() {
            let reason = format!(
                "task update `{task_id}` accepted before the first main-session post-turn epoch"
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
        self.request_parallel_mode_dispatch(
            workspace_directory,
            ParallelModeAutomationTrigger::TaskIntakeAfterEpoch,
        );
    }

    fn parallel_mode_dispatch_refresh_should_defer(&self) -> bool {
        if self.parallel_mode_readiness_snapshot.is_none() {
            return true;
        }

        match self.parallel_mode_supervisor_snapshot.as_ref() {
            Some(snapshot) => parallel_mode_supervisor_snapshot_is_loading(snapshot),
            None => true,
        }
    }

    pub(super) fn parallel_mode_post_turn_queue_signal(
        &self,
        event: &ConversationRuntimeEvent,
    ) -> Option<ParallelModePostTurnQueueSignal> {
        let ConversationRuntimeEvent::PostTurnEvaluated { evaluation } = event else {
            return None;
        };
        match &evaluation.action {
            super::conversation_runtime::ConversationPostTurnAction::SkipAutoFollow {
                reason: AutoFollowSkipReason::ParallelSessionCompleted,
            } => Some(ParallelModePostTurnQueueSignal::ParallelCompletionFinalized),
            _ => None,
        }
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
        self.request_parallel_mode_dispatch(workspace_directory, trigger);
    }

    fn open_parallel_mode_automation_epoch(&mut self) -> u64 {
        if let Some(epoch_id) = self.parallel_mode_automation_epoch_id {
            return epoch_id;
        }

        let epoch_id = self.next_parallel_mode_automation_epoch_id;
        self.next_parallel_mode_automation_epoch_id = self
            .next_parallel_mode_automation_epoch_id
            .saturating_add(1);
        self.parallel_mode_automation_epoch_id = Some(epoch_id);
        self.last_parallel_mode_dispatch_withheld_reason = None;
        event_log::emit_lazy("parallel_automation_epoch_opened", || {
            serde_json::json!({
                "workspace": self.planning_workspace_directory(),
                "epoch_id": epoch_id,
            })
        });
        epoch_id
    }

    fn close_parallel_mode_automation_epoch(&mut self) {
        let workspace_directory = self.planning_workspace_directory();
        let _ = self
            .parallel_mode_service
            .cancel_dispatch_commands(&workspace_directory, "parallel mode disabled");
        let epoch_id = self.parallel_mode_automation_epoch_id.take();
        self.pending_parallel_mode_dispatch_trigger = None;
        self.last_parallel_mode_dispatch_withheld_reason = None;
        if let Some(epoch_id) = epoch_id {
            event_log::emit_lazy("parallel_automation_epoch_closed", || {
                serde_json::json!({
                    "workspace": self.planning_workspace_directory(),
                    "epoch_id": epoch_id,
                })
            });
        }
    }

    fn request_parallel_mode_dispatch(
        &mut self,
        workspace_directory: String,
        trigger: ParallelModeAutomationTrigger,
    ) {
        let Some(epoch_id) = self.parallel_mode_automation_epoch_id else {
            self.record_parallel_mode_dispatch_withheld(
                Some(trigger),
                "automation epoch is not open",
            );
            return;
        };

        if self.parallel_mode_dispatch_refresh_should_defer() {
            self.pending_parallel_mode_dispatch_trigger = Some(trigger);
            self.record_parallel_mode_dispatch_withheld(
                Some(trigger),
                "entry loading or supervisor refresh is still in progress",
            );
            self.dispatch_conversation_input(ConversationInputEvent::StatusMessageShown {
                status_text:
                    "parallel mode: dispatch deferred until control-plane refresh finishes"
                        .to_string(),
            });
            return;
        }

        if self.parallel_mode_dispatch_refresh_in_flight {
            self.pending_parallel_mode_dispatch_trigger = Some(trigger);
            self.record_parallel_mode_dispatch_withheld(
                Some(trigger),
                "dispatch already in flight",
            );
            return;
        }

        self.parallel_mode_dispatch_refresh_in_flight = true;
        self.last_parallel_mode_automation_trigger = Some(trigger);
        self.last_parallel_mode_dispatch_withheld_reason = None;
        event_log::emit_lazy("parallel_dispatch_requested", || {
            serde_json::json!({
                "trigger": trigger.label(),
                "workspace": &workspace_directory,
                "epoch_id": epoch_id,
            })
        });
        self.spawn_parallel_mode_dispatch_refresh_worker(
            workspace_directory,
            trigger,
            epoch_id,
            Some(trigger),
        );
    }

    pub(super) fn apply_parallel_mode_dispatch_request(
        &mut self,
        workspace_directory: String,
        trigger: ParallelModeAutomationTrigger,
        epoch_id: u64,
    ) {
        if self.parallel_mode_automation_epoch_id != Some(epoch_id) {
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
        self.request_parallel_mode_dispatch(workspace_directory, trigger);
    }

    fn spawn_pending_parallel_mode_dispatch_if_ready(&mut self) {
        let Some(trigger) = self.pending_parallel_mode_dispatch_trigger.take() else {
            return;
        };
        if !self.parallel_mode_enabled() || self.parallel_mode_automation_epoch_id.is_none() {
            return;
        }
        let workspace_directory = self.planning_workspace_directory();
        self.request_parallel_mode_dispatch(workspace_directory, trigger);
    }

    pub(super) fn maybe_spawn_parallel_mode_pending_dispatch_command(&mut self) -> bool {
        if !self.parallel_mode_enabled() || self.parallel_mode_automation_epoch_id.is_none() {
            return false;
        }
        if self.parallel_mode_dispatch_refresh_should_defer()
            || self.parallel_mode_dispatch_refresh_in_flight
        {
            return false;
        }
        let workspace_directory = self.planning_workspace_directory();
        match self
            .parallel_mode_service
            .pending_dispatch_command_count(&workspace_directory)
        {
            Ok(0) => false,
            Ok(_) => {
                let epoch_id = self
                    .parallel_mode_automation_epoch_id
                    .expect("checked automation epoch should exist");
                self.parallel_mode_dispatch_refresh_in_flight = true;
                self.last_parallel_mode_automation_trigger =
                    Some(ParallelModeAutomationTrigger::TaskIntakeAfterEpoch);
                self.last_parallel_mode_dispatch_withheld_reason = None;
                self.spawn_parallel_mode_dispatch_refresh_worker(
                    workspace_directory,
                    ParallelModeAutomationTrigger::TaskIntakeAfterEpoch,
                    epoch_id,
                    None,
                );
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
                "epoch_id": self.parallel_mode_automation_epoch_id,
                "blocked_reason": reason,
            })
        });
    }

    fn spawn_parallel_mode_dispatch_refresh_worker(
        &self,
        workspace_directory: String,
        trigger: ParallelModeAutomationTrigger,
        epoch_id: u64,
        enqueue_trigger: Option<ParallelModeAutomationTrigger>,
    ) {
        let parallel_mode_service = self.parallel_mode_service.clone();
        let parallel_agent_worker_port = self.parallel_agent_worker_port.clone();
        let parallel_mode_turn_service = self.parallel_mode_turn_service();
        let planning = self.planning.clone();
        let tx = self.tx.clone();

        thread::spawn(move || {
            let planning_snapshot = planning
                .runtime
                .load_runtime_snapshot_or_invalid(&workspace_directory);
            let readiness_snapshot =
                parallel_mode_service.inspect_readiness(&workspace_directory, &planning_snapshot);

            let (supervisor_snapshot, outcome) = if readiness_snapshot.allows_parallel_mode() {
                if let Some(enqueue_trigger) = enqueue_trigger {
                    let runtime_event =
                        parallel_runtime_event_for_dispatch_trigger(enqueue_trigger);
                    if let Err(error) = parallel_mode_service.enqueue_dispatch_commands_for_event(
                        &workspace_directory,
                        runtime_event,
                        &planning_snapshot,
                        Some(epoch_id),
                    ) {
                        event_log::emit_lazy("parallel_dispatch_command_enqueue_failed", || {
                            serde_json::json!({
                                "trigger": enqueue_trigger.label(),
                                "workspace": &workspace_directory,
                                "epoch_id": epoch_id,
                                "error": error,
                            })
                        });
                    }
                }
                let outcome = match parallel_mode_service
                    .claim_next_dispatch_command(&workspace_directory)
                {
                    Ok(Some(mut command)) => {
                        let outcome =
                            dispatch_parallel_queue_pool(ParallelModeDispatchExecutionContext {
                                workspace_directory: &workspace_directory,
                                planning_snapshot: &planning_snapshot,
                                parallel_mode_service: &parallel_mode_service,
                                parallel_agent_worker_port,
                                parallel_mode_turn_service,
                                planning,
                                tx: tx.clone(),
                                trigger: command.trigger,
                                epoch_id,
                            });
                        persist_dispatch_command_outcome(
                            &parallel_mode_service,
                            &workspace_directory,
                            &mut command,
                            &outcome,
                        );
                        outcome
                    }
                    Ok(None) => {
                        let mut outcome = ParallelModeDispatchOutcome::new(
                            trigger,
                            workspace_directory.clone(),
                            epoch_id,
                        );
                        outcome.blocked_reason =
                            Some("no pending durable dispatch command".to_string());
                        outcome.status_copy_input = outcome.status_detail();
                        outcome
                    }
                    Err(error) => {
                        let mut outcome = ParallelModeDispatchOutcome::new(
                            trigger,
                            workspace_directory.clone(),
                            epoch_id,
                        );
                        outcome.blocked_reason =
                            Some(format!("dispatch command claim failed: {error}"));
                        outcome.status_copy_input = outcome.status_detail();
                        outcome
                    }
                };
                let supervisor_snapshot = parallel_mode_service.build_supervisor_snapshot(
                    &workspace_directory,
                    true,
                    Some(&readiness_snapshot),
                );
                (supervisor_snapshot, outcome)
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
                let mut outcome = ParallelModeDispatchOutcome::new(
                    trigger,
                    workspace_directory.clone(),
                    epoch_id,
                );
                outcome.blocked_reason = Some(format!(
                    "readiness: {} / {cause}",
                    readiness_snapshot.readiness_label()
                ));
                outcome.status_copy_input = outcome.blocked_reason.clone().unwrap_or_default();
                (supervisor_snapshot, outcome)
            };

            let _ = tx.send(BackgroundMessage::ParallelModeDispatchRefreshed {
                workspace_directory,
                readiness_snapshot,
                supervisor_snapshot: Box::new(supervisor_snapshot),
                outcome,
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
            self.parallel_mode_supervisor_refresh_in_flight = false;
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
            self.parallel_mode_supervisor_refresh_in_flight = false;
            return;
        }

        self.parallel_mode_supervisor_refresh_in_flight = false;
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
            self.spawn_pending_parallel_mode_dispatch_if_ready();
            self.maybe_spawn_parallel_mode_orchestrator_tick(workspace_directory);
        } else {
            self.pending_parallel_mode_dispatch_trigger = None;
        }
    }

    pub(super) fn apply_parallel_mode_supervisor_snapshot_refreshed(
        &mut self,
        workspace_directory: &str,
        supervisor_snapshot: ParallelModeSupervisorSnapshot,
    ) {
        self.parallel_mode_supervisor_refresh_in_flight = false;
        if !self.parallel_mode_enabled()
            || self.planning_workspace_directory() != workspace_directory
        {
            return;
        }

        self.parallel_mode_supervisor_snapshot = Some(supervisor_snapshot);
        self.maybe_spawn_parallel_mode_orchestrator_tick(workspace_directory);
    }

    pub(super) fn apply_parallel_mode_dispatch_refreshed(
        &mut self,
        workspace_directory: &str,
        readiness_snapshot: ParallelModeReadinessSnapshot,
        supervisor_snapshot: ParallelModeSupervisorSnapshot,
        outcome: ParallelModeDispatchOutcome,
    ) {
        if !self.parallel_mode_enabled()
            || self.planning_workspace_directory() != workspace_directory
            || self.parallel_mode_automation_epoch_id != Some(outcome.epoch_id)
        {
            self.parallel_mode_dispatch_refresh_in_flight = false;
            return;
        }

        self.parallel_mode_dispatch_refresh_in_flight = false;
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
        self.spawn_pending_parallel_mode_dispatch_if_ready();
        self.maybe_spawn_parallel_mode_orchestrator_tick(workspace_directory);
    }

    pub(super) fn apply_parallel_mode_orchestrator_tick_completed(
        &mut self,
        workspace_directory: &str,
        blocked: bool,
        notices: Vec<String>,
    ) {
        self.parallel_mode_orchestrator_tick_in_flight = false;
        if !self.parallel_mode_enabled()
            || self.planning_workspace_directory() != workspace_directory
        {
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
        let status_text = if blocked {
            format!("parallel mode: distributor retry blocked / notices: {notice_count}")
        } else {
            format!("parallel mode: distributor retry completed / notices: {notice_count}")
        };
        self.dispatch_conversation_input(ConversationInputEvent::StatusMessageShown {
            status_text,
        });
    }
}

fn parallel_mode_supervisor_snapshot_is_loading(snapshot: &ParallelModeSupervisorSnapshot) -> bool {
    snapshot
        .top_notice
        .as_deref()
        .is_some_and(|notice| notice.starts_with("loading "))
        || snapshot.pool.pool_root_label.starts_with("loading:")
}

fn parallel_mode_supervisor_snapshot_has_running_slot(
    snapshot: &ParallelModeSupervisorSnapshot,
) -> bool {
    snapshot.pool.running_slots > 0
}

fn parallel_mode_supervisor_snapshot_has_active_distributor_queue(
    snapshot: &ParallelModeSupervisorSnapshot,
) -> bool {
    !snapshot.distributor.queue_items.is_empty()
}

fn parallel_mode_supervisor_snapshot_has_recoverable_pool_issue(
    snapshot: &ParallelModeSupervisorSnapshot,
) -> bool {
    snapshot.pool.blocked_slots > 0
        || snapshot.pool.missing_slots > 0
        || snapshot.pool.unavailable_slots > 0
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

struct ParallelModeDispatchExecutionContext<'a> {
    workspace_directory: &'a str,
    planning_snapshot: &'a PlanningRuntimeSnapshot,
    parallel_mode_service: &'a ParallelModeService,
    parallel_agent_worker_port: Arc<dyn ParallelAgentWorkerPort>,
    parallel_mode_turn_service: ParallelModeTurnService,
    planning: PlanningServices,
    tx: Sender<BackgroundMessage>,
    trigger: ParallelModeAutomationTrigger,
    epoch_id: u64,
}

fn dispatch_parallel_queue_pool(
    context: ParallelModeDispatchExecutionContext<'_>,
) -> ParallelModeDispatchOutcome {
    /*
     * Dispatch is the handoff bridge from planning queue to parallel worker.
     * The service chooses candidates and leases slots; the TUI assembles the
     * sub-session prompt, starts a background worker, and reports a compact
     * launch summary back through conversation status.
     */

    let workspace_directory = context.workspace_directory;
    let trigger = context.trigger;
    let epoch_id = context.epoch_id;
    let mut outcome =
        ParallelModeDispatchOutcome::new(trigger, workspace_directory.to_string(), epoch_id);

    let dispatch_plan = match context.parallel_mode_service.build_dispatch_plan(
        workspace_directory,
        context.planning_snapshot,
        // A task-update dispatch refresh handles the currently actionable queue;
        // the service still limits work by idle slots and candidate rules.
        usize::MAX,
    ) {
        Ok(plan) => plan,
        Err(error) => {
            outcome.blocked_reason = Some(error);
            outcome.status_copy_input = outcome.status_detail();
            event_log::emit_lazy("parallel_dispatch_blocked", || {
                serde_json::json!({
                    "trigger": trigger.label(),
                    "workspace": workspace_directory,
                    "epoch_id": epoch_id,
                    "blocked_reason": outcome.blocked_reason,
                })
            });
            return outcome;
        }
    };
    outcome.idle_slot_count = dispatch_plan.idle_slot_count;
    outcome.candidate_task_ids = dispatch_plan
        .candidates
        .iter()
        .map(|task| task.task_id.clone())
        .collect();
    event_log::emit_lazy("parallel_dispatch_plan_built", || {
        serde_json::json!({
            "trigger": trigger.label(),
            "workspace": workspace_directory,
            "epoch_id": epoch_id,
            "idle_slot_count": dispatch_plan.idle_slot_count,
            "candidate_task_ids": &outcome.candidate_task_ids,
            "excluded_task_ids": &dispatch_plan.excluded_task_ids,
        })
    });
    // Distinguish infrastructure capacity from queue availability so the
    // operator can decide whether to wait for slots or change planning tasks.
    if dispatch_plan.idle_slot_count == 0 {
        outcome.blocked_reason = Some("no idle slot is available for auto dispatch".to_string());
        outcome.status_copy_input = outcome.status_detail();
        event_log::emit_lazy("parallel_dispatch_blocked", || {
            serde_json::json!({
                "trigger": trigger.label(),
                "workspace": workspace_directory,
                "epoch_id": epoch_id,
                "idle_slot_count": outcome.idle_slot_count,
                "task_ids": outcome.candidate_task_ids,
                "blocked_reason": outcome.blocked_reason,
            })
        });
        return outcome;
    }
    if dispatch_plan.candidates.is_empty() {
        let reason = if dispatch_plan.excluded_task_ids.is_empty() {
            "no actionable queue task to auto dispatch".to_string()
        } else {
            format!(
                "no undispatched queue task available for auto dispatch / excluded: {}",
                dispatch_plan.excluded_task_ids.join(", ")
            )
        };
        outcome.blocked_reason = Some(reason);
        outcome.status_copy_input = outcome.status_detail();
        event_log::emit_lazy("parallel_dispatch_blocked", || {
            serde_json::json!({
                "trigger": trigger.label(),
                "workspace": workspace_directory,
                "epoch_id": epoch_id,
                "idle_slot_count": outcome.idle_slot_count,
                "task_ids": outcome.candidate_task_ids,
                "blocked_reason": outcome.blocked_reason,
            })
        });
        return outcome;
    }

    let mut launched_titles = Vec::new();
    let mut blocked_details = Vec::new();
    let persona = load_parallel_agent_persona_config(workspace_directory)
        .map(|config| config.persona)
        .unwrap_or(ParallelAgentPersona::None);
    for task in dispatch_plan.candidates {
        // Handoff creation belongs to planning runtime because it knows how
        // to turn a queue task into sub-session prompt text and task identity.
        let handoff = context
            .planning
            .runtime
            .build_sub_session_task_handoff_with_persona(&task, persona);
        let lease_request = parallel_mode_slot_lease_request(&handoff.task);
        match context
            .parallel_mode_service
            .acquire_slot_lease(workspace_directory, lease_request)
        {
            Ok(lease) => {
                event_log::emit_lazy("parallel_dispatch_slot_lease_acquired", || {
                    serde_json::json!({
                        "trigger": trigger.label(),
                        "workspace": workspace_directory,
                        "epoch_id": epoch_id,
                        "slot_id": &lease.slot_id,
                        "agent_id": &lease.agent_id,
                        "task_id": &handoff.task.task_id,
                        "task_title": &handoff.task.task_title,
                        "worktree": &lease.worktree_path,
                        "service_name": &handoff.service_name,
                        "prompt_chars": handoff.prompt.chars().count(),
                        "developer_instructions_chars": handoff.developer_instructions.chars().count(),
                    })
                });
                // After the lease is acquired, the worker owns app-server
                // turn execution in the slot worktree. The TUI keeps only
                // status copy and receives later updates over its channel.
                let worker_request = ParallelDispatchWorkerRequest {
                    planning_workspace_directory: workspace_directory.to_string(),
                    worktree_directory: lease.worktree_path.clone(),
                    automation_epoch_id: epoch_id,
                    prompt: handoff.prompt,
                    developer_instructions: handoff.developer_instructions,
                    service_name: handoff.service_name,
                    handoff_task: handoff.task.clone(),
                };
                spawn_parallel_dispatch_worker(
                    worker_request,
                    context.parallel_agent_worker_port.clone(),
                    context.parallel_mode_turn_service.clone(),
                    context.planning.clone(),
                    context.tx.clone(),
                );
                outcome.launched_task_ids.push(handoff.task.task_id.clone());
                launched_titles.push(handoff.task.task_title);
            }
            Err(error) => blocked_details.push(format!("{}: {error}", handoff.task.task_id)),
        }
    }
    let launched_count = launched_titles.len();
    if launched_count == 0 {
        outcome.blocked_reason = Some(format!(
            "worker launch blocked / {}",
            blocked_details.join(" | ")
        ));
        outcome.status_copy_input = outcome.status_detail();
        event_log::emit_lazy("parallel_dispatch_blocked", || {
            serde_json::json!({
                "trigger": trigger.label(),
                "workspace": workspace_directory,
                "epoch_id": epoch_id,
                "idle_slot_count": outcome.idle_slot_count,
                "task_ids": outcome.candidate_task_ids,
                "blocked_reason": outcome.blocked_reason,
            })
        });
        return outcome;
    }

    let mut status = format!(
        "auto dispatched {launched_count} worker(s) / tasks: {}",
        launched_titles.join(" | ")
    );
    if !blocked_details.is_empty() {
        status.push_str(&format!(" / blocked: {}", blocked_details.join(" | ")));
    }
    outcome.status_copy_input = status;
    event_log::emit_lazy("parallel_dispatch_launched", || {
        serde_json::json!({
            "trigger": trigger.label(),
            "workspace": workspace_directory,
            "epoch_id": epoch_id,
            "idle_slot_count": outcome.idle_slot_count,
            "task_ids": outcome.candidate_task_ids,
            "launched_count": outcome.launched_task_ids.len(),
        })
    });
    outcome
}

fn parallel_runtime_event_for_dispatch_trigger(
    trigger: ParallelModeAutomationTrigger,
) -> ParallelModeRuntimeEvent {
    match trigger {
        ParallelModeAutomationTrigger::MainTurnPostEvaluation => {
            ParallelModeRuntimeEvent::AutoFollowQueued
        }
        ParallelModeAutomationTrigger::ParallelOfficialCompletion => {
            ParallelModeRuntimeEvent::ParallelCompletionFinalized
        }
        ParallelModeAutomationTrigger::TaskIntakeAfterEpoch => {
            ParallelModeRuntimeEvent::TaskIntakeCommitted
        }
    }
}

fn persist_dispatch_command_outcome(
    parallel_mode_service: &ParallelModeService,
    workspace_directory: &str,
    command: &mut ParallelModeDispatchCommandSnapshot,
    outcome: &ParallelModeDispatchOutcome,
) {
    let timestamp = Utc::now().to_rfc3339();
    if outcome.blocked_reason.is_some() && outcome.launched_task_ids.is_empty() {
        command.mark_blocked(outcome.status_detail(), timestamp);
    } else {
        command.mark_completed(outcome.status_detail(), timestamp);
    }
    if let Err(error) = parallel_mode_service.update_dispatch_command(workspace_directory, command)
    {
        event_log::emit_lazy("parallel_dispatch_command_update_failed", || {
            serde_json::json!({
                "workspace": workspace_directory,
                "command_id": &command.command_id,
                "state": command.state.label(),
                "error": error,
            })
        });
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
        assert!(parallel_mode_supervisor_snapshot_has_active_distributor_queue(&ready));
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
        assert!(!parallel_mode_supervisor_snapshot_has_active_distributor_queue(&snapshot));
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
