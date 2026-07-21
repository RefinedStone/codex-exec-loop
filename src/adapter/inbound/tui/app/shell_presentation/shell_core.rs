/*
 * ConversationProjectionSample captures the conversation shell's narrow Core
 * projection and render clocks once per terminal transaction.
 * ConversationScreenModel combines that owned sample with the UI-only facts
 * needed by copy, layout, cursor, and frame-cache code at one projection point.
 */
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use ratatui::text::Line;

use crate::application::service::parallel_mode::control_plane::ParallelModeControlPlanePresentationProjection;
use crate::application::service::planning::PlanningRuntimeProjection;
use crate::core::app::{
    ParallelModeProjection, PlanningParallelProjection, RevisionedPlanningParallelProjection,
};
use crate::domain::parallel_mode::{ParallelModeReadinessSnapshot, ParallelModeSupervisorSnapshot};
use crate::domain::planning::PlanningWorkerPanelState;

use super::super::parallel_presentation_bridge::{
    ParallelModePresentationLoadingStage, pending_parallel_mode_supervisor_snapshot,
};
use super::super::parallel_supervisor_events::ParallelSupervisorEventProjection;
use super::capability_projection::recent_session_status_label;
use super::{
    ConversationState, ConversationViewModel, HistoryInsertionMode, InlineHistoryRenderMode,
    NativeTuiApp, ParallelPanelStateController, ShellActionAvailability, ShellOverlay,
    StartupState, TuiLanguage,
};

const MAX_GITHUB_REVIEW_NOTICE_LEN: usize = 160;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::adapter::inbound::tui::app) struct ParallelPanelProjectionSample {
    parallel_control_plane: ParallelModeControlPlanePresentationProjection,
    parallel_mode: ParallelModeProjection,
}

impl ParallelPanelProjectionSample {
    pub(in crate::adapter::inbound::tui::app) fn capture(app: &NativeTuiApp) -> Self {
        Self::from_parts(
            app.core_runtime.parallel_mode_projection(),
            app.parallel_mode_control_plane.presentation_projection(),
        )
    }

    fn from_parts(
        parallel_mode: ParallelModeProjection,
        parallel_control_plane: ParallelModeControlPlanePresentationProjection,
    ) -> Self {
        Self {
            parallel_control_plane,
            parallel_mode,
        }
    }

    pub(in crate::adapter::inbound::tui::app) fn parallel_mode_enabled(&self) -> bool {
        self.parallel_control_plane.mode_enabled
    }

    pub(in crate::adapter::inbound::tui::app) fn parallel_mode_control_effect_in_flight(
        &self,
    ) -> bool {
        self.parallel_control_plane.control_effect_in_flight
    }

    pub(in crate::adapter::inbound::tui::app) fn last_parallel_mode_dispatch_withheld_reason(
        &self,
    ) -> Option<&str> {
        self.parallel_control_plane
            .last_dispatch_withheld_reason
            .as_deref()
    }

    pub(in crate::adapter::inbound::tui::app) fn parallel_mode_readiness_for_workspace(
        &self,
        workspace_directory: Option<&str>,
    ) -> Option<ParallelModeReadinessSnapshot> {
        self.parallel_mode
            .readiness
            .as_deref()
            .filter(|snapshot| {
                workspace_directory.is_none_or(|workspace| snapshot.workspace_path == workspace)
            })
            .cloned()
    }

    pub(in crate::adapter::inbound::tui::app) fn parallel_mode_supervisor_for_workspace(
        &self,
        workspace_directory: Option<&str>,
    ) -> Option<ParallelModeSupervisorSnapshot> {
        self.parallel_mode
            .supervisor
            .as_deref()
            .filter(|snapshot| {
                workspace_directory.is_none_or(|workspace| snapshot.workspace_path == workspace)
            })
            .cloned()
    }
}

pub(in crate::adapter::inbound::tui::app) struct ConversationProjectionSample {
    core_revision: u64,
    conversation_history_identity_revision: u64,
    planning_runtime_workspace_directory: Option<String>,
    planning_runtime: Box<PlanningRuntimeProjection>,
    parallel_panel: ParallelPanelProjectionSample,
    parallel_supervisor_events: ParallelSupervisorEventProjection,
    inline_history_render_mode: InlineHistoryRenderMode,
    history_insert_mode: HistoryInsertionMode,
    rendered_at: Instant,
    animation_elapsed_millis: u128,
}

impl ConversationProjectionSample {
    pub(in crate::adapter::inbound::tui::app) fn capture(app: &NativeTuiApp) -> Self {
        let RevisionedPlanningParallelProjection {
            revision: core_revision,
            planning_parallel,
        } = app.core_runtime.revisioned_planning_parallel_projection();
        let PlanningParallelProjection {
            planning_runtime_workspace_directory,
            planning_runtime,
            parallel_mode,
        } = planning_parallel;
        let parallel_control_plane = app.parallel_mode_control_plane.presentation_projection();
        Self {
            core_revision,
            conversation_history_identity_revision: app.conversation_history_identity_revision,
            planning_runtime_workspace_directory,
            planning_runtime,
            parallel_panel: ParallelPanelProjectionSample::from_parts(
                parallel_mode,
                parallel_control_plane,
            ),
            parallel_supervisor_events: app.parallel_supervisor_event_log.projection(),
            inline_history_render_mode: app.inline_history_render_mode,
            history_insert_mode: app.history_insert_mode,
            rendered_at: Instant::now(),
            animation_elapsed_millis: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_or(0, |duration| duration.as_millis()),
        }
    }

    pub(in crate::adapter::inbound::tui::app) fn parallel_mode_enabled(&self) -> bool {
        self.parallel_panel.parallel_mode_enabled()
    }

    pub(in crate::adapter::inbound::tui::app) fn conversation_history_identity_revision(
        &self,
    ) -> u64 {
        self.conversation_history_identity_revision
    }

    pub(in crate::adapter::inbound::tui::app) fn parallel_mode_control_effect_in_flight(
        &self,
    ) -> bool {
        self.parallel_panel.parallel_mode_control_effect_in_flight()
    }

    pub(in crate::adapter::inbound::tui::app) fn last_parallel_mode_dispatch_withheld_reason(
        &self,
    ) -> Option<&str> {
        self.parallel_panel
            .last_parallel_mode_dispatch_withheld_reason()
    }

    #[cfg(test)]
    pub(in crate::adapter::inbound::tui::app) fn parallel_panel_sample(
        &self,
    ) -> &ParallelPanelProjectionSample {
        &self.parallel_panel
    }

    pub(in crate::adapter::inbound::tui::app) fn inline_history_render_mode(
        &self,
    ) -> InlineHistoryRenderMode {
        self.inline_history_render_mode
    }

    pub(in crate::adapter::inbound::tui::app) fn history_insert_mode(
        &self,
    ) -> HistoryInsertionMode {
        self.history_insert_mode
    }

    pub(in crate::adapter::inbound::tui::app) fn parallel_supervisor_event_lines(
        &self,
    ) -> Vec<Line<'static>> {
        self.parallel_supervisor_events.live_lines()
    }

    pub(in crate::adapter::inbound::tui::app) fn parallel_supervisor_event_scrollback_lines_before_live_tail(
        &self,
        live_tail_rows: usize,
        width: u16,
    ) -> Vec<Line<'static>> {
        self.parallel_supervisor_events
            .scrollback_lines_before_rendered_live_tail(live_tail_rows, width)
    }

    pub(in crate::adapter::inbound::tui::app) fn parallel_mode_readiness_for_workspace(
        &self,
        workspace_directory: Option<&str>,
    ) -> Option<ParallelModeReadinessSnapshot> {
        self.parallel_panel
            .parallel_mode_readiness_for_workspace(workspace_directory)
    }

    pub(in crate::adapter::inbound::tui::app) fn parallel_mode_supervisor_for_workspace(
        &self,
        workspace_directory: Option<&str>,
    ) -> Option<ParallelModeSupervisorSnapshot> {
        self.parallel_panel
            .parallel_mode_supervisor_for_workspace(workspace_directory)
    }
}

#[derive(Clone, Copy)]
pub(in crate::adapter::inbound::tui::app) enum ShellConversationState<'a> {
    Loading,
    Failed(&'a str),
    Ready(&'a ConversationViewModel),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::adapter::inbound::tui::app) enum QueueMutationTailState {
    Idle,
    Pending(u64),
    RefreshRequired,
    UndoAvailable(usize),
}

pub(in crate::adapter::inbound::tui::app) struct ConversationScreenModel<'a> {
    pub(in crate::adapter::inbound::tui::app) core_revision: u64,
    pub(in crate::adapter::inbound::tui::app) rendered_at: Instant,
    pub(in crate::adapter::inbound::tui::app) animation_elapsed_millis: u128,
    pub(in crate::adapter::inbound::tui::app) startup_state: &'a StartupState,
    pub(in crate::adapter::inbound::tui::app) shell_action_availability: ShellActionAvailability,
    pub(in crate::adapter::inbound::tui::app) recent_session_status_label: String,
    pub(in crate::adapter::inbound::tui::app) github_review_polling_status_label: String,
    pub(in crate::adapter::inbound::tui::app) github_review_recent_changes_summary: Option<String>,
    pub(in crate::adapter::inbound::tui::app) tui_language: TuiLanguage,
    pub(in crate::adapter::inbound::tui::app) parallel_mode_enabled: bool,
    pub(in crate::adapter::inbound::tui::app) parallel_mode_control_effect_in_flight: bool,
    pub(in crate::adapter::inbound::tui::app) last_parallel_mode_dispatch_withheld_reason:
        Option<String>,
    pub(in crate::adapter::inbound::tui::app) parallel_mode_loading_prompt_indicator_visible: bool,
    pub(in crate::adapter::inbound::tui::app) parallel_mode_readiness:
        Option<ParallelModeReadinessSnapshot>,
    pub(in crate::adapter::inbound::tui::app) parallel_mode_supervisor:
        ParallelModeSupervisorSnapshot,
    pub(in crate::adapter::inbound::tui::app) parallel_supervisor_event_lines: Vec<Line<'static>>,
    pub(in crate::adapter::inbound::tui::app) planning_runtime_projection:
        PlanningRuntimeProjection,
    pub(in crate::adapter::inbound::tui::app) planning_worker_shows_debug_details: bool,
    pub(in crate::adapter::inbound::tui::app) planning_worker_panel_state: PlanningWorkerPanelState,
    pub(in crate::adapter::inbound::tui::app) queue_mutation_tail_state: QueueMutationTailState,
    pub(in crate::adapter::inbound::tui::app) turn_options_summary: Option<String>,
    pub(in crate::adapter::inbound::tui::app) shell_overlay: ShellOverlay,
    pub(in crate::adapter::inbound::tui::app) inline_history_render_mode: InlineHistoryRenderMode,
    pub(in crate::adapter::inbound::tui::app) exit_confirmation_visible: bool,
    pub(in crate::adapter::inbound::tui::app) turn_steer_confirmation_visible: bool,
    pub(in crate::adapter::inbound::tui::app) prompt_input_has_focus: bool,
    pub(in crate::adapter::inbound::tui::app) conversation_state: ShellConversationState<'a>,
}

impl<'a> ConversationScreenModel<'a> {
    #[cfg(test)]
    pub(in crate::adapter::inbound::tui::app) fn from_app(app: &'a NativeTuiApp) -> Self {
        let sample = ConversationProjectionSample::capture(app);
        Self::from_app_with_sample(app, &sample)
    }

    pub(in crate::adapter::inbound::tui::app) fn from_app_with_sample(
        app: &'a NativeTuiApp,
        sample: &ConversationProjectionSample,
    ) -> Self {
        let core_revision = sample.core_revision;
        let workspace_directory = presentation_workspace_directory(app);
        let planning_runtime_projection = if sample.planning_runtime_workspace_directory.as_deref()
            == workspace_directory.as_deref()
        {
            (*sample.planning_runtime).clone()
        } else {
            PlanningRuntimeProjection::uninitialized()
        };
        let parallel_mode_enabled = sample.parallel_mode_enabled();
        let parallel_mode_control_effect_in_flight =
            sample.parallel_mode_control_effect_in_flight();
        let parallel_mode_readiness =
            sample.parallel_mode_readiness_for_workspace(workspace_directory.as_deref());
        let current_parallel_mode_supervisor =
            sample.parallel_mode_supervisor_for_workspace(workspace_directory.as_deref());
        let parallel_panel_visible = app.shell_overlay == ShellOverlay::Supersession
            || (app.shell_overlay == ShellOverlay::Hidden && parallel_mode_enabled);
        let parallel_mode_loading_prompt_indicator_visible = parallel_panel_visible
            && parallel_mode_enabled
            && current_parallel_mode_supervisor
                .as_ref()
                .is_none_or(ParallelPanelStateController::snapshot_is_loading);
        let parallel_mode_prompt_input_locked = parallel_mode_loading_prompt_indicator_visible;
        let snapshot_workspace_directory = current_parallel_mode_supervisor
            .as_ref()
            .map(|snapshot| snapshot.workspace_path.clone())
            .or_else(|| {
                parallel_mode_readiness
                    .as_ref()
                    .map(|snapshot| snapshot.workspace_path.clone())
            });
        let presentation_workspace_directory = workspace_directory
            .clone()
            .or(snapshot_workspace_directory)
            .unwrap_or_else(|| ".".to_string());
        let parallel_mode_supervisor = current_parallel_mode_supervisor.unwrap_or_else(|| {
            pending_parallel_mode_supervisor_snapshot(
                &presentation_workspace_directory,
                parallel_mode_enabled,
                parallel_mode_readiness.as_ref(),
                ParallelModePresentationLoadingStage::Entering,
            )
        });
        let exit_confirmation_visible = app.is_exit_confirmation_visible();
        let turn_steer_confirmation_visible = app.is_turn_steer_confirmation_visible();
        let dialog_visible = exit_confirmation_visible || turn_steer_confirmation_visible;
        let prompt_input_has_focus = app
            .shell_overlay
            .prompt_input_has_focus(dialog_visible, parallel_mode_prompt_input_locked);
        let queue_mutation_tail_state =
            if let Some(operation_id) = app.pending_queue_mutation_operation_id() {
                QueueMutationTailState::Pending(operation_id)
            } else if app.queue_mutation_requires_authority_refresh() {
                QueueMutationTailState::RefreshRequired
            } else if let Some(task_count) = app.queue_receipt_undo_task_count() {
                QueueMutationTailState::UndoAvailable(task_count)
            } else {
                QueueMutationTailState::Idle
            };

        Self {
            core_revision,
            rendered_at: sample.rendered_at,
            animation_elapsed_millis: sample.animation_elapsed_millis,
            startup_state: &app.startup_state,
            shell_action_availability: app.shell_action_availability(),
            recent_session_status_label: recent_session_status_label(app, app.tui_language),
            github_review_polling_status_label: app.github_review_polling_status_label(),
            github_review_recent_changes_summary: app
                .github_review_recent_changes_summary(MAX_GITHUB_REVIEW_NOTICE_LEN),
            tui_language: app.tui_language,
            parallel_mode_enabled,
            parallel_mode_control_effect_in_flight,
            last_parallel_mode_dispatch_withheld_reason: sample
                .last_parallel_mode_dispatch_withheld_reason()
                .map(str::to_string),
            parallel_mode_loading_prompt_indicator_visible,
            parallel_mode_readiness,
            parallel_mode_supervisor,
            parallel_supervisor_event_lines: if parallel_mode_enabled {
                sample.parallel_supervisor_event_lines()
            } else {
                Vec::new()
            },
            planning_runtime_projection,
            planning_worker_shows_debug_details: app.planning_worker_shows_debug_details(),
            planning_worker_panel_state: app.planning_worker_panel_state.clone(),
            queue_mutation_tail_state,
            turn_options_summary: (!app.turn_options.is_default())
                .then(|| app.turn_options.summary_label()),
            shell_overlay: app.shell_overlay,
            inline_history_render_mode: sample.inline_history_render_mode(),
            exit_confirmation_visible,
            turn_steer_confirmation_visible,
            prompt_input_has_focus,
            conversation_state: match &app.conversation_state {
                ConversationState::Loading => ShellConversationState::Loading,
                ConversationState::Failed(message) => ShellConversationState::Failed(message),
                ConversationState::Ready(conversation) => {
                    ShellConversationState::Ready(conversation)
                }
            },
        }
    }

    pub(in crate::adapter::inbound::tui::app) fn ready_conversation(
        &self,
    ) -> Option<&'a ConversationViewModel> {
        match self.conversation_state {
            ShellConversationState::Ready(conversation) => Some(conversation),
            _ => None,
        }
    }

    pub(in crate::adapter::inbound::tui::app) fn startup_screen_is_active(&self) -> bool {
        conversation_startup_screen_is_active(self.parallel_mode_enabled, self.ready_conversation())
    }

    pub(in crate::adapter::inbound::tui::app) fn dialog_visible(&self) -> bool {
        self.exit_confirmation_visible || self.turn_steer_confirmation_visible
    }

    pub(in crate::adapter::inbound::tui::app) fn renders_viewport_transcript_handoff(
        &self,
    ) -> bool {
        self.shell_overlay == ShellOverlay::Hidden
            && !self.dialog_visible()
            && matches!(
                self.inline_history_render_mode,
                InlineHistoryRenderMode::ViewportReplay
            )
            && matches!(
                self.conversation_state,
                ShellConversationState::Ready(conversation)
                    if conversation
                        .viewport_transcript_handoff_release_messages()
                        .is_some()
            )
    }

    pub(in crate::adapter::inbound::tui::app) fn renders_parallel_viewport_handoff(&self) -> bool {
        self.parallel_mode_enabled && self.renders_viewport_transcript_handoff()
    }

    pub(in crate::adapter::inbound::tui::app) fn live_transcript_lines_include_committed_handoff(
        &self,
    ) -> bool {
        self.inline_history_render_mode.writes_host_scrollback()
            || self
                .ready_conversation()
                .is_some_and(ConversationViewModel::has_pending_viewport_transcript_handoff)
    }

    #[cfg(test)]
    pub(in crate::adapter::inbound::tui::app) fn from_test_parts(
        startup_state: &'a StartupState,
        shell_action_availability: ShellActionAvailability,
        conversation_state: ShellConversationState<'a>,
    ) -> Self {
        Self {
            core_revision: 0,
            rendered_at: Instant::now(),
            animation_elapsed_millis: 0,
            startup_state,
            shell_action_availability,
            recent_session_status_label: "loaded".to_string(),
            github_review_polling_status_label: "polling".to_string(),
            github_review_recent_changes_summary: None,
            tui_language: TuiLanguage::English,
            parallel_mode_enabled: false,
            parallel_mode_control_effect_in_flight: false,
            last_parallel_mode_dispatch_withheld_reason: None,
            parallel_mode_loading_prompt_indicator_visible: false,
            parallel_mode_readiness: None,
            parallel_mode_supervisor: pending_parallel_mode_supervisor_snapshot(
                ".",
                false,
                None,
                ParallelModePresentationLoadingStage::Entering,
            ),
            parallel_supervisor_event_lines: Vec::new(),
            planning_runtime_projection: PlanningRuntimeProjection::uninitialized(),
            planning_worker_shows_debug_details: false,
            planning_worker_panel_state: PlanningWorkerPanelState::default(),
            queue_mutation_tail_state: QueueMutationTailState::Idle,
            turn_options_summary: None,
            shell_overlay: ShellOverlay::Hidden,
            inline_history_render_mode: InlineHistoryRenderMode::HostScrollback,
            exit_confirmation_visible: false,
            turn_steer_confirmation_visible: false,
            prompt_input_has_focus: true,
            conversation_state,
        }
    }
}

pub(in crate::adapter::inbound::tui::app) fn conversation_startup_screen_is_active(
    parallel_mode_enabled: bool,
    conversation: Option<&ConversationViewModel>,
) -> bool {
    if parallel_mode_enabled {
        return false;
    }
    let Some(conversation) = conversation else {
        return false;
    };

    !conversation.has_active_thread()
        && conversation.messages.is_empty()
        && conversation.active_turn_id.is_none()
        && conversation.live_agent_message.is_none()
}

fn presentation_workspace_directory(app: &NativeTuiApp) -> Option<String> {
    match &app.conversation_state {
        ConversationState::Ready(conversation) => {
            Some(conversation.planning_workspace_directory().to_string())
        }
        ConversationState::Loading | ConversationState::Failed(_) => match &app.startup_state {
            StartupState::Ready(diagnostics) => Some(diagnostics.workspace_path.clone()),
            StartupState::Idle | StartupState::Loading | StartupState::Failed(_) => None,
        },
    }
}
