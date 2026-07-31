use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::text::Line;
use ratatui::widgets::ListState;

use crate::application::service::planning::PlanningRuntimeProjection;
use crate::domain::parallel_mode::ParallelModeSupervisorSnapshot;

use super::parallel_supervisor_events::ParallelEventStreamSnapshot;
use super::parallel_terminal_delivery::ParallelLiveStreamModel;
use super::shell_presentation::{
    ActivityOverlayDocument, ActivityOverlayView, ConversationProjectionSample,
    ConversationScreenFrameInput, ConversationScreenModel, DirectionsMaintenanceFrameInput,
    DirectionsMaintenanceOverlayView, HelpOverlayView, InlineTailView, LanguageSelectionFrameInput,
    LanguageSelectionOverlayView, MAX_GITHUB_REVIEW_NOTICE_LEN, ModelSelectionFrameInput,
    ModelSelectionOverlayView, ParallelPeekOverlayView, PlanningDraftEditorOverlayView,
    PlanningInitOverlayFrameInput, PlanningInitOverlayView, QueueMutationTailState,
    QueueOverlayView, ReviewsOverlayView, SessionOverlayView, StartupBannerFrameInput,
    StartupOverlayFrameInput, StartupOverlayView, SupersessionOverlayView,
    TranscriptHandoffDeliveryToken, TurnSteerConfirmationScreenModel, ViewSelectionFrameInput,
    ViewSelectionOverlayView, WorkCenterOverlayView, build_activity_overlay_list_view,
    build_directions_maintenance_overlay_view, build_help_overlay_view,
    build_inline_live_transcript_lines, build_inline_tail_view,
    build_language_selection_overlay_view, build_model_selection_overlay_view,
    build_operator_diagnostic_lines, build_parallel_peek_overlay_view_from_snapshot,
    build_planning_draft_editor_overlay_view_from_state,
    build_planning_init_overlay_view_from_projection, build_queue_overlay_view_from_screen_model,
    build_reviews_overlay_view, build_session_overlay_view, build_startup_banner_lines,
    build_startup_overlay_view, build_supersession_overlay_view, build_view_selection_overlay_view,
    build_work_center_overlay_view, format_conversation_scrollback_lines_with_expand,
    presentation_workspace_directory, shell_conversation_state,
};
use super::shell_rendering::{
    count_rendered_inline_rows, inline_frame_inspection_area, inline_parallel_event_stream_area,
    inline_section_height,
};
use super::*;

/*
 * Terminal delivery owns one fully materialized inline frame. This module is the
 * only production boundary allowed to sample NativeTuiApp for that frame. The
 * rendering modules consume the owned models below and return a delivery receipt;
 * they never reread or mutate the aggregate while terminal I/O is in progress.
 */
fn capture_conversation_screen_frame_input<'a>(
    app: &'a NativeTuiApp,
    sample: &ConversationProjectionSample,
) -> ConversationScreenFrameInput<'a> {
    let conversation_state =
        shell_conversation_state(&app.conversation.lifecycle.conversation_state);
    let workspace_directory =
        presentation_workspace_directory(conversation_state, &app.shell.chrome.startup_state);
    let inline_shell_command_capabilities = app.inline_shell_command_capabilities(
        sample.parallel_mode_enabled(),
        sample.parallel_mode_control_effect_in_flight(),
        sample.active_parallel_agent_count_for_workspace(workspace_directory.as_deref()),
    );
    let parallel_mode_enabled = sample.parallel_mode_enabled();
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
        planning_worker_panel_state: app.planning.planning_worker_panel_state.current().clone(),
        queue_mutation_tail_state,
        workspace_directory,
        turn_options_hud_label: app.conversation.turn_options.summary_label(),
        turn_options_summary: (!app.conversation.turn_options.is_default())
            .then(|| app.conversation.turn_options.summary_label()),
        shell_overlay: app.shell.chrome.shell_overlay,
        exit_confirmation_visible,
        turn_steer_confirmation,
        conversation_state,
    }
}

fn capture_session_overlay_screen_model(app: &NativeTuiApp) -> SessionOverlayScreenModel {
    SessionOverlayScreenModel::from_frame_input(
        app.can_open_session_list(),
        app.current_workspace_directory(),
        app.shell.tui_language,
        &app.shell.chrome.session_state,
        &app.shell.session_overlay_ui_state,
        app.shell.chrome.selected_session_index,
    )
}

pub(super) struct InlineConversationFrameProjection {
    pub(super) core_revision: u64,
    pub(super) rendered_at_epoch_millis: Option<i64>,
    pub(super) tail_view: InlineTailView,
    pub(super) live_transcript_lines: Vec<Line<'static>>,
    pub(super) shell_overlay: ShellOverlay,
    pub(super) tui_language: TuiLanguage,
    pub(super) inline_history_render_mode: InlineHistoryRenderMode,
    pub(super) parallel_mode_enabled: bool,
    pub(super) renders_viewport_transcript_handoff: bool,
    pub(super) transcript_handoff_delivery_token: Option<Box<TranscriptHandoffDeliveryToken>>,
    pub(super) renders_parallel_viewport_handoff: bool,
    pub(super) exit_confirmation_visible: bool,
    pub(super) turn_steer_confirmation: Option<Box<TurnSteerConfirmationScreenModel>>,
    pub(super) supersession_overlay_view: Option<Box<SupersessionOverlayView>>,
    pub(super) work_center_overlay_view: Option<Box<WorkCenterOverlayView>>,
    operator_diagnostic_lines: Vec<Line<'static>>,
    sampled_parallel_supervisor: Box<ParallelModeSupervisorSnapshot>,
    sampled_planning_runtime_projection: Box<PlanningRuntimeProjection>,
}

pub(super) struct ParallelConversationHandoffProjection {
    pub(super) lines: Vec<Line<'static>>,
    pub(super) delivery_token: TranscriptHandoffDeliveryToken,
}

pub(super) struct InlineTerminalSyncProjection {
    pub(super) sampled_parallel_frame_projection: Option<InlineConversationFrameProjection>,
    pub(super) parallel_handoff_conversation_lines: Option<ParallelConversationHandoffProjection>,
    pub(super) current_history_projection: Option<Vec<Line<'static>>>,
    pub(super) parallel_event_stream_snapshot: Option<ParallelEventStreamSnapshot>,
}

pub(super) fn capture_inline_terminal_sync_projection(
    app: &NativeTuiApp,
    viewport_area: Rect,
    sample: &ConversationProjectionSample,
) -> InlineTerminalSyncProjection {
    let sampled_parallel_frame_projection = sample.parallel_mode_enabled().then(|| {
        InlineConversationFrameProjection::from_app_with_sample(app, viewport_area.width, sample)
    });
    let parallel_handoff_conversation_lines = (sample.parallel_mode_enabled()
        && sample.inline_history_render_mode().writes_host_scrollback())
    .then(|| capture_parallel_conversation_handoff_projection(app, sample))
    .flatten();
    let current_history_projection = current_inline_history_lines_for_viewport(app, sample);
    let parallel_event_stream_snapshot = sample
        .parallel_mode_enabled()
        .then(|| sample.parallel_event_stream_snapshot());
    InlineTerminalSyncProjection {
        sampled_parallel_frame_projection,
        parallel_handoff_conversation_lines,
        current_history_projection,
        parallel_event_stream_snapshot,
    }
}

pub(super) fn capture_parallel_conversation_handoff_projection(
    app: &NativeTuiApp,
    sample: &ConversationProjectionSample,
) -> Option<ParallelConversationHandoffProjection> {
    let ConversationState::Ready(conversation) = &app.conversation.lifecycle.conversation_state
    else {
        return None;
    };
    conversation.viewport_transcript_handoff_release_messages()?;
    Some(ParallelConversationHandoffProjection {
        lines: format_conversation_scrollback_lines_with_expand(
            conversation.host_scrollback_messages(),
            app.conversation.conversation_view_mode,
            app.conversation
                .conversation_view_mode
                .shows_debug_details()
                || app.planning_worker_shows_debug_details(),
            Some(
                app.shell
                    .progressive_activity_overlay_ui_state
                    .expand_state(),
            ),
        ),
        delivery_token: TranscriptHandoffDeliveryToken::from_sample(sample)?,
    })
}

fn current_inline_history_lines_for_viewport(
    app: &NativeTuiApp,
    sample: &ConversationProjectionSample,
) -> Option<Vec<Line<'static>>> {
    if sample.parallel_mode_enabled() {
        // Parallel delivery has its own typed cursor and must not participate in
        // the transcript rendered-line baseline.
        return None;
    }
    let startup_conversation = match &app.conversation.lifecycle.conversation_state {
        ConversationState::Ready(conversation) => Some(conversation.as_ref()),
        ConversationState::Loading | ConversationState::Failed(_) => None,
    };
    if let Some(startup_banner_lines) = build_startup_banner_lines(
        StartupBannerFrameInput {
            show_startup_ascii_art: app.shell.show_startup_ascii_art,
            parallel_mode_enabled: sample.parallel_mode_enabled(),
            conversation: startup_conversation,
        },
        None,
    ) {
        return Some(startup_banner_lines);
    }
    match &app.conversation.lifecycle.conversation_state {
        ConversationState::Ready(conversation) => {
            let messages = conversation.host_scrollback_messages();
            if messages.is_empty()
                && conversation
                    .viewport_transcript_handoff_messages()
                    .is_some()
            {
                return Some(Vec::new());
            }
            Some(format_conversation_scrollback_lines_with_expand(
                messages,
                app.conversation.conversation_view_mode,
                app.conversation
                    .conversation_view_mode
                    .shows_debug_details()
                    || app.planning_worker_shows_debug_details(),
                Some(
                    app.shell
                        .progressive_activity_overlay_ui_state
                        .expand_state(),
                ),
            ))
        }
        // Loading/failure retain the prior host-scrollback diff baseline.
        ConversationState::Loading | ConversationState::Failed(_) => None,
    }
}

impl InlineConversationFrameProjection {
    #[cfg(test)]
    pub(super) fn from_app(app: &NativeTuiApp, content_width: u16) -> Self {
        let sample = ConversationProjectionSample::capture(app);
        Self::from_app_with_sample(app, content_width, &sample)
    }

    pub(super) fn from_app_with_sample(
        app: &NativeTuiApp,
        content_width: u16,
        sample: &ConversationProjectionSample,
    ) -> Self {
        let screen_model = ConversationScreenModel::from_screen_frame_input(
            capture_conversation_screen_frame_input(app, sample),
            sample,
        );
        let supersession_overlay_view = (screen_model.shell_overlay == ShellOverlay::Supersession
            || (screen_model.shell_overlay == ShellOverlay::Hidden
                && screen_model.parallel_mode_enabled))
            .then(|| {
                Box::new(build_supersession_overlay_view(
                    &screen_model,
                    &app.shell.supersession_mud_ui_state,
                ))
            });
        let work_center_overlay_view = (screen_model.shell_overlay == ShellOverlay::WorkCenter)
            .then(|| {
                Box::new(build_work_center_overlay_view(
                    &screen_model,
                    app.shell.work_center_overlay_ui_state.selected_section(),
                    content_width,
                ))
            });
        Self::from_screen_model(
            screen_model,
            content_width,
            supersession_overlay_view,
            work_center_overlay_view,
        )
    }

    fn from_screen_model(
        screen_model: ConversationScreenModel<'_>,
        content_width: u16,
        supersession_overlay_view: Option<Box<SupersessionOverlayView>>,
        work_center_overlay_view: Option<Box<WorkCenterOverlayView>>,
    ) -> Self {
        let operator_diagnostic_lines = build_operator_diagnostic_lines(&screen_model);
        let tail_view = build_inline_tail_view(&screen_model, content_width);
        let live_transcript_lines = build_inline_live_transcript_lines(&screen_model);
        let renders_viewport_transcript_handoff =
            screen_model.renders_viewport_transcript_handoff();
        let transcript_handoff_delivery_token = screen_model
            .transcript_handoff_delivery_token()
            .map(Box::new);
        let renders_parallel_viewport_handoff = screen_model.renders_parallel_viewport_handoff();
        let sampled_parallel_supervisor = Box::new(screen_model.parallel_mode_supervisor);
        let sampled_planning_runtime_projection =
            Box::new(screen_model.planning_runtime_projection);
        Self {
            core_revision: screen_model.core_revision,
            rendered_at_epoch_millis: i64::try_from(screen_model.animation_elapsed_millis).ok(),
            tail_view,
            live_transcript_lines,
            shell_overlay: screen_model.shell_overlay,
            tui_language: screen_model.tui_language,
            inline_history_render_mode: screen_model.inline_history_render_mode,
            parallel_mode_enabled: screen_model.parallel_mode_enabled,
            renders_viewport_transcript_handoff,
            transcript_handoff_delivery_token,
            renders_parallel_viewport_handoff,
            exit_confirmation_visible: screen_model.exit_confirmation_visible,
            turn_steer_confirmation: screen_model.turn_steer_confirmation.map(Box::new),
            supersession_overlay_view,
            work_center_overlay_view,
            operator_diagnostic_lines,
            sampled_parallel_supervisor,
            sampled_planning_runtime_projection,
        }
    }

    pub(super) fn parallel_live_stream(&self) -> Option<&ParallelLiveStreamModel> {
        self.supersession_overlay_view
            .as_deref()
            .map(|view| &view.event_stream)
    }

    pub(super) fn install_parallel_live_stream(&mut self, live_stream: ParallelLiveStreamModel) {
        if let Some(view) = self.supersession_overlay_view.as_deref_mut() {
            view.event_stream = live_stream;
        }
    }

    fn finalize_parallel_live_stream_geometry(&mut self, event_area: Rect) {
        if let Some(view) = self.supersession_overlay_view.as_deref_mut() {
            view.event_stream.finalize_pending_geometry(event_area);
        }
    }
}

pub(super) struct InlineShellFrameModel {
    pub(super) conversation: InlineConversationFrameProjection,
    pub(super) inspection: InlineInspectionFrameModel,
    receipt: InlineFrameRenderReceipt,
}

pub(super) enum InlineInspectionFrameModel {
    Conversation,
    ParallelSupervisor(SupersessionOverlayView),
    Startup {
        view: StartupOverlayView,
        warning_scroll_offset: u16,
    },
    Sessions {
        view: SessionOverlayView,
        list_state: ListState,
    },
    ModelSelection(ModelSelectionOverlayView),
    ViewSelection(ViewSelectionOverlayView),
    LanguageSelection(LanguageSelectionOverlayView),
    Supersession(SupersessionOverlayView),
    WorkCenter(WorkCenterOverlayView),
    ParallelPeek {
        view: ParallelPeekOverlayView,
        step: ParallelPeekOverlayStep,
        scroll_from_bottom: usize,
    },
    Activity(ActivityOverlayView),
    Help {
        language: TuiLanguage,
        view: HelpOverlayView,
        scroll_offset: u16,
    },
    Reviews(ReviewsOverlayView),
    Queue(QueueOverlayView),
    Directions(DirectionsMaintenanceOverlayView),
    PlanningInit(PlanningInitOverlayView),
    DraftEditor {
        title: &'static str,
        view: Option<PlanningDraftEditorOverlayView>,
    },
    Approval(ApprovalInlineScreenModel),
}

pub(super) struct ApprovalInlineScreenModel {
    pub(super) available: bool,
    pub(super) header_lines: Vec<Line<'static>>,
    pub(super) detail_lines: Vec<Line<'static>>,
    pub(super) key_lines: Vec<Line<'static>>,
    pub(super) scroll_offset: u16,
    pub(super) visible_start: usize,
    pub(super) visible_end: usize,
    pub(super) rendered_detail_rows: usize,
}

struct StateChange<T> {
    expected: T,
    next: T,
}

struct ApprovalScrollStateChange {
    conversation_history_identity_revision: u64,
    server_request_id: String,
    expected: usize,
    next: usize,
}

struct SessionListStateChange {
    expected_screen_model: SessionOverlayScreenModel,
    expected: ListState,
    next: ListState,
}

pub(super) struct InlineFrameRenderReceipt {
    activity: Option<StateChange<ProgressiveActivityOverlayUiState>>,
    planning_editor: Option<StateChange<PlanningDraftEditorUiState>>,
    startup_diagnostics_scroll_offset: Option<StateChange<usize>>,
    help_scroll_offset: Option<StateChange<usize>>,
    approval_scroll_offset: Option<ApprovalScrollStateChange>,
    session_list_state: Option<SessionListStateChange>,
    queue_receipt_undo_hit_area: StateChange<Option<Rect>>,
}

impl InlineShellFrameModel {
    pub(super) fn into_parts(
        self,
    ) -> (
        InlineConversationFrameProjection,
        InlineInspectionFrameModel,
        InlineFrameRenderReceipt,
    ) {
        (self.conversation, self.inspection, self.receipt)
    }
}

impl InlineFrameRenderReceipt {
    pub(super) fn record_queue_receipt_undo_hit_area(&mut self, hit_area: Option<Rect>) {
        self.queue_receipt_undo_hit_area.next = hit_area;
    }

    pub(super) fn record_session_list_state(&mut self, list_state: ListState) {
        if let Some(change) = self.session_list_state.as_mut() {
            change.next = list_state;
        }
    }
}

pub(super) fn capture_inline_shell_frame_model(
    app: &NativeTuiApp,
    mode: ShellFrontendMode,
    area: Rect,
    mut projection: InlineConversationFrameProjection,
) -> InlineShellFrameModel {
    let _ = mode;
    let inspection_area = inline_frame_inspection_area(&projection, area);
    let parallel_event_area = inline_parallel_event_stream_area(&projection, area);
    projection.finalize_parallel_live_stream_geometry(parallel_event_area);
    let mut receipt = InlineFrameRenderReceipt {
        activity: None,
        planning_editor: None,
        startup_diagnostics_scroll_offset: None,
        help_scroll_offset: None,
        approval_scroll_offset: None,
        session_list_state: None,
        queue_receipt_undo_hit_area: StateChange {
            expected: app.planning.queue_overlay_ui_state.receipt_undo_hit_area(),
            next: None,
        },
    };

    let inspection = match projection.shell_overlay {
        ShellOverlay::Hidden
            if projection.parallel_mode_enabled
                && !projection.renders_parallel_viewport_handoff =>
        {
            InlineInspectionFrameModel::ParallelSupervisor(
                projection
                    .supersession_overlay_view
                    .take()
                    .map(|view| *view)
                    .expect("parallel frame projection must own the supervisor view"),
            )
        }
        ShellOverlay::Hidden => InlineInspectionFrameModel::Conversation,
        ShellOverlay::Startup => {
            let view = build_startup_overlay_view(StartupOverlayFrameInput {
                startup_state: &app.shell.chrome.startup_state,
                language: app.shell.tui_language,
                parallel_mode_enabled: projection.parallel_mode_enabled,
                operator_diagnostic_lines: std::mem::take(
                    &mut projection.operator_diagnostic_lines,
                ),
            });
            let visible_rows = startup_visible_warning_rows(inspection_area, &view);
            let rendered_rows =
                count_rendered_inline_rows(&view.warning_lines, inspection_area.width);
            let max_scroll = rendered_rows.saturating_sub(visible_rows.max(1));
            let next = app.shell.startup_diagnostics_scroll_offset.min(max_scroll);
            receipt.startup_diagnostics_scroll_offset = Some(StateChange {
                expected: app.shell.startup_diagnostics_scroll_offset,
                next,
            });
            InlineInspectionFrameModel::Startup {
                view,
                warning_scroll_offset: next.min(usize::from(u16::MAX)) as u16,
            }
        }
        ShellOverlay::Sessions => {
            let screen_model = capture_session_overlay_screen_model(app);
            let view = build_session_overlay_view(&screen_model);
            let expected = app.shell.session_overlay_ui_state.list_state;
            let mut list_state = expected;
            if view.list_view.message_lines.is_none() {
                list_state.select(view.list_view.selected_index);
            }
            receipt.session_list_state = Some(SessionListStateChange {
                expected_screen_model: screen_model,
                expected,
                next: list_state,
            });
            InlineInspectionFrameModel::Sessions { view, list_state }
        }
        ShellOverlay::ModelSelection => {
            let state = &app.shell.model_selection_overlay_ui_state;
            InlineInspectionFrameModel::ModelSelection(build_model_selection_overlay_view(
                ModelSelectionFrameInput {
                    step: state.step(),
                    selected_model_index: state.selected_model_index(),
                    selected_effort_index: state.selected_effort_index(),
                    staged_model_index: state.staged_model_index(),
                    staged_model_label: state.staged_model().label,
                    current_model_label: app
                        .conversation
                        .turn_options
                        .model
                        .as_deref()
                        .unwrap_or("default"),
                    current_effort_label: app
                        .conversation
                        .turn_options
                        .reasoning_effort
                        .map(|effort| effort.label())
                        .unwrap_or("default"),
                },
            ))
        }
        ShellOverlay::ViewSelection => InlineInspectionFrameModel::ViewSelection(
            build_view_selection_overlay_view(ViewSelectionFrameInput {
                current_mode: app.conversation.conversation_view_mode,
                selected_mode_index: app
                    .shell
                    .view_selection_overlay_ui_state
                    .selected_mode_index(),
            }),
        ),
        ShellOverlay::LanguageSelection => InlineInspectionFrameModel::LanguageSelection(
            build_language_selection_overlay_view(LanguageSelectionFrameInput {
                current_language: app.shell.tui_language,
                selected_language_index: app
                    .shell
                    .language_selection_overlay_ui_state
                    .selected_language_index(),
            }),
        ),
        ShellOverlay::Supersession => InlineInspectionFrameModel::Supersession(
            projection
                .supersession_overlay_view
                .take()
                .map(|view| *view)
                .expect("supersession frame projection must own its view"),
        ),
        ShellOverlay::ParallelPeek => InlineInspectionFrameModel::ParallelPeek {
            view: build_parallel_peek_overlay_view_from_snapshot(
                &projection.sampled_parallel_supervisor,
                &app.shell.parallel_peek_overlay_ui_state,
            ),
            step: app.shell.parallel_peek_overlay_ui_state.step(),
            scroll_from_bottom: app
                .shell
                .parallel_peek_overlay_ui_state
                .conversation_scroll_from_bottom(),
        },
        ShellOverlay::WorkCenter => InlineInspectionFrameModel::WorkCenter(
            projection
                .work_center_overlay_view
                .take()
                .map(|view| *view)
                .expect("work center frame projection must own its view"),
        ),
        ShellOverlay::Activity => {
            let (view, change) =
                capture_activity_frame(app, inspection_area, projection.rendered_at_epoch_millis);
            receipt.activity = Some(change);
            InlineInspectionFrameModel::Activity(view)
        }
        ShellOverlay::Help => {
            let view = build_help_overlay_view(app.shell.tui_language);
            let visible_rows = help_visible_command_rows(inspection_area, &view);
            let rendered_rows =
                count_rendered_inline_rows(&view.command_lines, inspection_area.width);
            let max_scroll = rendered_rows.saturating_sub(visible_rows.max(1));
            let next = app.shell.help_scroll_offset.min(max_scroll);
            receipt.help_scroll_offset = Some(StateChange {
                expected: app.shell.help_scroll_offset,
                next,
            });
            InlineInspectionFrameModel::Help {
                language: app.shell.tui_language,
                view,
                scroll_offset: next.min(usize::from(u16::MAX)) as u16,
            }
        }
        ShellOverlay::Reviews => InlineInspectionFrameModel::Reviews(build_reviews_overlay_view(
            app.shell.reviews_overlay_ui_state.screen_model(),
        )),
        ShellOverlay::Queue => {
            InlineInspectionFrameModel::Queue(build_queue_overlay_view_from_screen_model(
                app.queue_overlay_screen_model_from_projection(
                    &projection.sampled_planning_runtime_projection,
                    projection.parallel_mode_enabled,
                ),
            ))
        }
        ShellOverlay::DirectionsMaintenance
            if app.planning.directions_maintenance_overlay_ui_state.step()
                == DirectionsMaintenanceOverlayStep::ManualEditor =>
        {
            capture_draft_editor_frame(
                app,
                inspection_area,
                "Directions Support Draft",
                &mut receipt,
            )
        }
        ShellOverlay::DirectionsMaintenance => InlineInspectionFrameModel::Directions(
            build_directions_maintenance_overlay_view(DirectionsMaintenanceFrameInput {
                ui_state: &app.planning.directions_maintenance_overlay_ui_state,
            }),
        ),
        ShellOverlay::PlanningInit
            if app.planning.planning_init_overlay_ui_state.step()
                == PlanningInitOverlayStep::ManualEditor =>
        {
            capture_draft_editor_frame(app, inspection_area, "Planning Draft", &mut receipt)
        }
        ShellOverlay::PlanningInit => {
            let workspace_directory = app.planning_workspace_directory();
            InlineInspectionFrameModel::PlanningInit(
                build_planning_init_overlay_view_from_projection(
                    PlanningInitOverlayFrameInput {
                        ui_state: &app.planning.planning_init_overlay_ui_state,
                        workspace_directory: &workspace_directory,
                        max_auto_turns_label: app.current_max_auto_turns_label(),
                        turn_budget_edit_buffer: app
                            .max_auto_turns_edit_buffer()
                            .map(str::to_string),
                    },
                    &projection.sampled_planning_runtime_projection,
                ),
            )
        }
        ShellOverlay::Approval => {
            let (model, change) = capture_approval_frame(app, inspection_area);
            receipt.approval_scroll_offset = change;
            InlineInspectionFrameModel::Approval(model)
        }
    };

    InlineShellFrameModel {
        conversation: projection,
        inspection,
        receipt,
    }
}

pub(super) fn apply_inline_frame_render_receipt(
    app: &mut NativeTuiApp,
    receipt: InlineFrameRenderReceipt,
) -> bool {
    // A terminal frame is one transaction. Validate every sampled value before
    // mutating any UI state so a stale receipt can never be partially applied.
    if !inline_frame_render_receipt_matches(app, &receipt) {
        return false;
    }

    let InlineFrameRenderReceipt {
        activity,
        planning_editor,
        startup_diagnostics_scroll_offset,
        help_scroll_offset,
        approval_scroll_offset,
        session_list_state,
        queue_receipt_undo_hit_area,
    } = receipt;

    if let Some(change) = activity {
        app.shell.progressive_activity_overlay_ui_state = change.next;
    }
    if let Some(change) = planning_editor {
        app.planning.planning_draft_editor_ui_state = change.next;
    }
    if let Some(change) = startup_diagnostics_scroll_offset {
        app.shell.startup_diagnostics_scroll_offset = change.next;
    }
    if let Some(change) = help_scroll_offset {
        app.shell.help_scroll_offset = change.next;
    }
    if let Some(change) = approval_scroll_offset {
        let ConversationState::Ready(conversation) =
            &mut app.conversation.lifecycle.conversation_state
        else {
            unreachable!("approval receipt was preflighted against a ready conversation");
        };
        conversation.approval_detail_scroll_offset = change.next;
    }
    if let Some(change) = session_list_state {
        app.shell.session_overlay_ui_state.list_state = change.next;
    }
    app.planning
        .queue_overlay_ui_state
        .bind_receipt_undo_hit_area(queue_receipt_undo_hit_area.next);
    true
}

fn inline_frame_render_receipt_matches(
    app: &NativeTuiApp,
    receipt: &InlineFrameRenderReceipt,
) -> bool {
    let activity_matches = receipt
        .activity
        .as_ref()
        .is_none_or(|change| app.shell.progressive_activity_overlay_ui_state == change.expected);
    let planning_editor_matches = receipt
        .planning_editor
        .as_ref()
        .is_none_or(|change| app.planning.planning_draft_editor_ui_state == change.expected);
    let startup_diagnostics_matches = receipt
        .startup_diagnostics_scroll_offset
        .as_ref()
        .is_none_or(|change| app.shell.startup_diagnostics_scroll_offset == change.expected);
    let help_matches = receipt
        .help_scroll_offset
        .as_ref()
        .is_none_or(|change| app.shell.help_scroll_offset == change.expected);
    let approval_matches = receipt
        .approval_scroll_offset
        .as_ref()
        .is_none_or(|change| {
            app.conversation.conversation_history_identity_revision
                == change.conversation_history_identity_revision
                && matches!(
                    &app.conversation.lifecycle.conversation_state,
                    ConversationState::Ready(conversation)
                        if conversation.approval_detail_scroll_offset == change.expected
                            && conversation
                                .pending_approval_request()
                                .is_some_and(|request| {
                                    request.server_request_id == change.server_request_id
                                })
                )
        });
    let session_matches = receipt.session_list_state.as_ref().is_none_or(|change| {
        app.shell.session_overlay_ui_state.list_state == change.expected
            && capture_session_overlay_screen_model(app) == change.expected_screen_model
    });
    let queue_matches = app.planning.queue_overlay_ui_state.receipt_undo_hit_area()
        == receipt.queue_receipt_undo_hit_area.expected;

    activity_matches
        && planning_editor_matches
        && startup_diagnostics_matches
        && help_matches
        && approval_matches
        && session_matches
        && queue_matches
}

fn capture_activity_frame(
    app: &NativeTuiApp,
    area: Rect,
    rendered_at_ms: Option<i64>,
) -> (
    ActivityOverlayView,
    StateChange<ProgressiveActivityOverlayUiState>,
) {
    let expected = app.shell.progressive_activity_overlay_ui_state.clone();
    let mut next = expected.clone();
    let selected_kind = next.selected_kind();
    let card_filter = next.card_filter();
    let (lifecycle_epoch, diff_available, output_available, cards, wait_status) =
        match &app.conversation.lifecycle.conversation_state {
            ConversationState::Ready(conversation) => {
                let detail = &conversation.progressive_activity_detail;
                (
                    detail.lifecycle_epoch(),
                    detail
                        .document(ProgressiveActivityDetailKind::Diff)
                        .is_some(),
                    detail
                        .document(ProgressiveActivityDetailKind::Output)
                        .is_some(),
                    detail.timeline_cards(rendered_at_ms),
                    detail.wait_status(
                        conversation.progressive_activity.retrying_summary(),
                        conversation.pending_approval_request().is_some(),
                    ),
                )
            }
            ConversationState::Loading | ConversationState::Failed(_) => {
                (0, false, false, Vec::new(), None)
            }
        };
    let filtered_indices = filter_cards_by_kind(&cards, card_filter);
    next.clamp_selected_card(filtered_indices.len());
    let selected_card_index = next.selected_card_index();
    let filtered_cards = filtered_indices
        .iter()
        .filter_map(|index| cards.get(*index).cloned())
        .collect::<Vec<_>>();
    let document = match &app.conversation.lifecycle.conversation_state {
        ConversationState::Ready(conversation) => filtered_indices
            .get(selected_card_index)
            .and_then(|card_index| cards.get(*card_index))
            .filter(|card| next.expand_state().is_card_expanded(card.key))
            .and_then(|card| conversation.progressive_activity_detail.card_document(card)),
        ConversationState::Loading | ConversationState::Failed(_) => None,
    };
    next.select_document(
        lifecycle_epoch,
        document.as_ref().map(|document| document.sequence),
    );
    let document_view = document.as_ref().map(|document| ActivityOverlayDocument {
        text: document.text(),
        source_bytes: document.source_bytes,
        retained_bytes: document.retained_bytes,
        truncated_bytes: document.truncated_bytes,
        history_incomplete: document.history_incomplete,
    });
    let header_view = build_activity_overlay_list_view(
        card_filter,
        &filtered_cards,
        selected_card_index,
        next.list_focus(),
        wait_status.as_ref(),
        next.expand_state(),
        selected_kind,
        diff_available,
        output_available,
        document_view,
        ProgressiveActivityPageCursor::at(0),
        area.width,
        0,
    );
    let desired_header_height = count_rendered_inline_rows(&header_view.header_lines, area.width)
        .saturating_add(1)
        .min(usize::from(u16::MAX)) as u16;
    let key_height =
        inline_section_height(&header_view.key_lines, 4).min(area.height.saturating_sub(4));
    let header_height =
        desired_header_height.min(area.height.saturating_sub(key_height).saturating_sub(2));
    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(header_height),
            Constraint::Min(2),
            Constraint::Length(key_height),
        ])
        .split(area);
    let body_height = layout[1].height.saturating_sub(1);
    next.sync_viewport(layout[1].width, body_height);
    let requested_page_cursor = next.current_page_cursor();
    let view = build_activity_overlay_list_view(
        card_filter,
        &filtered_cards,
        selected_card_index,
        next.list_focus(),
        wait_status.as_ref(),
        next.expand_state(),
        selected_kind,
        diff_available,
        output_available,
        document_view,
        requested_page_cursor,
        layout[1].width,
        body_height,
    );
    let header_body_y = layout[0].y.saturating_add(1);
    let header_bottom = layout[0].bottom();
    let card_hit_areas = view
        .card_rows
        .iter()
        .filter_map(|row| {
            let preceding_rows = count_rendered_inline_rows(
                &view.header_lines[..row.header_line_index],
                layout[0].width,
            );
            let y = header_body_y.saturating_add(u16::try_from(preceding_rows).unwrap_or(u16::MAX));
            (y < header_bottom).then_some(ProgressiveActivityCardHitArea {
                card_index: row.card_index,
                area: Rect::new(layout[0].x, y, layout[0].width, 1),
            })
        })
        .collect();
    next.bind_card_hit_areas(card_hit_areas);
    next.set_page_cursor_window(view.current_page_cursor, view.next_page_cursor);
    (view, StateChange { expected, next })
}

fn help_visible_command_rows(area: Rect, view: &HelpOverlayView) -> usize {
    let body_lines = view
        .header_lines
        .iter()
        .skip(1)
        .cloned()
        .collect::<Vec<_>>();
    let key_height = count_rendered_inline_rows(&view.key_lines, area.width)
        .saturating_add(1)
        .clamp(2, 4) as u16;
    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(inline_section_height(&body_lines, 4)),
            Constraint::Min(2),
            Constraint::Length(key_height),
        ])
        .split(area);
    usize::from(layout[1].height.saturating_sub(1))
}

fn startup_visible_warning_rows(area: Rect, view: &StartupOverlayView) -> usize {
    if view.warning_lines.is_empty() {
        return 0;
    }
    let body_lines = view
        .header_lines
        .iter()
        .skip(1)
        .cloned()
        .collect::<Vec<_>>();
    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(inline_section_height(&body_lines, 4)),
            Constraint::Length(inline_section_height(&view.summary_lines, 2)),
            Constraint::Length(inline_section_height(&view.warning_lines, 5)),
            Constraint::Length(inline_section_height(&view.key_lines, 4)),
            Constraint::Min(2),
        ])
        .split(area);
    usize::from(layout[2].height.saturating_sub(1))
}

fn capture_draft_editor_frame(
    app: &NativeTuiApp,
    area: Rect,
    title: &'static str,
    receipt: &mut InlineFrameRenderReceipt,
) -> InlineInspectionFrameModel {
    let expected = app.planning.planning_draft_editor_ui_state.clone();
    let mut next = expected.clone();
    let editor_height = area.height.saturating_sub(14).max(6);
    let editor_content_height = editor_height.saturating_sub(1).max(1);
    next.sync_editor_scroll(editor_content_height);
    let view = build_planning_draft_editor_overlay_view_from_state(&next, editor_content_height);
    receipt.planning_editor = Some(StateChange { expected, next });
    InlineInspectionFrameModel::DraftEditor { title, view }
}

fn capture_approval_frame(
    app: &NativeTuiApp,
    area: Rect,
) -> (ApprovalInlineScreenModel, Option<ApprovalScrollStateChange>) {
    let Some((request, requested_scroll_offset, submitted_decision)) =
        (match &app.conversation.lifecycle.conversation_state {
            ConversationState::Ready(conversation) => {
                conversation.pending_approval_request().map(|request| {
                    (
                        request.clone(),
                        conversation.approval_detail_scroll_offset,
                        conversation.pending_approval_decision(),
                    )
                })
            }
            ConversationState::Loading | ConversationState::Failed(_) => None,
        })
    else {
        return (
            ApprovalInlineScreenModel {
                available: false,
                header_lines: Vec::new(),
                detail_lines: Vec::new(),
                key_lines: Vec::new(),
                scroll_offset: 0,
                visible_start: 0,
                visible_end: 0,
                rendered_detail_rows: 0,
            },
            None,
        );
    };
    let kind = match request.kind {
        crate::domain::conversation::ConversationApprovalRequestKind::CommandExecution => {
            "Command execution"
        }
        crate::domain::conversation::ConversationApprovalRequestKind::FileChange => "File change",
        crate::domain::conversation::ConversationApprovalRequestKind::Permissions => "Permissions",
    };
    let mut header_lines = vec![
        Line::from(format!("Type: {kind}")),
        Line::from(format!("Request: {}", request.server_request_id)),
        Line::from(format!("Method: {}", request.method)),
        Line::from(""),
        Line::from(request.summary.clone()),
    ];
    if let Some(decision) = submitted_decision {
        header_lines.push(Line::from(format!(
            "Decision submitted: {} / waiting for runtime resolution",
            approval_decision_label(decision)
        )));
    }
    let detail_lines = request
        .details
        .iter()
        .cloned()
        .map(Line::from)
        .collect::<Vec<_>>();
    let key_lines = match submitted_decision {
        Some(decision) => vec![
            AkraTheme::key_line(format!(
                "Decision locked: {}",
                approval_decision_label(decision)
            )),
            AkraTheme::key_line("Waiting for runtime resolution"),
            AkraTheme::key_line("Up/Down/Page: scroll    Ctrl-C: stop turn"),
        ],
        None => vec![
            AkraTheme::key_line("Y: approve once"),
            AkraTheme::key_line("N / Esc: decline"),
            AkraTheme::key_line("Up/Down/Page: scroll    Ctrl-C: decline + stop"),
        ],
    };
    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(wrapped_approval_panel_height(&header_lines, area.width, 6)),
            Constraint::Min(3),
            Constraint::Length(wrapped_approval_panel_height(&key_lines, area.width, 4)),
        ])
        .split(area);
    let visible_detail_rows = layout[1].height.saturating_sub(1) as usize;
    let rendered_detail_rows = count_rendered_inline_rows(&detail_lines, layout[1].width);
    let max_scroll = rendered_detail_rows.saturating_sub(visible_detail_rows.max(1));
    let next_scroll = requested_scroll_offset
        .min(max_scroll)
        .min(u16::MAX as usize);
    let scroll_offset = next_scroll as u16;
    let visible_start = if rendered_detail_rows == 0 {
        0
    } else {
        usize::from(scroll_offset) + 1
    };
    let visible_end = (usize::from(scroll_offset) + visible_detail_rows).min(rendered_detail_rows);
    (
        ApprovalInlineScreenModel {
            available: true,
            header_lines,
            detail_lines,
            key_lines,
            scroll_offset,
            visible_start,
            visible_end,
            rendered_detail_rows,
        },
        Some(ApprovalScrollStateChange {
            conversation_history_identity_revision: app
                .conversation
                .conversation_history_identity_revision,
            server_request_id: request.server_request_id,
            expected: requested_scroll_offset,
            next: next_scroll,
        }),
    )
}

fn approval_decision_label(
    decision: crate::domain::conversation::ConversationApprovalDecision,
) -> &'static str {
    match decision {
        crate::domain::conversation::ConversationApprovalDecision::Accept => "accept",
        crate::domain::conversation::ConversationApprovalDecision::Decline => "decline",
    }
}

fn wrapped_approval_panel_height(lines: &[Line<'_>], width: u16, minimum: u16) -> u16 {
    let width = usize::from(width.max(1));
    let wrapped_rows = lines
        .iter()
        .map(|line| line.width().max(1).div_ceil(width))
        .sum::<usize>();
    u16::try_from(wrapped_rows.saturating_add(1))
        .unwrap_or(u16::MAX)
        .max(minimum)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapter::inbound::tui::app::test_helpers::test_native_tui_app;
    use crate::domain::conversation::{
        ConversationApprovalRequest, ConversationApprovalRequestKind,
    };
    use crate::domain::recent_sessions::{SessionCatalog, SessionCatalogTier};

    #[test]
    fn stale_receipt_rejects_the_entire_ui_transaction() {
        let mut app = test_native_tui_app();
        app.shell.chrome.shell_overlay = ShellOverlay::Help;
        app.shell.help_scroll_offset = usize::MAX;
        let area = Rect::new(0, 0, 80, 24);
        let projection = InlineConversationFrameProjection::from_app(&app, area.width);
        let model = capture_inline_shell_frame_model(
            &app,
            ShellFrontendMode::InlineMainBuffer,
            area,
            projection,
        );
        let (_, _, receipt) = model.into_parts();

        let conflicting_hit_area = Rect::new(2, 3, 4, 1);
        app.planning
            .queue_overlay_ui_state
            .bind_receipt_undo_hit_area(Some(conflicting_hit_area));

        assert!(!apply_inline_frame_render_receipt(&mut app, receipt));
        assert_eq!(app.shell.help_scroll_offset, usize::MAX);
        assert_eq!(
            app.planning.queue_overlay_ui_state.receipt_undo_hit_area(),
            Some(conflicting_hit_area)
        );
    }

    #[test]
    fn non_queryable_session_message_preserves_existing_list_state() {
        let mut app = test_native_tui_app();
        app.shell.chrome.shell_overlay = ShellOverlay::Sessions;
        app.shell.chrome.session_state = SessionState::Ready(SessionCatalog::unsupported(
            SessionCatalogTier::AttachOnly,
            "session listing is unsupported",
            vec!["manual attach only".to_string()],
        ));
        let existing_list_state = ListState::default().with_offset(4).with_selected(Some(5));
        app.shell.session_overlay_ui_state.list_state = existing_list_state;
        let area = Rect::new(0, 0, 80, 24);
        let projection = InlineConversationFrameProjection::from_app(&app, area.width);
        let model = capture_inline_shell_frame_model(
            &app,
            ShellFrontendMode::InlineMainBuffer,
            area,
            projection,
        );
        let (_, inspection, receipt) = model.into_parts();

        let InlineInspectionFrameModel::Sessions { view, list_state } = inspection else {
            panic!("session overlay must capture a session frame");
        };
        assert!(view.list_view.message_lines.is_some());
        assert_eq!(list_state, existing_list_state);
        assert!(apply_inline_frame_render_receipt(&mut app, receipt));
        assert_eq!(
            app.shell.session_overlay_ui_state.list_state,
            existing_list_state
        );
    }

    #[test]
    fn approval_receipt_uses_the_same_u16_bounded_scroll_as_the_frame() {
        let mut app = test_native_tui_app();
        app.shell.chrome.shell_overlay = ShellOverlay::Approval;
        let ConversationState::Ready(conversation) =
            &mut app.conversation.lifecycle.conversation_state
        else {
            panic!("test app should have a ready conversation");
        };
        let mut runtime_snapshot = conversation.runtime_snapshot().clone();
        runtime_snapshot.approval = Some(crate::core::app::ApprovalAuthoritySnapshot {
            request: ConversationApprovalRequest {
                approval_id: "approval-large-scroll".to_string(),
                server_request_id: "server-large-scroll".to_string(),
                method: "item/commandExecution/requestApproval".to_string(),
                kind: ConversationApprovalRequestKind::CommandExecution,
                summary: "large approval detail".to_string(),
                details: vec!["x".repeat(140_000)],
            },
            decision: None,
            phase: crate::core::app::ApprovalAuthorityPhase::Pending,
        });
        conversation.apply_runtime_snapshot(runtime_snapshot);
        conversation.approval_detail_scroll_offset = usize::MAX;
        let area = Rect::new(0, 0, 2, 10);
        let projection = InlineConversationFrameProjection::from_app(&app, area.width);
        let model = capture_inline_shell_frame_model(
            &app,
            ShellFrontendMode::InlineMainBuffer,
            area,
            projection,
        );
        let (_, inspection, receipt) = model.into_parts();

        let InlineInspectionFrameModel::Approval(model) = inspection else {
            panic!("approval overlay must capture an approval frame");
        };
        assert_eq!(model.scroll_offset, u16::MAX);
        assert!(model.rendered_detail_rows > usize::from(u16::MAX));
        assert!(apply_inline_frame_render_receipt(&mut app, receipt));
        let ConversationState::Ready(conversation) = &app.conversation.lifecycle.conversation_state
        else {
            panic!("test app should remain ready");
        };
        assert_eq!(
            conversation.approval_detail_scroll_offset,
            usize::from(model.scroll_offset)
        );
    }
}
