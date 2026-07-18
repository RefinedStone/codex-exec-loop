/*
 * ConversationProjectionSample captures the conversation shell's core snapshot
 * and render clocks once per terminal transaction. ConversationScreenModel
 * combines that owned sample with the UI-only facts needed by copy, layout,
 * cursor, and frame-cache code at one projection point.
 */
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use ratatui::text::Line;

use crate::application::service::planning::PlanningRuntimeProjection;
use crate::core::app::AppSnapshot;
use crate::domain::parallel_mode::{ParallelModeReadinessSnapshot, ParallelModeSupervisorSnapshot};

use super::super::parallel_presentation_bridge::{
    ParallelModePresentationLoadingStage, pending_parallel_mode_supervisor_snapshot,
};
use super::super::planning::PlanningWorkerPanelState;
use super::capability_projection::recent_session_status_label;
use super::{
    ConversationState, ConversationViewModel, InlineHistoryRenderMode, NativeTuiApp,
    ParallelPanelStateController, ShellActionAvailability, ShellOverlay, StartupState, TuiLanguage,
};

const MAX_GITHUB_REVIEW_NOTICE_LEN: usize = 160;

pub(in crate::adapter::inbound::tui::app) struct ConversationProjectionSample {
    core_snapshot: AppSnapshot,
    rendered_at: Instant,
    animation_elapsed_millis: u128,
}

impl ConversationProjectionSample {
    pub(in crate::adapter::inbound::tui::app) fn capture(app: &NativeTuiApp) -> Self {
        Self {
            core_snapshot: app.core_runtime.snapshot(),
            rendered_at: Instant::now(),
            animation_elapsed_millis: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_or(0, |duration| duration.as_millis()),
        }
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
        let core_snapshot = &sample.core_snapshot;
        let core_revision = core_snapshot.revision;
        let planning_runtime_projection =
            (*core_snapshot.planning_parallel.planning_runtime).clone();
        let workspace_directory = presentation_workspace_directory(app);
        let parallel_mode_enabled = app.parallel_mode_control_plane.mode_enabled();
        let parallel_mode_control_effect_in_flight =
            app.parallel_mode_control_plane.control_effect_in_flight();
        let parallel_mode_readiness = core_snapshot
            .planning_parallel
            .parallel_mode
            .readiness
            .as_deref()
            .filter(|snapshot| {
                workspace_directory
                    .as_deref()
                    .is_none_or(|workspace| snapshot.workspace_path == workspace)
            })
            .cloned();
        let current_parallel_mode_supervisor = core_snapshot
            .planning_parallel
            .parallel_mode
            .supervisor
            .as_deref()
            .filter(|snapshot| {
                workspace_directory
                    .as_deref()
                    .is_none_or(|workspace| snapshot.workspace_path == workspace)
            })
            .cloned();
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
            parallel_mode_loading_prompt_indicator_visible,
            parallel_mode_readiness,
            parallel_mode_supervisor,
            parallel_supervisor_event_lines: if parallel_mode_enabled {
                app.parallel_supervisor_event_lines()
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
            inline_history_render_mode: app.inline_history_render_mode,
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
