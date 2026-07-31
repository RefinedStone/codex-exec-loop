/*
 * ConversationProjectionSample captures the conversation shell's narrow Core
 * projection and render clocks once per terminal transaction.
 * ConversationScreenModel combines that owned sample with the UI-only facts
 * needed by copy, layout, cursor, and frame-cache code at one projection point.
 */
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use crate::application::service::parallel_mode::control_plane::{
    ParallelModeControlPlanePresentationProjection, ParallelModeGlobalRuntimeNoticeProjection,
};
use crate::application::service::planning::PlanningRuntimeProjection;
use crate::core::app::{
    AutoFollowPhase, ParallelModeProjection, PlanningParallelProjection,
    RevisionedPlanningParallelProjection,
};
use crate::domain::conversation::{
    ConversationMessage, ConversationMessageKind, ConversationTurnSteerRequest,
};
use crate::domain::parallel_mode::{ParallelModeReadinessSnapshot, ParallelModeSupervisorSnapshot};
use crate::domain::planning::PlanningWorkerPanelState;

use super::super::parallel_presentation_bridge::{
    ParallelModePresentationLoadingStage, pending_parallel_mode_supervisor_snapshot,
};
use super::super::parallel_supervisor_events::ParallelEventStreamSnapshot;
#[cfg(test)]
use super::NativeTuiApp;
use super::capability_projection::{
    recent_session_status_label, recent_session_status_requires_attention,
};
use super::{
    AutoFollowSnapshotPresentation, ConversationComposerState, ConversationInputState,
    ConversationState, ConversationViewModel, HistoryInsertionMode, InlineHistoryRenderMode,
    InlineShellCommandCapabilitySet, ParallelPanelStateController, ProgressiveActivityWaitStatus,
    SessionState, ShellActionAvailability, ShellOverlay, StartupState,
    TranscriptHandoffCorrelation, TuiLanguage,
};

pub(in crate::adapter::inbound::tui::app) const MAX_GITHUB_REVIEW_NOTICE_LEN: usize = 160;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::adapter::inbound::tui::app) struct ParallelPanelProjectionSample {
    parallel_control_plane: ParallelModeControlPlanePresentationProjection,
    parallel_mode: ParallelModeProjection,
}

impl ParallelPanelProjectionSample {
    #[cfg(test)]
    pub(in crate::adapter::inbound::tui::app) fn capture(app: &NativeTuiApp) -> Self {
        Self::from_parts(
            app.runtime.client_runtime.parallel_mode_projection(),
            app.runtime
                .client_runtime
                .parallel_control_plane_projection(),
        )
    }

    pub(in crate::adapter::inbound::tui::app) fn from_parts(
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

    pub(in crate::adapter::inbound::tui::app) fn global_runtime_notices(
        &self,
    ) -> &[ParallelModeGlobalRuntimeNoticeProjection] {
        &self.parallel_control_plane.global_runtime_notices
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

    pub(in crate::adapter::inbound::tui::app) fn active_parallel_agent_count_for_workspace(
        &self,
        workspace_directory: Option<&str>,
    ) -> usize {
        self.parallel_mode
            .supervisor
            .as_deref()
            .filter(|snapshot| {
                workspace_directory.is_none_or(|workspace| snapshot.workspace_path == workspace)
            })
            .map_or(0, |snapshot| {
                snapshot
                    .roster
                    .entries
                    .iter()
                    .filter(|entry| entry.counts_as_active())
                    .count()
            })
    }
}

pub(in crate::adapter::inbound::tui::app) struct ConversationProjectionSample {
    core_revision: u64,
    conversation_history_identity_revision: u64,
    transcript_handoff_correlation: Option<TranscriptHandoffCorrelation>,
    planning_runtime_workspace_directory: Option<String>,
    planning_runtime: Box<PlanningRuntimeProjection>,
    parallel_panel: ParallelPanelProjectionSample,
    parallel_supervisor_events: ParallelEventStreamSnapshot,
    inline_history_render_mode: InlineHistoryRenderMode,
    history_insert_mode: HistoryInsertionMode,
    rendered_at: Instant,
    animation_elapsed_millis: u128,
}

pub(in crate::adapter::inbound::tui::app) struct ConversationProjectionFrameInput {
    pub(in crate::adapter::inbound::tui::app) planning_parallel:
        RevisionedPlanningParallelProjection,
    pub(in crate::adapter::inbound::tui::app) parallel_control_plane:
        ParallelModeControlPlanePresentationProjection,
    pub(in crate::adapter::inbound::tui::app) conversation_history_identity_revision: u64,
    pub(in crate::adapter::inbound::tui::app) transcript_handoff_correlation:
        Option<TranscriptHandoffCorrelation>,
    pub(in crate::adapter::inbound::tui::app) parallel_supervisor_events:
        ParallelEventStreamSnapshot,
    pub(in crate::adapter::inbound::tui::app) inline_history_render_mode: InlineHistoryRenderMode,
    pub(in crate::adapter::inbound::tui::app) history_insert_mode: HistoryInsertionMode,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::adapter::inbound::tui::app) struct TranscriptHandoffDeliveryToken {
    conversation_history_identity_revision: u64,
    correlation: TranscriptHandoffCorrelation,
}

impl TranscriptHandoffDeliveryToken {
    pub(in crate::adapter::inbound::tui::app) fn from_sample(
        sample: &ConversationProjectionSample,
    ) -> Option<Self> {
        Some(Self {
            conversation_history_identity_revision: sample.conversation_history_identity_revision(),
            correlation: sample.transcript_handoff_correlation.clone()?,
        })
    }

    pub(in crate::adapter::inbound::tui::app) fn matches_current(
        &self,
        conversation_history_identity_revision: u64,
        transcript_handoff_correlation: Option<&TranscriptHandoffCorrelation>,
    ) -> bool {
        if self.conversation_history_identity_revision != conversation_history_identity_revision {
            return false;
        }
        transcript_handoff_correlation == Some(&self.correlation)
    }

    pub(in crate::adapter::inbound::tui::app) fn correlation(
        &self,
    ) -> &TranscriptHandoffCorrelation {
        &self.correlation
    }
}

impl ConversationProjectionSample {
    #[cfg(test)]
    pub(in crate::adapter::inbound::tui::app) fn capture(app: &NativeTuiApp) -> Self {
        Self::from_frame_input(ConversationProjectionFrameInput {
            planning_parallel: app
                .runtime
                .client_runtime
                .revisioned_planning_parallel_projection(),
            parallel_control_plane: app
                .runtime
                .client_runtime
                .parallel_control_plane_projection(),
            conversation_history_identity_revision: app
                .conversation
                .conversation_history_identity_revision,
            transcript_handoff_correlation: match &app.conversation.lifecycle.conversation_state {
                ConversationState::Ready(conversation) => {
                    conversation.viewport_transcript_handoff_correlation()
                }
                ConversationState::Loading | ConversationState::Failed(_) => None,
            },
            parallel_supervisor_events: app.shell.parallel_event_stream.snapshot(),
            inline_history_render_mode: app.shell.inline_history_render_mode,
            history_insert_mode: app.shell.history_insert_mode,
        })
    }

    pub(in crate::adapter::inbound::tui::app) fn from_frame_input(
        input: ConversationProjectionFrameInput,
    ) -> Self {
        let RevisionedPlanningParallelProjection {
            revision: core_revision,
            planning_parallel,
        } = input.planning_parallel;
        let PlanningParallelProjection {
            planning_runtime_workspace_directory,
            planning_runtime,
            parallel_mode,
        } = planning_parallel;
        Self {
            core_revision,
            conversation_history_identity_revision: input.conversation_history_identity_revision,
            transcript_handoff_correlation: input.transcript_handoff_correlation,
            planning_runtime_workspace_directory,
            planning_runtime,
            parallel_panel: ParallelPanelProjectionSample::from_parts(
                parallel_mode,
                input.parallel_control_plane,
            ),
            parallel_supervisor_events: input.parallel_supervisor_events,
            inline_history_render_mode: input.inline_history_render_mode,
            history_insert_mode: input.history_insert_mode,
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

    pub(in crate::adapter::inbound::tui::app) fn active_parallel_agent_count_for_workspace(
        &self,
        workspace_directory: Option<&str>,
    ) -> usize {
        self.parallel_panel
            .active_parallel_agent_count_for_workspace(workspace_directory)
    }

    pub(in crate::adapter::inbound::tui::app) fn last_parallel_mode_dispatch_withheld_reason(
        &self,
    ) -> Option<&str> {
        self.parallel_panel
            .last_parallel_mode_dispatch_withheld_reason()
    }

    pub(in crate::adapter::inbound::tui::app) fn global_runtime_notices(
        &self,
    ) -> &[ParallelModeGlobalRuntimeNoticeProjection] {
        self.parallel_panel.global_runtime_notices()
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

    pub(in crate::adapter::inbound::tui::app) fn parallel_event_stream_snapshot(
        &self,
    ) -> ParallelEventStreamSnapshot {
        self.parallel_supervisor_events.clone()
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
pub(in crate::adapter::inbound::tui::app) struct ConversationComposerScreenModel<'a> {
    pub(in crate::adapter::inbound::tui::app) state: &'a ConversationComposerState,
    pub(in crate::adapter::inbound::tui::app) input_state: ConversationInputState,
    pub(in crate::adapter::inbound::tui::app) post_turn_settlement_in_flight: bool,
    pub(in crate::adapter::inbound::tui::app) auto_follow_has_live_activity: bool,
    pub(in crate::adapter::inbound::tui::app) viewport_transcript_handoff_pending: bool,
}

impl<'a> ConversationComposerScreenModel<'a> {
    pub(in crate::adapter::inbound::tui::app) fn from_conversation(
        conversation: &'a ConversationViewModel,
    ) -> Self {
        Self {
            state: &conversation.composer,
            input_state: conversation.input_state(),
            post_turn_settlement_in_flight: conversation.has_post_turn_settlement_in_flight(),
            auto_follow_has_live_activity: conversation.auto_follow_state().has_live_activity(),
            viewport_transcript_handoff_pending: conversation
                .has_pending_viewport_transcript_handoff(),
        }
    }
}

#[derive(Debug, Clone)]
pub(in crate::adapter::inbound::tui::app) struct ConversationRuntimeStatusScreenModel {
    pub(in crate::adapter::inbound::tui::app) working_started_at: Option<Instant>,
    pub(in crate::adapter::inbound::tui::app) post_turn_settlement_in_flight: bool,
    pub(in crate::adapter::inbound::tui::app) auto_follow_phase: AutoFollowPhase,
    pub(in crate::adapter::inbound::tui::app) auto_follow_max_turns_label: String,
    pub(in crate::adapter::inbound::tui::app) input_state: ConversationInputState,
    pub(in crate::adapter::inbound::tui::app) live_agent_message_present: bool,
    pub(in crate::adapter::inbound::tui::app) wait_status: Option<ProgressiveActivityWaitStatus>,
    pub(in crate::adapter::inbound::tui::app) interrupt_support_label: &'static str,
}

impl ConversationRuntimeStatusScreenModel {
    fn from_conversation(conversation: &ConversationViewModel) -> Self {
        Self {
            working_started_at: conversation.live_activity_started_at(),
            post_turn_settlement_in_flight: conversation.has_post_turn_settlement_in_flight(),
            auto_follow_phase: conversation.auto_follow_state().phase.clone(),
            auto_follow_max_turns_label: conversation.auto_follow_state().max_auto_turns_label(),
            input_state: conversation.input_state(),
            live_agent_message_present: conversation.live_agent_message.is_some(),
            wait_status: conversation.progressive_activity_detail.wait_status(
                conversation.progressive_activity.retrying_summary(),
                conversation.pending_approval_request().is_some(),
            ),
            interrupt_support_label: conversation.interrupt_support_label(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::adapter::inbound::tui::app) struct ConversationLiveTranscriptScreenModel<'a> {
    pub(in crate::adapter::inbound::tui::app) handoff_messages: Option<&'a [ConversationMessage]>,
    pub(in crate::adapter::inbound::tui::app) buffered_tool_messages: &'a [ConversationMessage],
    pub(in crate::adapter::inbound::tui::app) live_agent_message: Option<&'a ConversationMessage>,
    pub(in crate::adapter::inbound::tui::app) recent_tail_messages:
        [Option<&'a ConversationMessage>; 2],
    pub(in crate::adapter::inbound::tui::app) handoff_pending: bool,
    pub(in crate::adapter::inbound::tui::app) acknowledge_handoff_after_successful_draw: bool,
}

impl<'a> ConversationLiveTranscriptScreenModel<'a> {
    fn from_conversation(
        conversation: &'a ConversationViewModel,
        inline_history_render_mode: InlineHistoryRenderMode,
        shell_overlay: ShellOverlay,
        dialog_visible: bool,
    ) -> Self {
        let release_handoff_messages = conversation.viewport_transcript_handoff_release_messages();
        let handoff_messages = conversation
            .viewport_transcript_handoff_messages()
            .or(release_handoff_messages);
        let handoff_pending = conversation.has_pending_viewport_transcript_handoff();
        Self {
            handoff_messages,
            buffered_tool_messages: conversation.buffered_tool_messages(),
            live_agent_message: conversation.live_agent_message.as_ref(),
            recent_tail_messages: if handoff_pending {
                [None, None]
            } else {
                Self::recent_tail_messages(conversation, inline_history_render_mode)
            },
            handoff_pending,
            acknowledge_handoff_after_successful_draw: shell_overlay == ShellOverlay::Hidden
                && !dialog_visible
                && matches!(
                    inline_history_render_mode,
                    InlineHistoryRenderMode::ViewportReplay
                )
                && release_handoff_messages.is_some(),
        }
    }

    fn recent_tail_messages(
        conversation: &'a ConversationViewModel,
        inline_history_render_mode: InlineHistoryRenderMode,
    ) -> [Option<&'a ConversationMessage>; 2] {
        if !inline_history_render_mode.mirrors_recent_transcript_in_tail() {
            return [None, None];
        }

        // Prefer the last two human-visible rows, in chronological display order.
        let mut messages = conversation.messages.iter().rev().filter(|message| {
            message.kind != ConversationMessageKind::Tool
                && message.kind != ConversationMessageKind::Status
        });
        let newest = messages.next();
        let previous = messages.next();
        if newest.is_some() {
            return [previous, newest];
        }

        // Status is useful fallback context before any user or agent message exists.
        let mut messages = conversation
            .messages
            .iter()
            .rev()
            .filter(|message| message.kind != ConversationMessageKind::Tool);
        let newest = messages.next();
        let previous = messages.next();
        [previous, newest]
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::adapter::inbound::tui::app) enum QueueMutationTailState {
    Idle,
    Pending(u64),
    RefreshRequired,
    UndoAvailable(usize),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::adapter::inbound::tui::app) struct TurnSteerConfirmationScreenModel {
    pub(in crate::adapter::inbound::tui::app) language: TuiLanguage,
    pub(in crate::adapter::inbound::tui::app) request: ConversationTurnSteerRequest,
}

pub(in crate::adapter::inbound::tui::app) struct ConversationScreenFrameInput<'a> {
    pub(in crate::adapter::inbound::tui::app) startup_state: &'a StartupState,
    pub(in crate::adapter::inbound::tui::app) session_state: &'a SessionState,
    pub(in crate::adapter::inbound::tui::app) can_open_session_list: bool,
    pub(in crate::adapter::inbound::tui::app) shell_action_availability: ShellActionAvailability,
    pub(in crate::adapter::inbound::tui::app) inline_shell_command_capabilities:
        InlineShellCommandCapabilitySet,
    pub(in crate::adapter::inbound::tui::app) github_review_polling_status_label: String,
    pub(in crate::adapter::inbound::tui::app) github_review_recent_changes_summary: Option<String>,
    pub(in crate::adapter::inbound::tui::app) tui_language: TuiLanguage,
    pub(in crate::adapter::inbound::tui::app) planning_worker_shows_debug_details: bool,
    pub(in crate::adapter::inbound::tui::app) planning_worker_panel_state: PlanningWorkerPanelState,
    pub(in crate::adapter::inbound::tui::app) queue_mutation_tail_state: QueueMutationTailState,
    pub(in crate::adapter::inbound::tui::app) workspace_directory: Option<String>,
    pub(in crate::adapter::inbound::tui::app) turn_options_hud_label: String,
    pub(in crate::adapter::inbound::tui::app) turn_options_summary: Option<String>,
    pub(in crate::adapter::inbound::tui::app) shell_overlay: ShellOverlay,
    pub(in crate::adapter::inbound::tui::app) exit_confirmation_visible: bool,
    pub(in crate::adapter::inbound::tui::app) turn_steer_confirmation:
        Option<TurnSteerConfirmationScreenModel>,
    pub(in crate::adapter::inbound::tui::app) conversation_state: ShellConversationState<'a>,
}

pub(in crate::adapter::inbound::tui::app) struct ConversationScreenModel<'a> {
    pub(in crate::adapter::inbound::tui::app) core_revision: u64,
    transcript_handoff_delivery_token: Option<TranscriptHandoffDeliveryToken>,
    pub(in crate::adapter::inbound::tui::app) rendered_at: Instant,
    pub(in crate::adapter::inbound::tui::app) animation_elapsed_millis: u128,
    pub(in crate::adapter::inbound::tui::app) startup_state: &'a StartupState,
    pub(in crate::adapter::inbound::tui::app) shell_action_availability: ShellActionAvailability,
    pub(in crate::adapter::inbound::tui::app) inline_shell_command_capabilities:
        InlineShellCommandCapabilitySet,
    pub(in crate::adapter::inbound::tui::app) recent_session_status_label: String,
    pub(in crate::adapter::inbound::tui::app) recent_session_status_requires_attention: bool,
    pub(in crate::adapter::inbound::tui::app) github_review_polling_status_label: String,
    pub(in crate::adapter::inbound::tui::app) github_review_recent_changes_summary: Option<String>,
    pub(in crate::adapter::inbound::tui::app) tui_language: TuiLanguage,
    pub(in crate::adapter::inbound::tui::app) parallel_mode_enabled: bool,
    pub(in crate::adapter::inbound::tui::app) parallel_mode_control_effect_in_flight: bool,
    pub(in crate::adapter::inbound::tui::app) last_parallel_mode_dispatch_withheld_reason:
        Option<String>,
    pub(in crate::adapter::inbound::tui::app) global_runtime_notices: Vec<String>,
    pub(in crate::adapter::inbound::tui::app) parallel_mode_loading_prompt_indicator_visible: bool,
    pub(in crate::adapter::inbound::tui::app) parallel_mode_readiness:
        Option<ParallelModeReadinessSnapshot>,
    pub(in crate::adapter::inbound::tui::app) parallel_mode_supervisor:
        ParallelModeSupervisorSnapshot,
    pub(in crate::adapter::inbound::tui::app) parallel_event_stream_snapshot:
        ParallelEventStreamSnapshot,
    pub(in crate::adapter::inbound::tui::app) planning_runtime_projection:
        PlanningRuntimeProjection,
    pub(in crate::adapter::inbound::tui::app) planning_worker_shows_debug_details: bool,
    pub(in crate::adapter::inbound::tui::app) planning_worker_panel_state: PlanningWorkerPanelState,
    pub(in crate::adapter::inbound::tui::app) queue_mutation_tail_state: QueueMutationTailState,
    pub(in crate::adapter::inbound::tui::app) workspace_directory: String,
    pub(in crate::adapter::inbound::tui::app) turn_options_hud_label: String,
    pub(in crate::adapter::inbound::tui::app) context_pressure_basis_points: Option<u16>,
    pub(in crate::adapter::inbound::tui::app) turn_options_summary: Option<String>,
    pub(in crate::adapter::inbound::tui::app) shell_overlay: ShellOverlay,
    pub(in crate::adapter::inbound::tui::app) inline_history_render_mode: InlineHistoryRenderMode,
    pub(in crate::adapter::inbound::tui::app) exit_confirmation_visible: bool,
    pub(in crate::adapter::inbound::tui::app) turn_steer_confirmation:
        Option<TurnSteerConfirmationScreenModel>,
    pub(in crate::adapter::inbound::tui::app) prompt_input_has_focus: bool,
    composer: Option<ConversationComposerScreenModel<'a>>,
    runtime_status: Option<ConversationRuntimeStatusScreenModel>,
    live_transcript: Option<ConversationLiveTranscriptScreenModel<'a>>,
    pub(in crate::adapter::inbound::tui::app) conversation_state: ShellConversationState<'a>,
}

impl<'a> ConversationScreenModel<'a> {
    #[cfg(test)]
    pub(in crate::adapter::inbound::tui::app) fn from_app(app: &'a NativeTuiApp) -> Self {
        let sample = ConversationProjectionSample::capture(app);
        Self::from_app_with_sample(app, &sample)
    }

    #[cfg(test)]
    pub(in crate::adapter::inbound::tui::app) fn from_app_with_sample(
        app: &'a NativeTuiApp,
        sample: &ConversationProjectionSample,
    ) -> Self {
        let parallel_mode_enabled = sample.parallel_mode_enabled();
        let conversation_state =
            shell_conversation_state(&app.conversation.lifecycle.conversation_state);
        let workspace_directory =
            presentation_workspace_directory(conversation_state, &app.shell.chrome.startup_state);
        let inline_shell_command_capabilities = app.inline_shell_command_capabilities(
            sample.parallel_mode_enabled(),
            sample.parallel_mode_control_effect_in_flight(),
            sample.active_parallel_agent_count_for_workspace(workspace_directory.as_deref()),
        );
        let exit_confirmation_visible = app.is_exit_confirmation_visible();
        let turn_steer_confirmation = app.is_turn_steer_confirmation_visible().then(|| {
            let intent = app
                .conversation
                .turn_steer_confirmation
                .as_ref()
                .expect("visible turn-steer confirmation must retain its intent");
            TurnSteerConfirmationScreenModel {
                language: app.shell.tui_language,
                request: intent.request.clone(),
            }
        });
        let queue_mutation_tail_state =
            if let Some(operation_id) = app.pending_queue_mutation_operation_id() {
                QueueMutationTailState::Pending(operation_id)
            } else if app.queue_mutation_requires_authority_refresh() {
                QueueMutationTailState::RefreshRequired
            } else if let Some(task_count) =
                app.queue_receipt_undo_task_count_for_parallel_mode(parallel_mode_enabled)
            {
                QueueMutationTailState::UndoAvailable(task_count)
            } else {
                QueueMutationTailState::Idle
            };
        Self::from_screen_frame_input(
            ConversationScreenFrameInput {
                startup_state: &app.shell.chrome.startup_state,
                session_state: &app.shell.chrome.session_state,
                can_open_session_list: app.can_open_session_list(),
                shell_action_availability: app.shell_action_availability(),
                inline_shell_command_capabilities,
                github_review_polling_status_label: app.github_review_polling_status_label(),
                github_review_recent_changes_summary: app
                    .github_review_recent_changes_summary(MAX_GITHUB_REVIEW_NOTICE_LEN),
                tui_language: app.shell.tui_language,
                planning_worker_shows_debug_details: app.planning_worker_shows_debug_details(),
                planning_worker_panel_state: app
                    .planning
                    .planning_worker_panel_state
                    .current()
                    .clone(),
                queue_mutation_tail_state,
                workspace_directory,
                turn_options_hud_label: app.conversation.turn_options.summary_label(),
                turn_options_summary: (!app.conversation.turn_options.is_default())
                    .then(|| app.conversation.turn_options.summary_label()),
                shell_overlay: app.shell.chrome.shell_overlay,
                exit_confirmation_visible,
                turn_steer_confirmation,
                conversation_state,
            },
            sample,
        )
    }

    pub(in crate::adapter::inbound::tui::app) fn from_screen_frame_input(
        input: ConversationScreenFrameInput<'a>,
        sample: &ConversationProjectionSample,
    ) -> Self {
        let core_revision = sample.core_revision;
        let workspace_directory = input.workspace_directory.clone();
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
        let parallel_panel_visible = input.shell_overlay == ShellOverlay::Supersession
            || (input.shell_overlay == ShellOverlay::Hidden && parallel_mode_enabled);
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
        let exit_confirmation_visible = input.exit_confirmation_visible;
        let turn_steer_confirmation = input.turn_steer_confirmation;
        let dialog_visible = exit_confirmation_visible || turn_steer_confirmation.is_some();
        let prompt_input_has_focus = input
            .shell_overlay
            .prompt_input_has_focus(dialog_visible, parallel_mode_prompt_input_locked);
        let queue_mutation_tail_state = input.queue_mutation_tail_state;
        let inline_history_render_mode = sample.inline_history_render_mode();
        let conversation_state = input.conversation_state;
        let composer = Self::composer_for_state(conversation_state);
        let runtime_status = Self::runtime_status_for_state(conversation_state);
        let live_transcript = Self::live_transcript_for_state(
            conversation_state,
            inline_history_render_mode,
            input.shell_overlay,
            dialog_visible,
        );

        Self {
            core_revision,
            transcript_handoff_delivery_token: TranscriptHandoffDeliveryToken::from_sample(sample),
            rendered_at: sample.rendered_at,
            animation_elapsed_millis: sample.animation_elapsed_millis,
            startup_state: input.startup_state,
            shell_action_availability: input.shell_action_availability,
            inline_shell_command_capabilities: input.inline_shell_command_capabilities,
            recent_session_status_label: recent_session_status_label(
                input.can_open_session_list,
                input.startup_state,
                input.session_state,
                input.tui_language,
            ),
            recent_session_status_requires_attention: recent_session_status_requires_attention(
                input.session_state,
            ),
            github_review_polling_status_label: input.github_review_polling_status_label,
            github_review_recent_changes_summary: input.github_review_recent_changes_summary,
            tui_language: input.tui_language,
            parallel_mode_enabled,
            parallel_mode_control_effect_in_flight,
            last_parallel_mode_dispatch_withheld_reason: sample
                .last_parallel_mode_dispatch_withheld_reason()
                .map(str::to_string),
            global_runtime_notices: sample
                .global_runtime_notices()
                .iter()
                .map(|notice| notice.notice.clone())
                .collect(),
            parallel_mode_loading_prompt_indicator_visible,
            parallel_mode_readiness,
            parallel_mode_supervisor,
            parallel_event_stream_snapshot: sample.parallel_event_stream_snapshot(),
            planning_runtime_projection,
            planning_worker_shows_debug_details: input.planning_worker_shows_debug_details,
            planning_worker_panel_state: input.planning_worker_panel_state,
            queue_mutation_tail_state,
            workspace_directory: presentation_workspace_directory,
            turn_options_hud_label: input.turn_options_hud_label,
            context_pressure_basis_points: match conversation_state {
                ShellConversationState::Ready(conversation) => conversation
                    .progressive_activity
                    .context_pressure_basis_points(),
                ShellConversationState::Loading | ShellConversationState::Failed(_) => None,
            },
            turn_options_summary: input.turn_options_summary,
            shell_overlay: input.shell_overlay,
            inline_history_render_mode,
            exit_confirmation_visible,
            turn_steer_confirmation,
            prompt_input_has_focus,
            composer,
            runtime_status,
            live_transcript,
            conversation_state,
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

    pub(in crate::adapter::inbound::tui::app) fn composer(
        &self,
    ) -> Option<&ConversationComposerScreenModel<'a>> {
        match (self.conversation_state, self.composer.as_ref()) {
            (ShellConversationState::Ready(_), Some(composer)) => Some(composer),
            (ShellConversationState::Loading | ShellConversationState::Failed(_), None) => None,
            _ => unreachable!("conversation and composer projections must agree"),
        }
    }

    fn composer_for_state(
        conversation_state: ShellConversationState<'a>,
    ) -> Option<ConversationComposerScreenModel<'a>> {
        match conversation_state {
            ShellConversationState::Ready(conversation) => Some(
                ConversationComposerScreenModel::from_conversation(conversation),
            ),
            ShellConversationState::Loading | ShellConversationState::Failed(_) => None,
        }
    }

    pub(in crate::adapter::inbound::tui::app) fn runtime_status(
        &self,
    ) -> Option<&ConversationRuntimeStatusScreenModel> {
        match (self.conversation_state, self.runtime_status.as_ref()) {
            (ShellConversationState::Ready(_), Some(runtime_status)) => Some(runtime_status),
            (ShellConversationState::Loading | ShellConversationState::Failed(_), None) => None,
            _ => unreachable!("conversation and runtime status projections must agree"),
        }
    }

    fn runtime_status_for_state(
        conversation_state: ShellConversationState<'a>,
    ) -> Option<ConversationRuntimeStatusScreenModel> {
        match conversation_state {
            ShellConversationState::Ready(conversation) => Some(
                ConversationRuntimeStatusScreenModel::from_conversation(conversation),
            ),
            ShellConversationState::Loading | ShellConversationState::Failed(_) => None,
        }
    }

    pub(in crate::adapter::inbound::tui::app) fn live_transcript(
        &self,
    ) -> Option<&ConversationLiveTranscriptScreenModel<'a>> {
        match (self.conversation_state, self.live_transcript.as_ref()) {
            (ShellConversationState::Ready(_), Some(live_transcript)) => Some(live_transcript),
            (ShellConversationState::Loading | ShellConversationState::Failed(_), None) => None,
            _ => unreachable!("conversation and live transcript projections must agree"),
        }
    }

    fn live_transcript_for_state(
        conversation_state: ShellConversationState<'a>,
        inline_history_render_mode: InlineHistoryRenderMode,
        shell_overlay: ShellOverlay,
        dialog_visible: bool,
    ) -> Option<ConversationLiveTranscriptScreenModel<'a>> {
        match conversation_state {
            ShellConversationState::Ready(conversation) => {
                Some(ConversationLiveTranscriptScreenModel::from_conversation(
                    conversation,
                    inline_history_render_mode,
                    shell_overlay,
                    dialog_visible,
                ))
            }
            ShellConversationState::Loading | ShellConversationState::Failed(_) => None,
        }
    }

    pub(in crate::adapter::inbound::tui::app) fn startup_screen_is_active(&self) -> bool {
        conversation_startup_screen_is_active(self.parallel_mode_enabled, self.ready_conversation())
    }

    pub(in crate::adapter::inbound::tui::app) fn dialog_visible(&self) -> bool {
        self.exit_confirmation_visible || self.turn_steer_confirmation.is_some()
    }

    pub(in crate::adapter::inbound::tui::app) fn renders_viewport_transcript_handoff(
        &self,
    ) -> bool {
        self.live_transcript().is_some_and(|live_transcript| {
            live_transcript.acknowledge_handoff_after_successful_draw
        })
    }

    pub(in crate::adapter::inbound::tui::app) fn transcript_handoff_delivery_token(
        &self,
    ) -> Option<TranscriptHandoffDeliveryToken> {
        if !self.renders_viewport_transcript_handoff() {
            return None;
        }
        self.transcript_handoff_delivery_token.clone()
    }

    pub(in crate::adapter::inbound::tui::app) fn renders_parallel_viewport_handoff(&self) -> bool {
        self.parallel_mode_enabled && self.renders_viewport_transcript_handoff()
    }

    #[cfg(test)]
    pub(in crate::adapter::inbound::tui::app) fn from_test_parts(
        startup_state: &'a StartupState,
        shell_action_availability: ShellActionAvailability,
        conversation_state: ShellConversationState<'a>,
    ) -> Self {
        let composer = Self::composer_for_state(conversation_state);
        let runtime_status = Self::runtime_status_for_state(conversation_state);
        let live_transcript = Self::live_transcript_for_state(
            conversation_state,
            InlineHistoryRenderMode::HostScrollback,
            ShellOverlay::Hidden,
            false,
        );
        Self {
            core_revision: 0,
            transcript_handoff_delivery_token: None,
            rendered_at: Instant::now(),
            animation_elapsed_millis: 0,
            startup_state,
            shell_action_availability,
            inline_shell_command_capabilities: InlineShellCommandCapabilitySet::default(),
            recent_session_status_label: "loaded".to_string(),
            recent_session_status_requires_attention: false,
            github_review_polling_status_label: "polling".to_string(),
            github_review_recent_changes_summary: None,
            tui_language: TuiLanguage::English,
            parallel_mode_enabled: false,
            parallel_mode_control_effect_in_flight: false,
            last_parallel_mode_dispatch_withheld_reason: None,
            global_runtime_notices: Vec::new(),
            parallel_mode_loading_prompt_indicator_visible: false,
            parallel_mode_readiness: None,
            parallel_mode_supervisor: pending_parallel_mode_supervisor_snapshot(
                ".",
                false,
                None,
                ParallelModePresentationLoadingStage::Entering,
            ),
            parallel_event_stream_snapshot: ParallelEventStreamSnapshot::default(),
            planning_runtime_projection: PlanningRuntimeProjection::uninitialized(),
            planning_worker_shows_debug_details: false,
            planning_worker_panel_state: PlanningWorkerPanelState::default(),
            queue_mutation_tail_state: QueueMutationTailState::Idle,
            workspace_directory: ".".to_string(),
            turn_options_hud_label: "model: default  |  think: default".to_string(),
            context_pressure_basis_points: None,
            turn_options_summary: None,
            shell_overlay: ShellOverlay::Hidden,
            inline_history_render_mode: InlineHistoryRenderMode::HostScrollback,
            exit_confirmation_visible: false,
            turn_steer_confirmation: None,
            prompt_input_has_focus: true,
            composer,
            runtime_status,
            live_transcript,
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
        && conversation.active_turn_id().is_none()
        && conversation.live_agent_message.is_none()
}

pub(in crate::adapter::inbound::tui::app) fn shell_conversation_state(
    conversation_state: &ConversationState,
) -> ShellConversationState<'_> {
    match conversation_state {
        ConversationState::Loading => ShellConversationState::Loading,
        ConversationState::Failed(message) => ShellConversationState::Failed(message),
        ConversationState::Ready(conversation) => ShellConversationState::Ready(conversation),
    }
}

pub(in crate::adapter::inbound::tui::app) fn presentation_workspace_directory(
    conversation_state: ShellConversationState<'_>,
    startup_state: &StartupState,
) -> Option<String> {
    match conversation_state {
        ShellConversationState::Ready(conversation) => {
            Some(conversation.planning_workspace_directory().to_string())
        }
        ShellConversationState::Loading | ShellConversationState::Failed(_) => {
            match startup_state {
                StartupState::Ready(diagnostics) => Some(diagnostics.workspace_path.clone()),
                StartupState::Idle | StartupState::Loading | StartupState::Failed(_) => None,
            }
        }
    }
}
