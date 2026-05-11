use crossterm::event::{self, KeyCode, KeyModifiers};
use std::time::Instant;

use crate::adapter::inbound::tui::shell_chrome::{ShellChromeEvent, ShellOverlay};
#[cfg(test)]
use crate::application::service::parallel_mode::control_plane::ParallelModeControlPlaneEffectId;
#[cfg(test)]
use crate::application::service::parallel_mode::control_plane::parallel_mode_distributor_tick_signature;
use crate::application::service::parallel_mode::control_plane::{
    ParallelModeControlPlaneBackgroundEvent, ParallelModeControlPlaneCommand,
    ParallelModeControlPlaneLoadingStage, ParallelModeControlPlanePresentationEvent,
};
use crate::core::app::CoreInput;
use crate::diagnostics::event_log;
use crate::domain::parallel_mode::{
    ParallelModeAgentRosterSnapshot, ParallelModeAutomationTrigger,
    ParallelModeDistributorSnapshot, ParallelModePoolBoardSnapshot,
    ParallelModePostTurnQueueSignal, ParallelModeReadinessSnapshot,
    ParallelModeSupervisorDetailSnapshot, ParallelModeSupervisorSnapshot,
    ParallelModeSupervisorState,
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
    ConversationInputEvent, ConversationRuntimeEvent, NativeTuiApp, ParallelPanelStateController,
    ParallelPanelUiEvent, ParallelPanelUiState,
};

impl NativeTuiApp {
    pub(super) fn apply_parallel_mode_control_plane_background_event(
        &mut self,
        event: ParallelModeControlPlaneBackgroundEvent,
    ) {
        let events = self
            .parallel_mode_control_plane
            .handle_background_event(event);
        self.apply_parallel_mode_control_plane_presentation_events(events);
    }

    fn apply_parallel_mode_control_plane_presentation_events(
        &mut self,
        events: Vec<ParallelModeControlPlanePresentationEvent>,
    ) -> bool {
        let changed = !events.is_empty();
        for event in events {
            self.apply_parallel_mode_control_plane_presentation_event(event);
        }
        changed
    }

    fn apply_parallel_mode_control_plane_presentation_event(
        &mut self,
        event: ParallelModeControlPlanePresentationEvent,
    ) {
        match event {
            ParallelModeControlPlanePresentationEvent::EnterProgress {
                workspace_directory,
                readiness_snapshot,
                loading_stage,
                status_text,
            } => {
                let stage = match loading_stage {
                    ParallelModeControlPlaneLoadingStage::ReconcilingPool => {
                        ParallelModeLoadingStage::ReconcilingPool
                    }
                };
                let supervisor_snapshot = pending_parallel_mode_supervisor_snapshot(
                    &workspace_directory,
                    true,
                    readiness_snapshot.as_ref(),
                    stage,
                );
                self.apply_parallel_mode_enter_progress(
                    &workspace_directory,
                    readiness_snapshot,
                    supervisor_snapshot,
                    status_text,
                );
            }
            ParallelModeControlPlanePresentationEvent::ReadinessSnapshotChanged {
                workspace_directory,
                snapshot,
            } => {
                if self.planning_workspace_directory() == workspace_directory {
                    self.sync_core_parallel_mode_readiness_projection(Some(snapshot));
                }
            }
            ParallelModeControlPlanePresentationEvent::SupervisorSnapshotChanged {
                workspace_directory,
                snapshot,
            } => {
                if self.planning_workspace_directory() == workspace_directory {
                    self.sync_core_parallel_mode_supervisor_projection(Some(*snapshot));
                }
            }
            ParallelModeControlPlanePresentationEvent::StatusShown { status_text } => {
                self.dispatch_conversation_input(ConversationInputEvent::StatusMessageShown {
                    status_text,
                });
            }
            ParallelModeControlPlanePresentationEvent::ConversationRuntimeNotice { notice } => {
                self.dispatch_conversation_runtime(
                    ConversationRuntimeEvent::RuntimeNoticeObserved { notice },
                );
            }
            ParallelModeControlPlanePresentationEvent::PostTurnAutoFollowPromptConsumed => {}
            ParallelModeControlPlanePresentationEvent::PlanningRuntimeRefreshRequested {
                workspace_directory,
            } => {
                self.refresh_ready_conversation_planning_runtime_projection_for_workspace(
                    &workspace_directory,
                );
            }
            ParallelModeControlPlanePresentationEvent::ModeDisabled { .. } => {}
        }
    }
}

impl NativeTuiApp {
    fn apply_parallel_mode_control_plane_command(
        &mut self,
        command: ParallelModeControlPlaneCommand,
    ) -> bool {
        let events = self.parallel_mode_control_plane.handle_command(command);
        self.apply_parallel_mode_control_plane_presentation_events(events)
    }

    pub(crate) fn parallel_mode_enabled(&self) -> bool {
        self.parallel_mode_control_plane.mode_enabled()
    }
    pub(crate) fn parallel_mode_readiness_snapshot(&self) -> Option<ParallelModeReadinessSnapshot> {
        self.current_parallel_mode_readiness_projection()
    }
    #[cfg(test)]
    pub(crate) fn parallel_mode_automation_epoch_id(&self) -> Option<u64> {
        let workspace_directory = self.planning_workspace_directory();
        self.parallel_mode_control_plane
            .current_epoch_id_for_workspace(&workspace_directory)
    }
    #[cfg(test)]
    pub(crate) fn parallel_mode_supervisor_refresh_in_flight(&self) -> bool {
        self.parallel_mode_control_plane
            .supervisor_refresh_in_flight()
    }
    #[cfg(test)]
    pub(crate) fn parallel_mode_orchestrator_wake_in_flight(&self) -> bool {
        self.parallel_mode_control_plane
            .orchestrator_wake_in_flight()
    }
    #[cfg(test)]
    pub(crate) fn set_parallel_mode_enabled_for_test(&mut self, enabled: bool) {
        let workspace_directory = self.planning_workspace_directory();
        self.parallel_mode_control_plane
            .force_mode_for_test(workspace_directory, enabled);
    }
    #[cfg(test)]
    pub(crate) fn set_parallel_mode_initial_pool_reset_completed_for_test(
        &mut self,
        completed: bool,
    ) {
        self.parallel_mode_control_plane
            .force_initial_pool_reset_completed_for_test(completed);
    }
    #[cfg(test)]
    pub(crate) fn set_parallel_mode_automation_epoch_for_test(&mut self, epoch_id: u64) {
        let workspace_directory = self.planning_workspace_directory();
        self.parallel_mode_control_plane
            .force_epoch_for_test(workspace_directory, epoch_id);
    }
    #[cfg(test)]
    pub(crate) fn mark_parallel_mode_supervisor_refresh_in_flight_for_test(
        &mut self,
    ) -> (u64, ParallelModeControlPlaneEffectId) {
        let workspace_directory = self.planning_workspace_directory();
        let epoch_id = self.parallel_mode_automation_epoch_id().unwrap_or(1);
        let effect_id = self
            .parallel_mode_control_plane
            .force_supervisor_refresh_in_flight_for_test(workspace_directory, epoch_id);
        (epoch_id, effect_id)
    }
    pub(crate) fn last_parallel_mode_automation_trigger(
        &self,
    ) -> Option<ParallelModeAutomationTrigger> {
        self.parallel_mode_control_plane.last_automation_trigger()
    }
    pub(crate) fn last_parallel_mode_dispatch_withheld_reason(&self) -> Option<String> {
        self.parallel_mode_control_plane
            .last_dispatch_withheld_reason()
    }
    pub(crate) fn parallel_mode_supervisor_snapshot(&self) -> ParallelModeSupervisorSnapshot {
        let workspace_directory = self.planning_workspace_directory();
        if let Some(snapshot) = self.current_parallel_mode_supervisor_projection() {
            return snapshot;
        }

        let readiness_snapshot = self.parallel_mode_readiness_snapshot();
        pending_parallel_mode_supervisor_snapshot(
            &workspace_directory,
            self.parallel_mode_enabled(),
            readiness_snapshot.as_ref(),
            ParallelModeLoadingStage::Entering,
        )
    }

    fn current_parallel_mode_readiness_projection(&self) -> Option<ParallelModeReadinessSnapshot> {
        let workspace_directory = self.planning_workspace_directory();
        self.core_parallel_mode_readiness_snapshot()
            .filter(|snapshot| snapshot.workspace_path == workspace_directory)
            .or_else(|| {
                self.parallel_mode_readiness_snapshot
                    .clone()
                    .filter(|snapshot| snapshot.workspace_path == workspace_directory)
            })
    }

    fn core_parallel_mode_readiness_snapshot(&self) -> Option<ParallelModeReadinessSnapshot> {
        self.core_runtime
            .snapshot()
            .planning_parallel
            .parallel_mode
            .readiness
            .map(|snapshot| *snapshot)
    }

    fn current_parallel_mode_supervisor_projection(
        &self,
    ) -> Option<ParallelModeSupervisorSnapshot> {
        let workspace_directory = self.planning_workspace_directory();
        self.core_parallel_mode_supervisor_snapshot()
            .filter(|snapshot| snapshot.workspace_path == workspace_directory)
            .or_else(|| {
                self.parallel_mode_supervisor_snapshot
                    .clone()
                    .filter(|snapshot| snapshot.workspace_path == workspace_directory)
            })
    }

    fn core_parallel_mode_supervisor_snapshot(&self) -> Option<ParallelModeSupervisorSnapshot> {
        self.core_runtime
            .snapshot()
            .planning_parallel
            .parallel_mode
            .supervisor
            .map(|snapshot| *snapshot)
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
                self.current_parallel_mode_supervisor_projection()
                    .map(Box::new),
            ),
        ];
        if let Some(reason) = self.last_parallel_mode_dispatch_withheld_reason() {
            events.push(ParallelPanelUiEvent::StatusShown(format!(
                "parallel mode: dispatch withheld / {reason}"
            )));
        }
        ParallelPanelStateController::project(events)
    }

    pub(super) fn invalidate_parallel_mode_supervisor_snapshot(&mut self) {
        // Worker dispatch changes leases asynchronously. Keep the last concrete
        // board on screen and refresh a new snapshot off the input/render path.
        let readiness_snapshot = self.parallel_mode_readiness_snapshot();
        let workspace_directory = self.planning_workspace_directory();
        if self.current_parallel_mode_supervisor_projection().is_none() {
            self.sync_core_parallel_mode_supervisor_projection(Some(
                pending_parallel_mode_supervisor_snapshot(
                    &workspace_directory,
                    self.parallel_mode_enabled(),
                    readiness_snapshot.as_ref(),
                    ParallelModeLoadingStage::RefreshingBoard,
                ),
            ));
        }
        self.apply_parallel_mode_control_plane_command(
            ParallelModeControlPlaneCommand::RefreshSupervisor {
                workspace_directory,
            },
        );
    }

    fn inspect_parallel_mode_supervisor(
        &mut self,
        reconcile_pool: bool,
        show_status: bool,
    ) -> bool {
        let workspace_directory = self.planning_workspace_directory();
        self.apply_parallel_mode_control_plane_command(
            ParallelModeControlPlaneCommand::InspectSupervisor {
                workspace_directory,
                reconcile_pool,
                show_status,
            },
        )
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
        self.inspect_parallel_mode_supervisor(false, false);
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
                self.inspect_parallel_mode_supervisor(false, false);
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
                // control tower first, then send one application command. The
                // runtime owns mode and initial-reset policy; this adapter only
                // projects the loading state.
                let workspace_directory = self.planning_workspace_directory();
                self.sync_core_parallel_mode_readiness_projection(None);
                self.sync_core_parallel_mode_supervisor_projection(Some(
                    pending_parallel_mode_supervisor_snapshot(
                        &workspace_directory,
                        true,
                        None,
                        ParallelModeLoadingStage::Entering,
                    ),
                ));
                self.show_supersession_overlay();
                self.apply_parallel_mode_control_plane_command(
                    ParallelModeControlPlaneCommand::Enable {
                        workspace_directory,
                    },
                );
                self.dispatch_conversation_input(ConversationInputEvent::StatusMessageShown {
                    status_text:
                        "parallel mode: loading 1/3 / checking readiness before pool setup"
                            .to_string(),
                });
            }
        }
    }

    pub(super) fn refresh_parallel_mode_dispatch_after_task_update(&mut self, _task_id: &str) {
        if !self.parallel_mode_enabled() {
            return;
        }

        let workspace_directory = self.planning_workspace_directory();
        self.request_parallel_mode_dispatch(
            workspace_directory,
            ParallelModeAutomationTrigger::TaskIntakeAfterEpoch,
            None,
        );
    }

    pub(super) fn parallel_mode_post_turn_queue_signal(
        &self,
        event: &ConversationRuntimeEvent,
    ) -> Option<ParallelModePostTurnQueueSignal> {
        let ConversationRuntimeEvent::PostTurnEvaluationCompleted { evaluation } = event else {
            return None;
        };
        evaluation.provenance.parallel_queue_signal
    }

    pub(super) fn apply_parallel_mode_post_turn_queue_continuation(
        &mut self,
        auto_follow_prompt_queued: bool,
        event_signal: Option<ParallelModePostTurnQueueSignal>,
    ) -> bool {
        let workspace_directory = self.planning_workspace_directory();
        let control_plane = self.parallel_mode_control_plane.clone();
        let outcome = control_plane.continue_post_turn_queue(
            workspace_directory,
            event_signal,
            auto_follow_prompt_queued,
        );
        self.apply_parallel_mode_control_plane_presentation_events(outcome.presentation_events);
        outcome.auto_follow_prompt_consumed
    }

    pub(super) fn close_parallel_mode_automation_epoch(&mut self) {
        let (workspace_directory, epoch_id) = {
            let snapshot = self.parallel_mode_control_plane.epoch_snapshot();
            (
                snapshot
                    .workspace_directory
                    .clone()
                    .unwrap_or_else(|| self.planning_workspace_directory()),
                snapshot.current_epoch_id,
            )
        };
        self.apply_parallel_mode_control_plane_command(ParallelModeControlPlaneCommand::Disable {
            workspace_directory: workspace_directory.clone(),
        });
        self.parallel_mode_control_plane
            .clear_dispatch_withheld_reason();
        if let Some(epoch_id) = epoch_id {
            event_log::emit_lazy("parallel_automation_epoch_closed", || {
                serde_json::json!({
                    "workspace": workspace_directory,
                    "epoch_id": epoch_id,
                })
            });
        }
    }

    #[cfg(test)]
    pub(super) fn apply_parallel_mode_orchestrator_wake_request(
        &mut self,
        workspace_directory: String,
        trigger: ParallelModeAutomationTrigger,
        epoch_id: u64,
    ) {
        self.request_parallel_mode_dispatch(workspace_directory, trigger, Some(epoch_id));
    }

    fn request_parallel_mode_dispatch(
        &mut self,
        workspace_directory: String,
        trigger: ParallelModeAutomationTrigger,
        epoch_id: Option<u64>,
    ) {
        let command = match epoch_id {
            Some(epoch_id) => ParallelModeControlPlaneCommand::RequestDispatchForEpoch {
                workspace_directory,
                trigger,
                epoch_id,
            },
            None => ParallelModeControlPlaneCommand::RequestDispatch {
                workspace_directory,
                trigger,
            },
        };
        self.apply_parallel_mode_control_plane_command(command);
    }

    pub(super) fn tick_parallel_mode_control_plane(&mut self, now: Instant) -> bool {
        let workspace_directory = self.planning_workspace_directory();
        let activity_pulse_visible = self.parallel_mode_activity_pulse_visible();
        let events =
            self.parallel_mode_control_plane
                .tick(now, workspace_directory, activity_pulse_visible);
        self.apply_parallel_mode_control_plane_presentation_events(events)
    }

    #[cfg(test)]
    pub(super) fn parallel_mode_supervisor_refresh_due_for_test(&self, now: Instant) -> bool {
        self.parallel_mode_control_plane
            .supervisor_refresh_due(now, self.parallel_mode_activity_pulse_visible())
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
            self.sync_core_parallel_mode_readiness_projection(Some(readiness_snapshot));
        }
        self.sync_core_parallel_mode_supervisor_projection(Some(supervisor_snapshot));
        self.dispatch_conversation_input(ConversationInputEvent::StatusMessageShown {
            status_text,
        });
    }

    fn sync_core_parallel_mode_readiness_projection(
        &mut self,
        snapshot: Option<ParallelModeReadinessSnapshot>,
    ) {
        self.parallel_mode_readiness_snapshot = snapshot.clone();
        self.dispatch_core_input(CoreInput::ParallelModeReadinessProjectionChanged(
            snapshot.map(Box::new),
        ));
    }

    fn sync_core_parallel_mode_supervisor_projection(
        &mut self,
        snapshot: Option<ParallelModeSupervisorSnapshot>,
    ) {
        self.parallel_mode_supervisor_snapshot = snapshot.clone();
        self.dispatch_core_input(CoreInput::ParallelModeSupervisorProjectionChanged(
            snapshot.map(Box::new),
        ));
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
                self.inspect_parallel_mode_supervisor(true, true);
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
