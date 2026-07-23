use crossterm::event::{self, KeyCode, KeyModifiers};
use std::time::Instant;

use crate::adapter::inbound::tui::shell_chrome::{ShellChromeEvent, ShellOverlay};
#[cfg(test)]
use crate::application::service::parallel_mode::control_plane::ParallelModeControlPlaneEffectId;
#[cfg(test)]
use crate::application::service::parallel_mode::control_plane::parallel_mode_distributor_tick_signature;
use crate::application::service::parallel_mode::control_plane::{
    ParallelModeControlPlaneBackgroundEvent, ParallelModeControlPlaneCommand,
    ParallelModeControlPlanePresentationEvent,
};
use crate::core::app::CoreInput;
use crate::diagnostics::event_log;
use crate::domain::parallel_mode::{
    ParallelModeAutomationTrigger, ParallelModePostTurnQueueSignal, ParallelModeReadinessSnapshot,
    ParallelModeSupervisorSnapshot,
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
use super::parallel_presentation_bridge::{
    ParallelModePresentationAction, ParallelModePresentationBridgeContext,
    ParallelModePresentationLoadingStage, parallel_mode_presentation_actions,
    pending_parallel_mode_supervisor_snapshot,
};
use super::shell_presentation::ParallelPanelProjectionSample;
use super::{
    ConversationInputEvent, ConversationRuntimeEvent, ConversationState, NativeTuiApp,
    ParallelPanelStateController, ParallelPanelUiEvent, ParallelPanelUiState,
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
        let current_workspace_directory = self.planning_workspace_directory();
        let context = ParallelModePresentationBridgeContext::new(
            current_workspace_directory,
            self.parallel_mode_enabled(),
        );
        let actions = parallel_mode_presentation_actions(&context, events);
        let changed = !actions.is_empty();
        for action in actions {
            self.apply_parallel_mode_presentation_action(action);
        }
        changed
    }

    fn apply_parallel_mode_presentation_action(&mut self, action: ParallelModePresentationAction) {
        match action {
            ParallelModePresentationAction::SyncReadinessProjection(snapshot) => {
                self.sync_core_parallel_mode_readiness_projection(Some(snapshot));
            }
            ParallelModePresentationAction::SyncSupervisorProjection(snapshot) => {
                self.sync_core_parallel_mode_supervisor_projection(Some(*snapshot));
            }
            ParallelModePresentationAction::ShowStatus(status_text) => {
                self.dispatch_conversation_input(ConversationInputEvent::StatusMessageShown {
                    status_text,
                });
            }
            ParallelModePresentationAction::ObserveRuntimeNotice(notice) => {
                self.dispatch_client_event(CoreInput::ConversationRuntimeNotice(notice));
            }
            // The control-plane ledger already changed before this invalidation
            // reaches the adapter. Keeping a marker action preserves redraw
            // scheduling without creating a second TUI writer.
            ParallelModePresentationAction::GlobalRuntimeNoticesChanged => {}
            ParallelModePresentationAction::RefreshPlanningRuntimeProjection {
                workspace_directory,
            } => {
                self.refresh_ready_conversation_planning_runtime_projection_for_workspace(
                    &workspace_directory,
                );
            }
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
    pub(crate) fn parallel_mode_control_effect_in_flight(&self) -> bool {
        self.parallel_mode_control_plane.control_effect_in_flight()
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
    #[cfg(test)]
    pub(crate) fn last_parallel_mode_automation_trigger(
        &self,
    ) -> Option<ParallelModeAutomationTrigger> {
        self.parallel_mode_control_plane.last_automation_trigger()
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
            ParallelModePresentationLoadingStage::Entering,
        )
    }

    fn current_parallel_mode_readiness_projection(&self) -> Option<ParallelModeReadinessSnapshot> {
        let workspace_directory = self.planning_workspace_directory();
        self.core_parallel_mode_readiness_snapshot()
            .filter(|snapshot| snapshot.workspace_path == workspace_directory)
    }

    fn core_parallel_mode_readiness_snapshot(&self) -> Option<ParallelModeReadinessSnapshot> {
        self.client_runtime
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
    }

    fn core_parallel_mode_supervisor_snapshot(&self) -> Option<ParallelModeSupervisorSnapshot> {
        self.client_runtime
            .snapshot()
            .planning_parallel
            .parallel_mode
            .supervisor
            .map(|snapshot| *snapshot)
    }

    #[cfg(test)]
    pub(crate) fn parallel_mode_activity_pulse_visible(&self) -> bool {
        let sample = ParallelPanelProjectionSample::capture(self);
        self.parallel_mode_activity_pulse_visible_with_sample(&sample)
    }

    pub(crate) fn parallel_mode_prompt_input_locked(&self) -> bool {
        let sample = ParallelPanelProjectionSample::capture(self);
        self.parallel_mode_prompt_input_locked_with_sample(&sample)
    }

    pub(super) fn parallel_mode_activity_pulse_visible_with_sample(
        &self,
        sample: &ParallelPanelProjectionSample,
    ) -> bool {
        ParallelPanelStateController::activity_pulse_visible(&self.parallel_panel_ui_state(sample))
    }

    pub(super) fn parallel_mode_prompt_input_locked_with_sample(
        &self,
        sample: &ParallelPanelProjectionSample,
    ) -> bool {
        ParallelPanelStateController::prompt_input_locked(&self.parallel_panel_ui_state(sample))
    }

    fn parallel_panel_ui_state(
        &self,
        sample: &ParallelPanelProjectionSample,
    ) -> ParallelPanelUiState {
        let mode_enabled = sample.parallel_mode_enabled();
        let overlay_event = if self.parallel_mode_panel_visible(mode_enabled) {
            ParallelPanelUiEvent::OverlayShown
        } else {
            ParallelPanelUiEvent::OverlayHidden
        };
        let mut events = vec![
            overlay_event,
            ParallelPanelUiEvent::ModeSet(mode_enabled),
            ParallelPanelUiEvent::SupervisorSnapshotChanged(
                sample
                    .parallel_mode_supervisor_for_workspace(Some(
                        &self.planning_workspace_directory(),
                    ))
                    .map(Box::new),
            ),
        ];
        if let Some(reason) = sample.last_parallel_mode_dispatch_withheld_reason() {
            events.push(ParallelPanelUiEvent::StatusShown(format!(
                "parallel mode: dispatch withheld / {reason}"
            )));
        }
        ParallelPanelStateController::project(events)
    }

    fn parallel_mode_panel_visible(&self, mode_enabled: bool) -> bool {
        self.shell_overlay == ShellOverlay::Supersession
            || (self.shell_overlay == ShellOverlay::Hidden && mode_enabled)
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
                    ParallelModePresentationLoadingStage::RefreshingBoard,
                ),
            ));
        }
        self.apply_parallel_mode_control_plane_command(
            ParallelModeControlPlaneCommand::RefreshSupervisor {
                workspace_directory,
            },
        );
    }

    pub(super) fn inspect_parallel_mode_supervisor(
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
                // The parsed parallel command is the enable entrypoint. Open
                // the control tower first, then send one application command.
                // The runtime owns mode and initial-reset policy; this adapter
                // only projects the loading state.
                let workspace_directory = self.planning_workspace_directory();
                if let ConversationState::Ready(conversation) = &mut self.conversation_state {
                    conversation.rearm_parallel_post_turn_continuation();
                }
                self.sync_core_parallel_mode_readiness_projection(None);
                self.sync_core_parallel_mode_supervisor_projection(Some(
                    pending_parallel_mode_supervisor_snapshot(
                        &workspace_directory,
                        true,
                        None,
                        ParallelModePresentationLoadingStage::Entering,
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

    pub(super) fn parallel_mode_post_turn_queue_projection(
        &self,
        event: &ConversationRuntimeEvent,
    ) -> (
        Option<ParallelModePostTurnQueueSignal>,
        Option<String>,
        bool,
    ) {
        let ConversationRuntimeEvent::PostTurnEvaluationCompleted { evaluation } = event else {
            return (None, None, false);
        };
        (
            evaluation.provenance.parallel_queue_signal,
            evaluation
                .provenance
                .runtime_projection_workspace_directory
                .clone(),
            evaluation.provenance.has_actionable_queue_head,
        )
    }

    pub(super) fn apply_parallel_mode_post_turn_queue_continuation(
        &mut self,
        accepted_workspace_directory: Option<String>,
        auto_follow_prompt_queued: bool,
        event_signal: Option<ParallelModePostTurnQueueSignal>,
        has_actionable_queue_head: bool,
    ) -> bool {
        let workspace_directory =
            accepted_workspace_directory.unwrap_or_else(|| self.planning_workspace_directory());
        let control_plane = self.parallel_mode_control_plane.clone();
        let outcome = control_plane.continue_post_turn_queue(
            workspace_directory,
            event_signal,
            auto_follow_prompt_queued,
            has_actionable_queue_head,
        );
        self.apply_parallel_mode_control_plane_presentation_events(outcome.presentation_events);
        outcome.auto_follow_prompt_consumed
    }

    pub(super) fn close_parallel_mode_automation_epoch(&mut self) {
        // A post-turn evaluator can have captured parallel mode as its only
        // continuation opt-in. Closing the epoch must invalidate that evaluator
        // as well as the control-plane workers; otherwise a late result can
        // recreate a queued continuation after the operator turned parallel off.
        self.post_turn_continuation_gate.advance();
        if let ConversationState::Ready(conversation) = &mut self.conversation_state {
            conversation.disarm_parallel_post_turn_continuation();
        }
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

    pub(super) fn close_parallel_mode_epoch_before_workspace_transition(
        &mut self,
        target_workspace_directory: &str,
    ) {
        let epoch = self.parallel_mode_control_plane.epoch_snapshot();
        let leaves_active_workspace = epoch.current_epoch_id.is_some()
            && epoch.workspace_directory.as_deref() != Some(target_workspace_directory);
        if leaves_active_workspace {
            self.close_parallel_mode_automation_epoch();
        }
    }

    pub(super) fn open_parallel_mode_automation_epoch(&mut self, workspace_directory: String) {
        self.apply_parallel_mode_control_plane_command(
            ParallelModeControlPlaneCommand::OpenEpoch {
                workspace_directory,
            },
        );
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

    pub(super) fn request_parallel_mode_dispatch(
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

    pub(super) fn tick_parallel_mode_control_plane(
        &mut self,
        now: Instant,
        sample: &ParallelPanelProjectionSample,
    ) -> bool {
        let workspace_directory = self.planning_workspace_directory();
        let activity_pulse_visible = self.parallel_mode_activity_pulse_visible_with_sample(sample);
        let events =
            self.parallel_mode_control_plane
                .tick(now, workspace_directory, activity_pulse_visible);
        self.apply_parallel_mode_control_plane_presentation_events(events)
    }

    #[cfg(test)]
    pub(super) fn parallel_mode_supervisor_refresh_due_for_test(&self, now: Instant) -> bool {
        let sample = ParallelPanelProjectionSample::capture(self);
        self.parallel_mode_supervisor_refresh_due_with_sample_for_test(now, &sample)
    }

    #[cfg(test)]
    pub(super) fn parallel_mode_supervisor_refresh_due_with_sample_for_test(
        &self,
        now: Instant,
        sample: &ParallelPanelProjectionSample,
    ) -> bool {
        self.parallel_mode_control_plane.supervisor_refresh_due(
            now,
            self.parallel_mode_activity_pulse_visible_with_sample(sample),
        )
    }

    fn sync_core_parallel_mode_readiness_projection(
        &mut self,
        snapshot: Option<ParallelModeReadinessSnapshot>,
    ) {
        self.dispatch_client_event(CoreInput::ParallelModeReadinessProjectionChanged(
            snapshot.map(Box::new),
        ));
    }

    fn sync_core_parallel_mode_supervisor_projection(
        &mut self,
        snapshot: Option<ParallelModeSupervisorSnapshot>,
    ) {
        if let Some(snapshot) = snapshot.as_ref() {
            self.record_parallel_supervisor_snapshot_for_stream(snapshot);
        }
        self.dispatch_client_event(CoreInput::ParallelModeSupervisorProjectionChanged(
            snapshot.map(Box::new),
        ));
    }

    #[cfg(test)]
    pub(crate) fn set_parallel_mode_readiness_snapshot_for_test(
        &mut self,
        snapshot: Option<ParallelModeReadinessSnapshot>,
    ) {
        self.sync_core_parallel_mode_readiness_projection(snapshot);
    }

    #[cfg(test)]
    pub(crate) fn set_parallel_mode_supervisor_snapshot_for_test(
        &mut self,
        snapshot: Option<ParallelModeSupervisorSnapshot>,
    ) {
        self.sync_core_parallel_mode_supervisor_projection(snapshot);
    }
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
mod global_runtime_notice_tests {
    use super::*;
    use crate::adapter::inbound::tui::app::test_helpers::{
        self, test_native_tui_app, test_native_tui_app_with_parallel_mode_composition,
    };
    use crate::adapter::inbound::tui::app::{
        ConversationViewModel, shell_presentation::ConversationScreenModel,
    };
    use crate::adapter::outbound::filesystem::FilesystemPlanningWorkspaceAdapter;
    use crate::application::port::outbound::parallel_agent_worker_port::NoopParallelAgentWorkerPort;
    use crate::application::port::outbound::planning_authority_port::NoopPlanningAuthorityPort;
    use crate::application::port::outbound::planning_task_repository_port::NoopPlanningTaskRepositoryPort;
    use crate::application::port::outbound::planning_worker_port::NoopPlanningWorkerPort;
    use crate::application::service::planning::PlanningServices;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    fn tick_parallel_mode_control_plane_for_test(app: &mut NativeTuiApp, now: Instant) {
        let sample = ParallelPanelProjectionSample::capture(app);
        app.tick_parallel_mode_control_plane(now, &sample);
    }

    #[test]
    fn ordinary_runtime_notice_enters_client_runtime_and_projects_once() {
        let mut app = test_native_tui_app();
        let workspace_directory = app.planning_workspace_directory();
        let notice = "parallel worker completed".to_string();

        app.apply_parallel_mode_control_plane_presentation_events(vec![
            ParallelModeControlPlanePresentationEvent::ConversationRuntimeNotice {
                workspace_directory,
                notice: notice.clone(),
            },
        ]);

        let ConversationState::Ready(conversation) = &app.conversation_state else {
            panic!("runtime notice must preserve the ready conversation");
        };
        assert_eq!(
            conversation
                .runtime_notices
                .iter()
                .filter(|candidate| *candidate == &notice)
                .count(),
            1
        );
    }

    #[test]
    fn global_runtime_notice_marker_requests_redraw_without_tui_state_mutation() {
        let mut app = test_native_tui_app();
        let before = match &app.conversation_state {
            ConversationState::Ready(conversation) => conversation.runtime_notices.clone(),
            _ => panic!("test app must start ready"),
        };

        assert!(
            app.apply_parallel_mode_control_plane_presentation_events(vec![
                ParallelModeControlPlanePresentationEvent::GlobalRuntimeNoticesChanged,
            ]),
            "projection invalidation must preserve the redraw signal"
        );

        let ConversationState::Ready(conversation) = &app.conversation_state else {
            panic!("redraw invalidation must preserve conversation state");
        };
        assert_eq!(conversation.runtime_notices, before);
        assert!(
            ConversationScreenModel::from_app(&app)
                .global_runtime_notices
                .is_empty(),
            "the marker must not fabricate projection state in the TUI"
        );
    }

    #[test]
    fn owned_cleanup_projection_survives_non_ready_state_and_settles_without_dual_writes() {
        for (operation_id, initial_state) in [
            (11, ConversationState::Loading),
            (
                12,
                ConversationState::Failed("conversation load failed".to_string()),
            ),
        ] {
            let mutation_gate = Arc::new(Mutex::new(()));
            let mutation_count = Arc::new(AtomicUsize::new(0));
            let cancel_error = Arc::new(Mutex::new(Some("sqlite cleanup busy".to_string())));
            let authority = Arc::new(
                NoopPlanningAuthorityPort::default()
                    .with_shared_runtime_dispatch_mutation_gate(mutation_gate.clone())
                    .with_shared_runtime_dispatch_mutation_count(mutation_count.clone())
                    .with_shared_cancel_runtime_dispatch_commands_error(cancel_error),
            );
            let mut app = test_app_with_retry_authority(authority);
            let workspace_directory = format!("/tmp/pulse-cleanup-{operation_id}");
            app.conversation_state = initial_state;
            app.parallel_mode_control_plane
                .force_epoch_for_test(&workspace_directory, 1);

            app.apply_parallel_mode_control_plane_command(
                ParallelModeControlPlaneCommand::Disable {
                    workspace_directory: workspace_directory.clone(),
                },
            );
            wait_for_mutation_count(&mutation_count, 1);
            let failed_cleanup = recv_control_plane_background_event(&app);
            assert!(matches!(
                &failed_cleanup,
                ParallelModeControlPlaneBackgroundEvent::DispatchMutationCompleted {
                    result: Err(error),
                    ..
                } if error == "sqlite cleanup busy"
            ));
            let replacement_workspace = format!("/tmp/pulse-cleanup-replacement-{operation_id}");
            app.parallel_mode_control_plane
                .force_epoch_for_test(&replacement_workspace, 2);
            assert!(
                app.parallel_mode_control_plane
                    .presentation_projection()
                    .global_runtime_notices
                    .is_empty(),
                "the worker must not mutate authority before its background event is reduced"
            );
            app.apply_parallel_mode_control_plane_background_event(failed_cleanup);
            let projection = app
                .parallel_mode_control_plane
                .presentation_projection()
                .global_runtime_notices;
            let [projected_notice] = projection.as_slice() else {
                panic!("failed cancellation should project one exact notice: {projection:?}");
            };
            let original_cleanup = projected_notice.cleanup_correlation.clone();
            let notice_copy = projected_notice.notice.clone();
            assert_eq!(original_cleanup.workspace_directory, workspace_directory);
            assert_eq!(original_cleanup.epoch_id, 1);
            assert_eq!(
                original_cleanup.command_identity,
                "cancel_runtime_dispatch_commands"
            );
            assert!(!matches!(
                app.conversation_state,
                ConversationState::Ready(_)
            ));
            assert_eq!(
                ConversationScreenModel::from_app(&app).global_runtime_notices,
                vec![notice_copy.clone()],
                "a non-ready conversation must still capture the application-owned projection"
            );

            let gate_guard = mutation_gate
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            let first_pulse = Instant::now();
            tick_parallel_mode_control_plane_for_test(&mut app, first_pulse);
            wait_for_mutation_count(&mutation_count, 2);
            for seconds in 1..=4 {
                tick_parallel_mode_control_plane_for_test(
                    &mut app,
                    first_pulse + Duration::from_secs(seconds),
                );
            }
            assert_eq!(
                mutation_count.load(Ordering::SeqCst),
                2,
                "duplicate pulses must share the one exact cleanup retry worker"
            );

            app.conversation_state = ConversationState::ready(ConversationViewModel::new_draft(
                workspace_directory.clone(),
            ));
            assert_eq!(
                ConversationScreenModel::from_app(&app).global_runtime_notices,
                vec![notice_copy.clone()],
                "becoming ready must surface the same projection without a lifecycle write"
            );
            app.dispatch_client_event(CoreInput::ConversationRuntimeNotice(notice_copy.clone()));

            drop(gate_guard);
            let settled_cleanup = recv_control_plane_background_event(&app);
            assert!(matches!(
                settled_cleanup,
                ParallelModeControlPlaneBackgroundEvent::DispatchMutationCompleted {
                    result: Ok(0),
                    ..
                }
            ));
            assert_eq!(
                app.parallel_mode_control_plane
                    .presentation_projection()
                    .global_runtime_notices
                    .len(),
                1,
                "the background worker must not settle authority until its event is reduced"
            );
            app.apply_parallel_mode_control_plane_background_event(settled_cleanup);

            assert_eq!(
                mutation_count.load(Ordering::SeqCst),
                2,
                "the production pulse must execute one additional cancellation"
            );
            assert!(
                app.parallel_mode_control_plane
                    .presentation_projection()
                    .global_runtime_notices
                    .is_empty()
            );
            assert!(
                ConversationScreenModel::from_app(&app)
                    .global_runtime_notices
                    .is_empty(),
                "the next frame must own the settled projection without a TUI clear writer"
            );
            let ConversationState::Ready(conversation) = &app.conversation_state else {
                panic!("cleanup settlement must preserve the ready conversation");
            };
            assert!(
                conversation.runtime_notices.contains(&notice_copy),
                "settling cleanup must not delete an ordinary notice with identical copy"
            );
            assert_eq!(
                app.parallel_mode_control_plane.epoch_snapshot(),
                crate::application::service::parallel_mode::control_plane::ParallelModeControlPlaneEpochSnapshot {
                    workspace_directory: Some(replacement_workspace),
                    current_epoch_id: Some(2),
                },
                "exact stale cleanup retry must not replace the newer workspace projection"
            );
        }
    }

    #[test]
    fn production_tui_pulse_uses_its_existing_interval_after_retry_failure() {
        let mutation_count = Arc::new(AtomicUsize::new(0));
        let authority = Arc::new(
            NoopPlanningAuthorityPort::default()
                .with_shared_runtime_dispatch_mutation_count(mutation_count.clone())
                .with_cancel_runtime_dispatch_commands_error("sqlite cleanup busy"),
        );
        let mut app = test_app_with_retry_authority(authority);
        let workspace_directory = "/tmp/pulse-cleanup-backoff".to_string();
        app.conversation_state = ConversationState::Loading;
        app.parallel_mode_control_plane
            .force_epoch_for_test(&workspace_directory, 1);

        app.apply_parallel_mode_control_plane_command(ParallelModeControlPlaneCommand::Disable {
            workspace_directory,
        });
        wait_for_mutation_count(&mutation_count, 1);
        let failed_cleanup = recv_control_plane_background_event(&app);
        app.apply_parallel_mode_control_plane_background_event(failed_cleanup);

        let first_pulse = Instant::now();
        tick_parallel_mode_control_plane_for_test(&mut app, first_pulse);
        wait_for_mutation_count(&mutation_count, 2);
        let failed_retry = recv_control_plane_background_event(&app);
        app.apply_parallel_mode_control_plane_background_event(failed_retry);

        tick_parallel_mode_control_plane_for_test(
            &mut app,
            first_pulse + Duration::from_millis(999),
        );
        assert_eq!(
            mutation_count.load(Ordering::SeqCst),
            2,
            "a failed retry must not create a busy loop before the next pulse interval"
        );

        tick_parallel_mode_control_plane_for_test(&mut app, first_pulse + Duration::from_secs(1));
        wait_for_mutation_count(&mutation_count, 3);
        let failed_retry = recv_control_plane_background_event(&app);
        app.apply_parallel_mode_control_plane_background_event(failed_retry);
        assert_eq!(
            app.parallel_mode_control_plane
                .presentation_projection()
                .global_runtime_notices
                .len(),
            1,
            "the exact cleanup must remain one unsettled row across periodic retries"
        );
    }

    fn test_app_with_retry_authority(authority: Arc<NoopPlanningAuthorityPort>) -> NativeTuiApp {
        let planning = PlanningServices::from_ports(
            Arc::new(FilesystemPlanningWorkspaceAdapter::new()),
            authority.clone(),
            Arc::new(NoopPlanningTaskRepositoryPort),
            Arc::new(NoopPlanningWorkerPort),
        );
        let parallel_mode_service =
            test_helpers::test_parallel_mode_service_with_authority(authority);
        let composition = test_helpers::test_parallel_mode_control_plane_composition_with_worker(
            parallel_mode_service,
            planning,
            Arc::new(NoopParallelAgentWorkerPort),
        );
        test_native_tui_app_with_parallel_mode_composition(composition)
    }

    fn recv_control_plane_background_event(
        app: &NativeTuiApp,
    ) -> ParallelModeControlPlaneBackgroundEvent {
        match app
            .rx
            .recv_timeout(Duration::from_secs(2))
            .expect("control-plane worker should return through the production TUI channel")
        {
            super::super::BackgroundMessage::ParallelModeControlPlaneEvent(event) => *event,
            event => panic!("expected a control-plane background event, got {event:?}"),
        }
    }

    fn wait_for_mutation_count(count: &AtomicUsize, expected: usize) {
        let deadline = Instant::now() + Duration::from_secs(2);
        while count.load(Ordering::SeqCst) < expected && Instant::now() < deadline {
            std::thread::yield_now();
        }
        assert_eq!(count.load(Ordering::SeqCst), expected);
    }
}

#[cfg(test)]
mod orchestrator_retry_tests {
    use super::*;
    use crate::domain::parallel_mode::{
        ParallelModeAgentRosterSnapshot, ParallelModeDistributorQueueItem,
        ParallelModeDistributorSnapshot, ParallelModeOrchestratorStatus,
        ParallelModePoolBoardSnapshot, ParallelModeQueueItemState,
        ParallelModeSupervisorDetailSnapshot, ParallelModeSupervisorSnapshot,
        ParallelModeSupervisorState,
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
