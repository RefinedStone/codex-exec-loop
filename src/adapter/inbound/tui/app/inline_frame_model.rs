use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::text::Line;
use ratatui::widgets::ListState;

use crate::application::service::planning::PlanningRuntimeProjection;
use crate::domain::parallel_mode::ParallelModeSupervisorSnapshot;

use super::shell_presentation::{
    ActivityOverlayDocument, ActivityOverlayView, ConversationProjectionSample,
    ConversationScreenModel, DirectionsMaintenanceOverlayView, HelpOverlayView, InlineTailView,
    LanguageSelectionOverlayView, ModelSelectionOverlayView, ParallelPeekOverlayView,
    PlanningDraftEditorOverlayView, PlanningInitOverlayView, QueueOverlayView, ReviewsOverlayView,
    SessionOverlayView, StartupOverlayView, SupersessionOverlayView,
    TranscriptHandoffDeliveryToken, TurnSteerConfirmationScreenModel, ViewSelectionOverlayView,
    build_activity_overlay_list_view, build_directions_maintenance_overlay_view,
    build_help_overlay_view, build_inline_live_transcript_lines, build_inline_tail_view,
    build_language_selection_overlay_view, build_model_selection_overlay_view,
    build_parallel_peek_overlay_view_from_snapshot,
    build_planning_draft_editor_overlay_view_from_state,
    build_planning_init_overlay_view_from_projection, build_queue_overlay_view_from_projection,
    build_reviews_overlay_view, build_session_overlay_view, build_startup_overlay_view,
    build_supersession_overlay_view, build_view_selection_overlay_view,
};
use super::shell_rendering::{
    count_rendered_inline_rows, inline_frame_inspection_area, inline_section_height,
};
use super::*;

/*
 * Terminal delivery owns one fully materialized inline frame. This module is the
 * only production boundary allowed to sample NativeTuiApp for that frame. The
 * rendering modules consume the owned models below and return a delivery receipt;
 * they never reread or mutate the aggregate while terminal I/O is in progress.
 */
pub(super) struct InlineConversationFrameProjection {
    pub(super) core_revision: u64,
    pub(super) tail_view: InlineTailView,
    pub(super) live_transcript_lines: Vec<Line<'static>>,
    pub(super) shell_overlay: ShellOverlay,
    pub(super) inline_history_render_mode: InlineHistoryRenderMode,
    pub(super) parallel_mode_enabled: bool,
    pub(super) renders_viewport_transcript_handoff: bool,
    pub(super) transcript_handoff_delivery_token: Option<Box<TranscriptHandoffDeliveryToken>>,
    pub(super) renders_parallel_viewport_handoff: bool,
    pub(super) exit_confirmation_visible: bool,
    pub(super) turn_steer_confirmation: Option<Box<TurnSteerConfirmationScreenModel>>,
    pub(super) parallel_supervisor_event_lines: Vec<Line<'static>>,
    pub(super) supersession_overlay_view: Option<Box<SupersessionOverlayView>>,
    sampled_parallel_supervisor: Box<ParallelModeSupervisorSnapshot>,
    sampled_planning_runtime_projection: Box<PlanningRuntimeProjection>,
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
        let screen_model = ConversationScreenModel::from_app_with_sample(app, sample);
        let supersession_overlay_view = (screen_model.shell_overlay == ShellOverlay::Supersession
            || (screen_model.shell_overlay == ShellOverlay::Hidden
                && screen_model.parallel_mode_enabled))
            .then(|| {
                Box::new(build_supersession_overlay_view(
                    &screen_model,
                    &app.supersession_mud_ui_state,
                ))
            });
        Self::from_screen_model(screen_model, content_width, supersession_overlay_view)
    }

    fn from_screen_model(
        screen_model: ConversationScreenModel<'_>,
        content_width: u16,
        supersession_overlay_view: Option<Box<SupersessionOverlayView>>,
    ) -> Self {
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
            tail_view,
            live_transcript_lines,
            shell_overlay: screen_model.shell_overlay,
            inline_history_render_mode: screen_model.inline_history_render_mode,
            parallel_mode_enabled: screen_model.parallel_mode_enabled,
            renders_viewport_transcript_handoff,
            transcript_handoff_delivery_token,
            renders_parallel_viewport_handoff,
            exit_confirmation_visible: screen_model.exit_confirmation_visible,
            turn_steer_confirmation: screen_model.turn_steer_confirmation.map(Box::new),
            parallel_supervisor_event_lines: screen_model.parallel_supervisor_event_lines,
            supersession_overlay_view,
            sampled_parallel_supervisor,
            sampled_planning_runtime_projection,
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
    Startup(StartupOverlayView),
    Sessions {
        view: SessionOverlayView,
        list_state: ListState,
    },
    ModelSelection(ModelSelectionOverlayView),
    ViewSelection(ViewSelectionOverlayView),
    LanguageSelection(LanguageSelectionOverlayView),
    Supersession(SupersessionOverlayView),
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
    let mut receipt = InlineFrameRenderReceipt {
        activity: None,
        planning_editor: None,
        help_scroll_offset: None,
        approval_scroll_offset: None,
        session_list_state: None,
        queue_receipt_undo_hit_area: StateChange {
            expected: app.queue_overlay_ui_state.receipt_undo_hit_area(),
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
        ShellOverlay::Startup => InlineInspectionFrameModel::Startup(build_startup_overlay_view(
            app,
            projection.parallel_mode_enabled,
        )),
        ShellOverlay::Sessions => {
            let screen_model = SessionOverlayScreenModel::capture(app);
            let view = build_session_overlay_view(&screen_model);
            let expected = app.session_overlay_ui_state.list_state;
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
            InlineInspectionFrameModel::ModelSelection(build_model_selection_overlay_view(app))
        }
        ShellOverlay::ViewSelection => {
            InlineInspectionFrameModel::ViewSelection(build_view_selection_overlay_view(app))
        }
        ShellOverlay::LanguageSelection => InlineInspectionFrameModel::LanguageSelection(
            build_language_selection_overlay_view(app),
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
                &app.parallel_peek_overlay_ui_state,
            ),
            step: app.parallel_peek_overlay_ui_state.step(),
            scroll_from_bottom: app
                .parallel_peek_overlay_ui_state
                .conversation_scroll_from_bottom(),
        },
        ShellOverlay::Activity => {
            let (view, change) = capture_activity_frame(app, inspection_area);
            receipt.activity = Some(change);
            InlineInspectionFrameModel::Activity(view)
        }
        ShellOverlay::Help => {
            let view = build_help_overlay_view(app.tui_language);
            let visible_rows = help_visible_command_rows(inspection_area, &view);
            let rendered_rows =
                count_rendered_inline_rows(&view.command_lines, inspection_area.width);
            let max_scroll = rendered_rows.saturating_sub(visible_rows.max(1));
            let next = app.help_scroll_offset.min(max_scroll);
            receipt.help_scroll_offset = Some(StateChange {
                expected: app.help_scroll_offset,
                next,
            });
            InlineInspectionFrameModel::Help {
                language: app.tui_language,
                view,
                scroll_offset: next.min(usize::from(u16::MAX)) as u16,
            }
        }
        ShellOverlay::Reviews => InlineInspectionFrameModel::Reviews(build_reviews_overlay_view(
            app.reviews_overlay_ui_state.screen_model(),
        )),
        ShellOverlay::Queue => {
            InlineInspectionFrameModel::Queue(build_queue_overlay_view_from_projection(
                app,
                &projection.sampled_planning_runtime_projection,
                projection.parallel_mode_enabled,
            ))
        }
        ShellOverlay::DirectionsMaintenance
            if app.directions_maintenance_overlay_ui_state.step()
                == DirectionsMaintenanceOverlayStep::ManualEditor =>
        {
            capture_draft_editor_frame(
                app,
                inspection_area,
                "Directions Support Draft",
                &mut receipt,
            )
        }
        ShellOverlay::DirectionsMaintenance => {
            InlineInspectionFrameModel::Directions(build_directions_maintenance_overlay_view(app))
        }
        ShellOverlay::PlanningInit
            if app.planning_init_overlay_ui_state.step()
                == PlanningInitOverlayStep::ManualEditor =>
        {
            capture_draft_editor_frame(app, inspection_area, "Planning Draft", &mut receipt)
        }
        ShellOverlay::PlanningInit => InlineInspectionFrameModel::PlanningInit(
            build_planning_init_overlay_view_from_projection(
                app,
                &projection.sampled_planning_runtime_projection,
            ),
        ),
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
        help_scroll_offset,
        approval_scroll_offset,
        session_list_state,
        queue_receipt_undo_hit_area,
    } = receipt;

    if let Some(change) = activity {
        app.progressive_activity_overlay_ui_state = change.next;
    }
    if let Some(change) = planning_editor {
        app.planning_draft_editor_ui_state = change.next;
    }
    if let Some(change) = help_scroll_offset {
        app.help_scroll_offset = change.next;
    }
    if let Some(change) = approval_scroll_offset {
        let ConversationState::Ready(conversation) = &mut app.conversation_state else {
            unreachable!("approval receipt was preflighted against a ready conversation");
        };
        conversation.approval_detail_scroll_offset = change.next;
    }
    if let Some(change) = session_list_state {
        app.session_overlay_ui_state.list_state = change.next;
    }
    app.queue_overlay_ui_state
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
        .is_none_or(|change| app.progressive_activity_overlay_ui_state == change.expected);
    let planning_editor_matches = receipt
        .planning_editor
        .as_ref()
        .is_none_or(|change| app.planning_draft_editor_ui_state == change.expected);
    let help_matches = receipt
        .help_scroll_offset
        .as_ref()
        .is_none_or(|change| app.help_scroll_offset == change.expected);
    let approval_matches = receipt
        .approval_scroll_offset
        .as_ref()
        .is_none_or(|change| {
            app.conversation_history_identity_revision
                == change.conversation_history_identity_revision
                && matches!(
                    &app.conversation_state,
                    ConversationState::Ready(conversation)
                        if conversation.approval_detail_scroll_offset == change.expected
                            && conversation
                                .pending_approval_request
                                .as_ref()
                                .is_some_and(|request| {
                                    request.server_request_id == change.server_request_id
                                })
                )
        });
    let session_matches = receipt.session_list_state.as_ref().is_none_or(|change| {
        app.session_overlay_ui_state.list_state == change.expected
            && SessionOverlayScreenModel::capture(app) == change.expected_screen_model
    });
    let queue_matches = app.queue_overlay_ui_state.receipt_undo_hit_area()
        == receipt.queue_receipt_undo_hit_area.expected;

    activity_matches
        && planning_editor_matches
        && help_matches
        && approval_matches
        && session_matches
        && queue_matches
}

fn capture_activity_frame(
    app: &NativeTuiApp,
    area: Rect,
) -> (
    ActivityOverlayView,
    StateChange<ProgressiveActivityOverlayUiState>,
) {
    let expected = app.progressive_activity_overlay_ui_state.clone();
    let mut next = expected.clone();
    let selected_kind = next.selected_kind();
    let card_filter = next.card_filter();
    let (lifecycle_epoch, diff_available, output_available, cards) = match &app.conversation_state {
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
                detail.cards(),
            )
        }
        ConversationState::Loading | ConversationState::Failed(_) => (0, false, false, Vec::new()),
    };
    let filtered_indices = filter_cards_by_kind(&cards, card_filter);
    next.clamp_selected_card(filtered_indices.len());
    let selected_card_index = next.selected_card_index();
    let filtered_cards = filtered_indices
        .iter()
        .filter_map(|index| cards.get(*index).cloned())
        .collect::<Vec<_>>();
    let document = match &app.conversation_state {
        ConversationState::Ready(conversation) => filtered_indices
            .get(selected_card_index)
            .and_then(|card_index| cards.get(*card_index))
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
        selected_kind,
        diff_available,
        output_available,
        document_view,
        requested_page_cursor,
        layout[1].width,
        body_height,
    );
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

fn capture_draft_editor_frame(
    app: &NativeTuiApp,
    area: Rect,
    title: &'static str,
    receipt: &mut InlineFrameRenderReceipt,
) -> InlineInspectionFrameModel {
    let expected = app.planning_draft_editor_ui_state.clone();
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
        (match &app.conversation_state {
            ConversationState::Ready(conversation) => conversation
                .pending_approval_request
                .as_ref()
                .map(|request| {
                    (
                        request.clone(),
                        conversation.approval_detail_scroll_offset,
                        conversation.pending_approval_decision(),
                    )
                }),
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
            conversation_history_identity_revision: app.conversation_history_identity_revision,
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
        app.shell_overlay = ShellOverlay::Help;
        app.help_scroll_offset = usize::MAX;
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
        app.queue_overlay_ui_state
            .bind_receipt_undo_hit_area(Some(conflicting_hit_area));

        assert!(!apply_inline_frame_render_receipt(&mut app, receipt));
        assert_eq!(app.help_scroll_offset, usize::MAX);
        assert_eq!(
            app.queue_overlay_ui_state.receipt_undo_hit_area(),
            Some(conflicting_hit_area)
        );
    }

    #[test]
    fn non_queryable_session_message_preserves_existing_list_state() {
        let mut app = test_native_tui_app();
        app.shell_overlay = ShellOverlay::Sessions;
        app.session_state = SessionState::Ready(SessionCatalog::unsupported(
            SessionCatalogTier::AttachOnly,
            "session listing is unsupported",
            vec!["manual attach only".to_string()],
        ));
        let existing_list_state = ListState::default().with_offset(4).with_selected(Some(5));
        app.session_overlay_ui_state.list_state = existing_list_state;
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
        assert_eq!(app.session_overlay_ui_state.list_state, existing_list_state);
    }

    #[test]
    fn approval_receipt_uses_the_same_u16_bounded_scroll_as_the_frame() {
        let mut app = test_native_tui_app();
        app.shell_overlay = ShellOverlay::Approval;
        let ConversationState::Ready(conversation) = &mut app.conversation_state else {
            panic!("test app should have a ready conversation");
        };
        conversation.pending_approval_request = Some(ConversationApprovalRequest {
            approval_id: "approval-large-scroll".to_string(),
            server_request_id: "server-large-scroll".to_string(),
            method: "item/commandExecution/requestApproval".to_string(),
            kind: ConversationApprovalRequestKind::CommandExecution,
            summary: "large approval detail".to_string(),
            details: vec!["x".repeat(140_000)],
        });
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
        let ConversationState::Ready(conversation) = &app.conversation_state else {
            panic!("test app should remain ready");
        };
        assert_eq!(
            conversation.approval_detail_scroll_offset,
            usize::from(model.scroll_offset)
        );
    }
}
