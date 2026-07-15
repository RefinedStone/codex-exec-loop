use super::super::shell_presentation::{
    ActivityOverlayDocument, ActivityOverlayView, DirectionsMaintenanceOverlayView,
    HelpOverlayView, LanguageSelectionOverlayView, ModelSelectionOverlayView, OverlayListView,
    ParallelPeekOverlayView, PlanningDraftEditorOverlayView, PlanningInitOverlayView,
    QueueOverlayView, SessionOverlayView, StartupOverlayView, SupersessionOverlayView,
    ViewSelectionOverlayView, build_activity_overlay_view,
    build_directions_maintenance_overlay_view, build_help_overlay_view,
    build_language_selection_overlay_view, build_model_selection_overlay_view,
    build_parallel_peek_overlay_view, build_planning_draft_editor_overlay_view,
    build_planning_init_overlay_view, build_queue_overlay_view, build_reviews_overlay_view,
    build_session_overlay_view, build_startup_overlay_view, build_supersession_overlay_view,
    build_view_selection_overlay_view,
};
use super::super::{
    AkraTheme, DirectionsMaintenanceOverlayStep, NativeTuiApp, ParallelPeekOverlayStep,
    PlanningInitOverlayStep, ShellOverlay,
};
use super::inline_layout::{
    InlineAppendOnlyStream, InlineAppendOnlyStreamTitle, InlineScrolledPanel, InlineTitledPanel,
    count_rendered_inline_rows, inline_section_height, set_cursor_if_visible, split_inline_section,
    take_panel_body_lines,
};
use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::text::Line;
use ratatui::widgets::{List, ListItem, Paragraph, Wrap};

const PARALLEL_EVENT_STREAM_TITLE: &str = "Parallel Event Stream";

// Inline inspection renders shell overlays inside the app-server main buffer.
// It reuses presentation view builders, then maps those view models into
// frameless sections that can replace the transcript area without popup chrome.
fn inline_overlay_title(name: &'static str) -> Line<'static> {
    AkraTheme::title_line(name, " / inline inspection")
}

fn render_inline_titled_panel(
    frame: &mut Frame<'_>,
    area: Rect,
    title: Line<'static>,
    lines: Vec<Line<'static>>,
    trim: bool,
) {
    InlineTitledPanel::new(title, lines, trim).render(frame, area);
}

fn render_inline_scrolled_panel(
    frame: &mut Frame<'_>,
    area: Rect,
    title: Line<'static>,
    lines: Vec<Line<'static>>,
    scroll_offset: u16,
) {
    InlineScrolledPanel::new(title, lines, scroll_offset).render(frame, area);
}

pub(super) fn draw_inline_shell_inspection(
    frame: &mut Frame<'_>,
    app: &mut NativeTuiApp,
    inspection_area: Rect,
) {
    // The top-level router mirrors ShellOverlay exactly so hidden overlays stay
    // silent and every visible overlay owns a focused inline composition.
    match app.shell_overlay {
        ShellOverlay::Hidden => {}
        ShellOverlay::Startup => draw_inline_startup_inspection(frame, inspection_area, app),
        ShellOverlay::Sessions => draw_inline_session_inspection(frame, inspection_area, app),
        ShellOverlay::ModelSelection => {
            draw_inline_model_selection_inspection(frame, inspection_area, app)
        }
        ShellOverlay::ViewSelection => {
            draw_inline_view_selection_inspection(frame, inspection_area, app)
        }
        ShellOverlay::LanguageSelection => {
            draw_inline_language_selection_inspection(frame, inspection_area, app)
        }
        ShellOverlay::Supersession => {
            draw_inline_supersession_inspection(frame, inspection_area, app)
        }
        ShellOverlay::ParallelPeek => {
            draw_inline_parallel_peek_inspection(frame, inspection_area, app)
        }
        ShellOverlay::Activity => draw_inline_activity_inspection(frame, inspection_area, app),
        ShellOverlay::Help => draw_inline_help_inspection(frame, inspection_area),
        ShellOverlay::Reviews => draw_inline_reviews_inspection(frame, inspection_area, app),
        ShellOverlay::Queue => draw_inline_queue_inspection(frame, inspection_area, app),
        ShellOverlay::DirectionsMaintenance => {
            draw_inline_directions_maintenance_inspection(frame, inspection_area, app)
        }
        ShellOverlay::PlanningInit => {
            draw_inline_planning_init_inspection(frame, inspection_area, app)
        }
        ShellOverlay::Approval => draw_inline_approval_inspection(frame, inspection_area, app),
    }
}

fn draw_inline_activity_inspection(frame: &mut Frame<'_>, area: Rect, app: &mut NativeTuiApp) {
    let selected_kind = app.progressive_activity_overlay_ui_state.selected_kind();
    let (lifecycle_epoch, diff_available, output_available, document) =
        match &app.conversation_state {
            super::ConversationState::Ready(conversation) => {
                let detail = &conversation.progressive_activity_detail;
                (
                    detail.lifecycle_epoch(),
                    detail
                        .document(super::ProgressiveActivityDetailKind::Diff)
                        .is_some(),
                    detail
                        .document(super::ProgressiveActivityDetailKind::Output)
                        .is_some(),
                    detail.document(selected_kind),
                )
            }
            super::ConversationState::Loading | super::ConversationState::Failed(_) => {
                (0, false, false, None)
            }
        };
    app.progressive_activity_overlay_ui_state.select_document(
        lifecycle_epoch,
        document.as_ref().map(|document| document.sequence),
    );
    let document_view = document.as_ref().map(|document| ActivityOverlayDocument {
        sequence: document.sequence,
        text: document.text(),
        source_bytes: document.source_bytes,
        retained_bytes: document.retained_bytes,
        truncated_bytes: document.truncated_bytes,
        history_incomplete: document.history_incomplete,
    });

    // Header copy is bounded independently of the retained document. Build it
    // with a zero-row body first so the real document is scanned only once.
    let header_view = build_activity_overlay_view(
        selected_kind,
        diff_available,
        output_available,
        document_view,
        0,
        area.width,
        0,
    );
    let desired_header_height = count_rendered_inline_rows(&header_view.header_lines, area.width)
        .saturating_add(1)
        .min(usize::from(u16::MAX)) as u16;
    let header_height = desired_header_height.min(area.height.saturating_sub(2));
    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(header_height), Constraint::Min(2)])
        .split(area);
    let body_height = layout[1].height.saturating_sub(1);
    app.progressive_activity_overlay_ui_state
        .sync_viewport(layout[1].width, body_height);
    let requested_page_start = app
        .progressive_activity_overlay_ui_state
        .current_page_start();
    let ActivityOverlayView {
        header_lines,
        detail_title,
        detail_lines,
        current_page_start,
        next_page_start,
    } = build_activity_overlay_view(
        selected_kind,
        diff_available,
        output_available,
        document_view,
        requested_page_start,
        layout[1].width,
        body_height,
    );
    app.progressive_activity_overlay_ui_state
        .set_page_window(current_page_start, next_page_start);

    render_inline_titled_panel(
        frame,
        layout[0],
        inline_overlay_title("Activity"),
        header_lines,
        false,
    );
    render_inline_scrolled_panel(frame, layout[1], detail_title, detail_lines, 0);
}

fn draw_inline_approval_inspection(frame: &mut Frame<'_>, area: Rect, app: &mut NativeTuiApp) {
    let Some((request, requested_scroll_offset, submitted_decision)) =
        (match &app.conversation_state {
            super::ConversationState::Ready(conversation) => conversation
                .pending_approval_request
                .as_ref()
                .map(|request| {
                    (
                        request.clone(),
                        conversation.approval_detail_scroll_offset,
                        conversation.pending_approval_decision(),
                    )
                }),
            super::ConversationState::Loading | super::ConversationState::Failed(_) => None,
        })
    else {
        render_inline_titled_panel(
            frame,
            area,
            inline_overlay_title("Approval"),
            vec![Line::from("The approval request is no longer available.")],
            true,
        );
        return;
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
    let scroll_offset = requested_scroll_offset
        .min(max_scroll)
        .min(u16::MAX as usize) as u16;
    if let super::ConversationState::Ready(conversation) = &mut app.conversation_state {
        conversation.approval_detail_scroll_offset = usize::from(scroll_offset);
    }
    let visible_start = if rendered_detail_rows == 0 {
        0
    } else {
        usize::from(scroll_offset) + 1
    };
    let visible_end = (usize::from(scroll_offset) + visible_detail_rows).min(rendered_detail_rows);
    render_inline_titled_panel(
        frame,
        layout[0],
        inline_overlay_title("Approval Required"),
        header_lines,
        true,
    );
    render_inline_scrolled_panel(
        frame,
        layout[1],
        Line::from(format!(
            "Requested Details / {visible_start}-{visible_end} of {rendered_detail_rows}",
        )),
        detail_lines,
        scroll_offset,
    );
    render_inline_titled_panel(frame, layout[2], Line::from("Decision"), key_lines, true);
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

pub(super) fn draw_inline_parallel_mode_inspection(
    frame: &mut Frame<'_>,
    area: Rect,
    app: &NativeTuiApp,
) {
    draw_inline_supersession_inspection(frame, area, app);
}

fn draw_inline_parallel_peek_inspection(frame: &mut Frame<'_>, area: Rect, app: &NativeTuiApp) {
    let overlay_view = build_parallel_peek_overlay_view(app);
    let step = app.parallel_peek_overlay_ui_state.step();
    let ParallelPeekOverlayView {
        header_lines,
        agent_lines,
        conversation_lines,
        status_lines,
        key_lines,
    } = overlay_view;
    let body_lines = take_panel_body_lines(header_lines);

    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(inline_section_height(&body_lines, 4)),
            Constraint::Min(6),
            Constraint::Length(inline_section_height(&status_lines, 4)),
            Constraint::Length(inline_section_height(&key_lines, 4)),
        ])
        .split(area);

    render_inline_titled_panel(
        frame,
        layout[0],
        inline_overlay_title("Parallel Peek"),
        body_lines,
        true,
    );
    match step {
        ParallelPeekOverlayStep::AgentList => render_inline_scrolled_panel(
            frame,
            layout[1],
            Line::from("Active Agents"),
            agent_lines,
            0,
        ),
        ParallelPeekOverlayStep::ConversationPreview => {
            let scroll_offset = inline_preview_scroll_offset(
                layout[1],
                conversation_lines.len(),
                app.parallel_peek_overlay_ui_state
                    .conversation_scroll_from_bottom(),
            );
            render_inline_scrolled_panel(
                frame,
                layout[1],
                Line::from("Conversation Preview"),
                conversation_lines,
                scroll_offset,
            );
        }
    }
    render_inline_titled_panel(frame, layout[2], Line::from("Status"), status_lines, true);
    render_inline_titled_panel(frame, layout[3], Line::from("Keys"), key_lines, true);
}

fn inline_preview_scroll_offset(area: Rect, line_count: usize, scroll_from_bottom: usize) -> u16 {
    let visible_body_height = area.height.saturating_sub(1) as usize;
    let max_scroll = line_count.saturating_sub(visible_body_height);
    max_scroll
        .saturating_sub(scroll_from_bottom)
        .min(u16::MAX as usize) as u16
}

fn draw_inline_help_inspection(frame: &mut Frame<'_>, area: Rect) {
    let HelpOverlayView {
        header_lines,
        command_lines,
        key_lines,
    } = build_help_overlay_view();
    let body_lines = take_panel_body_lines(header_lines);
    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(inline_section_height(&body_lines, 4)),
            Constraint::Length(inline_section_height(
                &command_lines,
                command_lines.len().saturating_add(1).min(u16::MAX as usize) as u16,
            )),
            Constraint::Length(inline_section_height(&key_lines, 4)),
        ])
        .split(area);

    render_inline_titled_panel(
        frame,
        layout[0],
        inline_overlay_title("Shell Commands"),
        body_lines,
        true,
    );
    render_inline_titled_panel(
        frame,
        layout[1],
        Line::from("Commands"),
        command_lines,
        false,
    );
    render_inline_titled_panel(frame, layout[2], Line::from("Keys"), key_lines, true);
}
fn draw_inline_directions_maintenance_inspection(
    frame: &mut Frame<'_>,
    area: Rect,
    app: &NativeTuiApp,
) {
    // Directions and planning setup both switch into the shared draft editor
    // renderer when their manual editor steps are active.
    if app.directions_maintenance_overlay_ui_state.step()
        == DirectionsMaintenanceOverlayStep::ManualEditor
    {
        draw_inline_directions_draft_editor_inspection(frame, area, app);
        return;
    }
    let overlay_view = build_directions_maintenance_overlay_view(app);
    let DirectionsMaintenanceOverlayView {
        header_lines,
        summary_lines,
        option_lines,
        status_lines,
        key_lines,
    } = overlay_view;
    let body_lines = take_panel_body_lines(header_lines);
    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(inline_section_height(&body_lines, 4)),
            Constraint::Length(inline_section_height(&summary_lines, 5)),
            Constraint::Min(8),
            Constraint::Length(inline_section_height(&status_lines, 5)),
            Constraint::Length(inline_section_height(&key_lines, 4)),
        ])
        .split(area);

    render_inline_titled_panel(
        frame,
        layout[0],
        inline_overlay_title("Directions"),
        body_lines,
        true,
    );
    render_inline_titled_panel(frame, layout[1], Line::from("Summary"), summary_lines, true);
    render_inline_titled_panel(frame, layout[2], Line::from("Options"), option_lines, false);
    render_inline_titled_panel(frame, layout[3], Line::from("Status"), status_lines, true);
    render_inline_titled_panel(frame, layout[4], Line::from("Keys"), key_lines, true);
}
fn draw_inline_startup_inspection(frame: &mut Frame<'_>, area: Rect, app: &NativeTuiApp) {
    let overlay_view = build_startup_overlay_view(app);
    let StartupOverlayView {
        header_lines,
        summary_lines,
        check_lines,
        warning_lines,
        key_lines,
    } = overlay_view;
    let body_lines = take_panel_body_lines(header_lines);
    let check_height = inline_section_height(&check_lines, 10).max(4);
    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(inline_section_height(&body_lines, 4)),
            Constraint::Length(inline_section_height(&summary_lines, 4)),
            Constraint::Min(check_height),
            Constraint::Length(inline_section_height(&warning_lines, 5)),
            Constraint::Length(inline_section_height(&key_lines, 4)),
        ])
        .split(area);

    render_inline_titled_panel(
        frame,
        layout[0],
        inline_overlay_title("Diagnostics"),
        body_lines,
        true,
    );
    render_inline_titled_panel(frame, layout[1], Line::from("Startup"), summary_lines, true);
    render_inline_titled_panel(frame, layout[2], Line::from("Checks"), check_lines, false);
    render_inline_titled_panel(
        frame,
        layout[3],
        Line::from("Warnings"),
        warning_lines,
        true,
    );
    render_inline_titled_panel(frame, layout[4], Line::from("Keys"), key_lines, true);
}
fn draw_inline_session_inspection(frame: &mut Frame<'_>, area: Rect, app: &mut NativeTuiApp) {
    let overlay_view = build_session_overlay_view(app);
    let SessionOverlayView {
        header_lines,
        list_view,
        detail_lines,
        warning_lines,
        key_lines,
    } = overlay_view;
    let body_lines = take_panel_body_lines(header_lines);
    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(inline_section_height(&body_lines, 4)),
            Constraint::Min(8),
            Constraint::Length(inline_section_height(&warning_lines, 5)),
            Constraint::Length(inline_section_height(&key_lines, 4)),
        ])
        .split(area);

    render_inline_titled_panel(
        frame,
        layout[0],
        inline_overlay_title("Recent Sessions"),
        body_lines,
        true,
    );
    // Sessions need horizontal space for list/detail comparison; warnings and
    // keys stay below so degraded catalogs remain visible.
    let content_layout = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(42), Constraint::Percentage(58)])
        .split(layout[1]);

    draw_inline_session_list_panel(frame, content_layout[0], app, list_view);
    render_inline_titled_panel(
        frame,
        content_layout[1],
        Line::from("Selected Session"),
        detail_lines,
        false,
    );

    render_inline_titled_panel(
        frame,
        layout[2],
        Line::from("Session Warnings"),
        warning_lines,
        true,
    );
    render_inline_titled_panel(frame, layout[3], Line::from("Keys"), key_lines, true);
}
fn draw_inline_model_selection_inspection(frame: &mut Frame<'_>, area: Rect, app: &NativeTuiApp) {
    let overlay_view = build_model_selection_overlay_view(app);
    let ModelSelectionOverlayView {
        header_lines,
        model_lines,
        effort_lines,
        status_lines,
        key_lines,
    } = overlay_view;
    let body_lines = take_panel_body_lines(header_lines);
    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(inline_section_height(&body_lines, 4)),
            Constraint::Min(12),
            Constraint::Length(inline_section_height(&status_lines, 4)),
            Constraint::Length(inline_section_height(&key_lines, 3)),
        ])
        .split(area);

    render_inline_titled_panel(
        frame,
        layout[0],
        inline_overlay_title("Select Model and Effort"),
        body_lines,
        true,
    );
    let picker_layout = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(58), Constraint::Percentage(42)])
        .split(layout[1]);
    render_inline_titled_panel(
        frame,
        picker_layout[0],
        Line::from("Models"),
        model_lines,
        false,
    );
    render_inline_titled_panel(
        frame,
        picker_layout[1],
        Line::from("Think Level"),
        effort_lines,
        false,
    );
    render_inline_titled_panel(frame, layout[2], Line::from("Status"), status_lines, true);
    render_inline_titled_panel(frame, layout[3], Line::from("Keys"), key_lines, true);
}
fn draw_inline_view_selection_inspection(frame: &mut Frame<'_>, area: Rect, app: &NativeTuiApp) {
    let overlay_view = build_view_selection_overlay_view(app);
    let ViewSelectionOverlayView {
        header_lines,
        mode_lines,
        status_lines,
        key_lines,
    } = overlay_view;
    let body_lines = take_panel_body_lines(header_lines);
    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(inline_section_height(&body_lines, 4)),
            Constraint::Min(8),
            Constraint::Length(inline_section_height(&status_lines, 4)),
            Constraint::Length(inline_section_height(&key_lines, 3)),
        ])
        .split(area);

    render_inline_titled_panel(
        frame,
        layout[0],
        inline_overlay_title("Select Conversation View"),
        body_lines,
        true,
    );
    render_inline_titled_panel(frame, layout[1], Line::from("Views"), mode_lines, false);
    render_inline_titled_panel(frame, layout[2], Line::from("Status"), status_lines, true);
    render_inline_titled_panel(frame, layout[3], Line::from("Keys"), key_lines, true);
}
fn draw_inline_language_selection_inspection(
    frame: &mut Frame<'_>,
    area: Rect,
    app: &NativeTuiApp,
) {
    let overlay_view = build_language_selection_overlay_view(app);
    let LanguageSelectionOverlayView {
        header_lines,
        language_lines,
        status_lines,
        key_lines,
    } = overlay_view;
    let body_lines = take_panel_body_lines(header_lines);
    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(inline_section_height(&body_lines, 4)),
            Constraint::Min(7),
            Constraint::Length(inline_section_height(&status_lines, 4)),
            Constraint::Length(inline_section_height(&key_lines, 3)),
        ])
        .split(area);

    render_inline_titled_panel(
        frame,
        layout[0],
        inline_overlay_title("Select Language"),
        body_lines,
        true,
    );
    render_inline_titled_panel(
        frame,
        layout[1],
        Line::from("Languages"),
        language_lines,
        false,
    );
    render_inline_titled_panel(frame, layout[2], Line::from("Status"), status_lines, true);
    render_inline_titled_panel(frame, layout[3], Line::from("Keys"), key_lines, true);
}
fn draw_inline_supersession_inspection(frame: &mut Frame<'_>, area: Rect, app: &NativeTuiApp) {
    let overlay_view = build_supersession_overlay_view(app);
    let SupersessionOverlayView {
        header_lines,
        summary_lines,
        capability_lines,
        pool_lines,
        roster_lines,
        detail_lines,
        distributor_lines: _distributor_lines,
        key_lines,
    } = overlay_view;
    let body_lines = take_panel_body_lines(header_lines);
    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(inline_section_height(&body_lines, 4)),
            Constraint::Length(inline_section_height(&summary_lines, 7)),
            Constraint::Length(10),
            Constraint::Min(8),
            Constraint::Length(inline_section_height(&key_lines, 4)),
        ])
        .split(area);

    render_inline_titled_panel(
        frame,
        layout[0],
        inline_overlay_title("Parallel Mode"),
        body_lines,
        true,
    );
    render_inline_titled_panel(
        frame,
        layout[1],
        Line::from("Basic Info"),
        summary_lines,
        true,
    );
    let status_layout = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(34),
            Constraint::Percentage(33),
            Constraint::Percentage(33),
        ])
        .split(layout[2]);

    render_inline_titled_panel(
        frame,
        status_layout[0],
        Line::from("Distributor"),
        capability_lines,
        false,
    );
    render_inline_titled_panel(
        frame,
        status_layout[1],
        Line::from("Pool"),
        pool_lines,
        false,
    );
    render_inline_titled_panel(
        frame,
        status_layout[2],
        Line::from("Orchestrator"),
        roster_lines,
        false,
    );
    render_inline_parallel_event_stream(frame, layout[3], detail_lines);
    render_inline_titled_panel(
        frame,
        layout[4],
        Line::from("Command Hints"),
        key_lines,
        true,
    );
}

fn render_inline_parallel_event_stream(
    frame: &mut Frame<'_>,
    area: Rect,
    lines: Vec<Line<'static>>,
) {
    let stream_visible_rows =
        parallel_event_stream_visible_rows_for_lines(&lines, area.width, area);
    let stream_scroll_offset =
        event_boundary_scroll_offset(&lines, area.width, stream_visible_rows);
    let title = if parallel_event_stream_title_visible(&lines, area.width, area) {
        InlineAppendOnlyStreamTitle::Visible(Line::from(PARALLEL_EVENT_STREAM_TITLE))
    } else {
        /*
         * Architecture contract: a split event stream is not a titled panel.
         * In host-scrollback mode, overflowed rows above this inline area are
         * already durable terminal history. Rendering title chrome here puts a
         * non-event row between two chunks of the same stream, so the live tail
         * must stay data-only.
         */
        InlineAppendOnlyStreamTitle::Hidden
    };
    InlineAppendOnlyStream::new(title, lines, stream_scroll_offset).render(frame, area);
}

fn event_boundary_scroll_offset(lines: &[Line<'static>], width: u16, visible_rows: usize) -> u16 {
    if lines.is_empty() || visible_rows == 0 || width == 0 {
        return 0;
    }

    let total_rendered_rows = count_rendered_inline_rows(lines, width);
    let minimum_scroll_offset = total_rendered_rows.saturating_sub(visible_rows);
    if minimum_scroll_offset == 0 {
        return 0;
    }

    let mut rendered_rows_before_line = 0usize;
    for line in lines {
        let rendered_rows_after_line = rendered_rows_before_line + rendered_line_rows(line, width);
        if minimum_scroll_offset < rendered_rows_after_line {
            return rendered_rows_before_line.min(u16::MAX as usize) as u16;
        }
        if minimum_scroll_offset == rendered_rows_after_line {
            return rendered_rows_after_line.min(u16::MAX as usize) as u16;
        }
        rendered_rows_before_line = rendered_rows_after_line;
    }

    total_rendered_rows.min(u16::MAX as usize) as u16
}

fn parallel_event_stream_title_visible(lines: &[Line<'static>], width: u16, area: Rect) -> bool {
    let titled_body_rows = area.height.saturating_sub(1) as usize;
    count_rendered_inline_rows(lines, width) <= titled_body_rows
}

fn parallel_event_stream_visible_rows_for_lines(
    lines: &[Line<'static>],
    width: u16,
    area: Rect,
) -> usize {
    if parallel_event_stream_title_visible(lines, width, area) {
        area.height.saturating_sub(1) as usize
    } else {
        area.height as usize
    }
}

fn rendered_line_rows(line: &Line<'_>, width: u16) -> usize {
    let line_width = line.width();
    if line_width == 0 {
        1
    } else {
        line_width.div_ceil(width.max(1) as usize)
    }
}

pub(super) fn parallel_event_stream_visible_rows(app: &NativeTuiApp, area: Rect) -> usize {
    let overlay_view = build_supersession_overlay_view(app);
    let SupersessionOverlayView {
        header_lines,
        summary_lines,
        detail_lines,
        key_lines,
        ..
    } = overlay_view;
    let body_lines = take_panel_body_lines(header_lines);
    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(inline_section_height(&body_lines, 4)),
            Constraint::Length(inline_section_height(&summary_lines, 7)),
            Constraint::Length(10),
            Constraint::Min(8),
            Constraint::Length(inline_section_height(&key_lines, 4)),
        ])
        .split(area);

    parallel_event_stream_visible_rows_for_lines(&detail_lines, layout[3].width, layout[3])
}
fn draw_inline_queue_inspection(frame: &mut Frame<'_>, area: Rect, app: &NativeTuiApp) {
    let overlay_view = build_queue_overlay_view(app);
    let QueueOverlayView {
        header_lines,
        summary_lines,
        queue_lines,
        proposal_lines,
        note_lines,
        selected_content_line_index,
        key_lines,
    } = overlay_view;
    let body_lines = take_panel_body_lines(header_lines);
    // Queue, proposal, and note lines are merged into one scrollable section to
    // preserve vertical space in inline mode.
    let mut content_lines = vec![Line::from("Ready Queue")];
    content_lines.extend(queue_lines);
    if !proposal_lines.is_empty() {
        content_lines.push(Line::from("Proposals"));
        content_lines.extend(proposal_lines);
    }
    if !note_lines.is_empty() {
        content_lines.push(Line::from("Notes"));
        content_lines.extend(note_lines);
    }
    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(inline_section_height(&body_lines, 3)),
            Constraint::Length(inline_section_height(&summary_lines, 3)),
            Constraint::Min(4),
            Constraint::Length(inline_section_height(&key_lines, 2)),
        ])
        .split(area);
    let visible_content_rows = layout[2].height.saturating_sub(1) as usize;
    let content_scroll_offset = selected_content_scroll_offset(
        &content_lines,
        selected_content_line_index,
        layout[2].width,
        visible_content_rows,
    )
    .min(u16::MAX as usize) as u16;

    render_inline_titled_panel(
        frame,
        layout[0],
        inline_overlay_title("Planning Queue"),
        body_lines,
        true,
    );
    render_inline_titled_panel(frame, layout[1], Line::from("Summary"), summary_lines, true);
    render_inline_scrolled_panel(
        frame,
        layout[2],
        Line::from("Queue"),
        content_lines,
        content_scroll_offset,
    );
    render_inline_titled_panel(frame, layout[3], Line::from("Keys"), key_lines, true);
}

fn selected_content_scroll_offset(
    content_lines: &[Line<'_>],
    selected_line_index: Option<usize>,
    width: u16,
    visible_rows: usize,
) -> usize {
    selected_line_index
        .and_then(|selected_index| {
            let selected_line = content_lines.get(selected_index)?;
            let selected_start =
                count_rendered_inline_rows(&content_lines[..selected_index], width);
            let selected_rows = rendered_line_rows(selected_line, width);
            let visible_rows = visible_rows.max(1);
            Some(if selected_rows >= visible_rows {
                selected_start
            } else {
                (selected_start + selected_rows).saturating_sub(visible_rows)
            })
        })
        .unwrap_or(0)
}

fn draw_inline_reviews_inspection(frame: &mut Frame<'_>, area: Rect, app: &NativeTuiApp) {
    let overlay_view = build_reviews_overlay_view(app);
    let current_thread_lines = overlay_view.current_thread_section_lines();
    let inbox_lines = overlay_view.inbox_section_lines();
    let history_lines = overlay_view.history_section_lines();
    let body_lines = take_panel_body_lines(overlay_view.header_lines);
    let summary_lines = overlay_view.summary_lines;
    let key_lines = overlay_view.key_lines;
    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(inline_section_height(&body_lines, 4)),
            Constraint::Length(inline_section_height(&summary_lines, 5)),
            Constraint::Min(8),
            Constraint::Length(inline_section_height(&key_lines, 3)),
        ])
        .split(area);

    render_inline_titled_panel(
        frame,
        layout[0],
        inline_overlay_title("Review Center"),
        body_lines,
        true,
    );
    render_inline_titled_panel(frame, layout[1], Line::from("Summary"), summary_lines, true);
    let content_layout = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(34),
            Constraint::Percentage(33),
            Constraint::Percentage(33),
        ])
        .split(layout[2]);
    render_inline_titled_panel(
        frame,
        content_layout[0],
        Line::from("Active Thread"),
        current_thread_lines,
        false,
    );
    render_inline_titled_panel(
        frame,
        content_layout[1],
        Line::from("Inbox"),
        inbox_lines,
        false,
    );
    render_inline_titled_panel(
        frame,
        content_layout[2],
        Line::from("Recent History"),
        history_lines,
        false,
    );
    render_inline_titled_panel(frame, layout[3], Line::from("Keys"), key_lines, true);
}
fn draw_inline_planning_init_inspection(frame: &mut Frame<'_>, area: Rect, app: &NativeTuiApp) {
    // The planning init flow becomes the same file editor used by directions
    // maintenance once it reaches manual editing.
    if app.planning_init_overlay_ui_state.step() == PlanningInitOverlayStep::ManualEditor {
        draw_inline_planning_draft_editor_inspection(frame, area, app);
        return;
    }
    let overlay_view = build_planning_init_overlay_view(app);
    let PlanningInitOverlayView {
        header_lines,
        summary_lines,
        option_lines,
        status_lines,
        key_lines,
    } = overlay_view;
    let body_lines = take_panel_body_lines(header_lines);
    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(inline_section_height(&body_lines, 4)),
            Constraint::Length(inline_section_height(&summary_lines, 5)),
            Constraint::Min(8),
            Constraint::Length(inline_section_height(&status_lines, 5)),
            Constraint::Length(inline_section_height(&key_lines, 4)),
        ])
        .split(area);

    render_inline_titled_panel(
        frame,
        layout[0],
        inline_overlay_title("Planning"),
        body_lines,
        true,
    );
    render_inline_titled_panel(frame, layout[1], Line::from("Summary"), summary_lines, true);
    render_inline_titled_panel(frame, layout[2], Line::from("Options"), option_lines, false);
    render_inline_titled_panel(frame, layout[3], Line::from("Status"), status_lines, true);
    render_inline_titled_panel(frame, layout[4], Line::from("Keys"), key_lines, true);
}
fn draw_inline_planning_draft_editor_inspection(
    frame: &mut Frame<'_>,
    area: Rect,
    app: &NativeTuiApp,
) {
    draw_inline_draft_editor_inspection(frame, area, app, "Planning Draft");
}
fn draw_inline_draft_editor_inspection(
    frame: &mut Frame<'_>,
    area: Rect,
    app: &NativeTuiApp,
    title: &'static str,
) {
    // The editor height calculation reserves room for files, status, and keys
    // while still guaranteeing at least one visible editor content row.
    let editor_height = area.height.saturating_sub(14).max(6);
    let editor_content_height = editor_height.saturating_sub(1).max(1);
    let Some(overlay_view) = build_planning_draft_editor_overlay_view(app, editor_content_height)
    else {
        return;
    };
    let PlanningDraftEditorOverlayView {
        header_lines,
        file_lines,
        editor_title,
        editor_lines,
        editor_scroll,
        editor_cursor_offset,
        status_lines,
        key_lines,
    } = overlay_view;
    let body_lines = take_panel_body_lines(header_lines);
    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(inline_section_height(&body_lines, 4)),
            Constraint::Length(inline_section_height(&file_lines, 5)),
            Constraint::Min(editor_height),
            Constraint::Length(inline_section_height(&status_lines, 6)),
            Constraint::Length(inline_section_height(&key_lines, 5)),
        ])
        .split(area);

    render_inline_titled_panel(
        frame,
        layout[0],
        inline_overlay_title(title),
        body_lines,
        true,
    );
    render_inline_titled_panel(frame, layout[1], Line::from("Files"), file_lines, true);
    render_inline_scrolled_panel(
        frame,
        layout[2],
        Line::from(editor_title),
        editor_lines,
        editor_scroll,
    );
    // Cursor placement happens after rendering because the editor section title
    // consumes the first row of the split section.
    let editor_content_area = split_inline_section(layout[2])[1];
    set_cursor_if_visible(frame, editor_content_area, editor_cursor_offset);
    render_inline_titled_panel(frame, layout[3], Line::from("Status"), status_lines, true);
    render_inline_titled_panel(frame, layout[4], Line::from("Keys"), key_lines, true);
}
fn draw_inline_directions_draft_editor_inspection(
    frame: &mut Frame<'_>,
    area: Rect,
    app: &NativeTuiApp,
) {
    draw_inline_draft_editor_inspection(frame, area, app, "Directions Support Draft");
}
fn draw_inline_session_list_panel(
    frame: &mut Frame<'_>,
    area: Rect,
    app: &mut NativeTuiApp,
    list_view: OverlayListView,
) {
    let section_layout = split_inline_section(area);
    frame.render_widget(
        Paragraph::new(vec![Line::from("Threads")]),
        section_layout[0],
    );
    if let Some(message_lines) = list_view.message_lines {
        // Unsupported or partial catalogs provide explanatory copy instead of
        // navigable rows, and should not mutate the list selection state.
        frame.render_widget(
            Paragraph::new(message_lines).wrap(Wrap { trim: true }),
            section_layout[1],
        );
        return;
    }
    let list = List::new(
        list_view
            .items
            .into_iter()
            .map(|item| ListItem::new(item.lines)),
    )
    .highlight_style(AkraTheme::selected())
    .highlight_symbol(AkraTheme::list_highlight_symbol());

    app.session_overlay_ui_state
        .sync_selected_session(list_view.selected_index);
    frame.render_stateful_widget(
        list,
        section_layout[1],
        &mut app.session_overlay_ui_state.list_state,
    );
}

#[cfg(test)]
mod tests {
    use ratatui::text::Line;

    use super::{event_boundary_scroll_offset, selected_content_scroll_offset};

    #[test]
    fn event_boundary_scroll_offset_keeps_wrapped_event_intact() {
        let lines = vec![Line::from("alpha beta gamma"), Line::from("tail event")];

        assert_eq!(
            event_boundary_scroll_offset(&lines, 10, 2),
            0,
            "boundary inside the first wrapped event should keep the whole event live"
        );
        assert_eq!(
            event_boundary_scroll_offset(&lines, 10, 1),
            2,
            "boundary exactly after the first wrapped event may start at the next event"
        );
    }

    #[test]
    fn selected_content_scroll_keeps_marker_when_row_is_taller_than_viewport() {
        let lines = vec![
            Line::from("heading"),
            Line::from(format!("> selected {}", "detail ".repeat(20))),
        ];

        assert_eq!(
            selected_content_scroll_offset(&lines, Some(1), 48, 2),
            1,
            "a tall selected row should start at its marker instead of its wrapped tail"
        );
    }
}
