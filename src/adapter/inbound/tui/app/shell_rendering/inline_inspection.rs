use super::super::parallel_supervisor_events::{
    rendered_parallel_event_line_rows, rendered_parallel_event_tail_start_index,
};
use super::super::shell_presentation::{
    ActivityOverlayView, DirectionsMaintenanceOverlayView, HelpOverlayView,
    LanguageSelectionOverlayView, ModelSelectionOverlayView, OverlayListView,
    ParallelPeekOverlayView, PlanningDraftEditorOverlayView, PlanningInitOverlayView,
    QueueOverlayView, ReviewsOverlayView, SessionOverlayView, StartupOverlayView,
    SupersessionOverlayView, ViewSelectionOverlayView,
};
use super::super::{AkraTheme, ParallelPeekOverlayStep, TuiLanguage};
use super::inline_layout::{
    InlineAppendOnlyStream, InlineAppendOnlyStreamTitle, InlineScrolledPanel, InlineTitledPanel,
    count_rendered_inline_rows, inline_section_height, set_cursor_if_visible, split_inline_section,
    take_panel_body_lines,
};
use super::{ApprovalInlineScreenModel, InlineInspectionFrameModel};
use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::text::Line;
use ratatui::widgets::{List, ListItem, ListState, Paragraph, Wrap};

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
    inspection_area: Rect,
    model: InlineInspectionFrameModel,
) -> Option<ListState> {
    match model {
        InlineInspectionFrameModel::Conversation => {}
        InlineInspectionFrameModel::ParallelSupervisor(view)
        | InlineInspectionFrameModel::Supersession(view) => {
            draw_inline_supersession_inspection(frame, inspection_area, view)
        }
        InlineInspectionFrameModel::Startup(view) => {
            draw_inline_startup_inspection(frame, inspection_area, view)
        }
        InlineInspectionFrameModel::Sessions { view, list_state } => {
            return Some(draw_inline_session_inspection(
                frame,
                inspection_area,
                view,
                list_state,
            ));
        }
        InlineInspectionFrameModel::ModelSelection(view) => {
            draw_inline_model_selection_inspection(frame, inspection_area, view)
        }
        InlineInspectionFrameModel::ViewSelection(view) => {
            draw_inline_view_selection_inspection(frame, inspection_area, view)
        }
        InlineInspectionFrameModel::LanguageSelection(view) => {
            draw_inline_language_selection_inspection(frame, inspection_area, view)
        }
        InlineInspectionFrameModel::ParallelPeek {
            view,
            step,
            scroll_from_bottom,
        } => draw_inline_parallel_peek_inspection(
            frame,
            inspection_area,
            view,
            step,
            scroll_from_bottom,
        ),
        InlineInspectionFrameModel::Activity(view) => {
            draw_inline_activity_inspection(frame, inspection_area, view)
        }
        InlineInspectionFrameModel::Help {
            language,
            view,
            scroll_offset,
        } => draw_inline_help_inspection(frame, inspection_area, language, view, scroll_offset),
        InlineInspectionFrameModel::Reviews(view) => {
            draw_inline_reviews_inspection(frame, inspection_area, view)
        }
        InlineInspectionFrameModel::Queue(view) => {
            draw_inline_queue_inspection(frame, inspection_area, view)
        }
        InlineInspectionFrameModel::Directions(view) => {
            draw_inline_directions_maintenance_inspection(frame, inspection_area, view)
        }
        InlineInspectionFrameModel::PlanningInit(view) => {
            draw_inline_planning_init_inspection(frame, inspection_area, view)
        }
        InlineInspectionFrameModel::DraftEditor { title, view } => {
            draw_inline_draft_editor_inspection(frame, inspection_area, title, view)
        }
        InlineInspectionFrameModel::Approval(model) => {
            draw_inline_approval_inspection(frame, inspection_area, model)
        }
    }
    None
}

fn draw_inline_activity_inspection(frame: &mut Frame<'_>, area: Rect, view: ActivityOverlayView) {
    let ActivityOverlayView {
        header_lines,
        detail_title,
        detail_lines,
        key_lines,
        ..
    } = view;
    let desired_header_height = count_rendered_inline_rows(&header_lines, area.width)
        .saturating_add(1)
        .min(usize::from(u16::MAX)) as u16;
    let key_height = inline_section_height(&key_lines, 4).min(area.height.saturating_sub(4));
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

    render_inline_titled_panel(
        frame,
        layout[0],
        inline_overlay_title("Activity"),
        header_lines,
        false,
    );
    render_inline_scrolled_panel(frame, layout[1], detail_title, detail_lines, 0);
    render_inline_titled_panel(frame, layout[2], Line::from("Keys"), key_lines, true);
}

fn draw_inline_approval_inspection(
    frame: &mut Frame<'_>,
    area: Rect,
    model: ApprovalInlineScreenModel,
) {
    if !model.available {
        render_inline_titled_panel(
            frame,
            area,
            inline_overlay_title("Approval"),
            vec![Line::from("The approval request is no longer available.")],
            true,
        );
        return;
    }
    let ApprovalInlineScreenModel {
        header_lines,
        detail_lines,
        key_lines,
        scroll_offset,
        visible_start,
        visible_end,
        rendered_detail_rows,
        ..
    } = model;
    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(wrapped_approval_panel_height(&header_lines, area.width, 6)),
            Constraint::Min(3),
            Constraint::Length(wrapped_approval_panel_height(&key_lines, area.width, 4)),
        ])
        .split(area);
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

fn draw_inline_parallel_peek_inspection(
    frame: &mut Frame<'_>,
    area: Rect,
    overlay_view: ParallelPeekOverlayView,
    step: ParallelPeekOverlayStep,
    scroll_from_bottom: usize,
) {
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
                scroll_from_bottom,
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

fn draw_inline_help_inspection(
    frame: &mut Frame<'_>,
    area: Rect,
    language: TuiLanguage,
    view: HelpOverlayView,
    scroll_offset: u16,
) {
    let HelpOverlayView {
        header_lines,
        command_lines,
        key_lines,
    } = view;
    let body_lines = take_panel_body_lines(header_lines);
    let key_height = count_rendered_inline_rows(&key_lines, area.width)
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

    render_inline_titled_panel(
        frame,
        layout[0],
        AkraTheme::title_line(
            language.shell_commands_panel_title(),
            language.shell_command_help_context(),
        ),
        body_lines,
        true,
    );
    render_inline_scrolled_panel(
        frame,
        layout[1],
        Line::from(language.commands_section_title()),
        command_lines,
        scroll_offset,
    );
    render_inline_titled_panel(
        frame,
        layout[2],
        Line::from(language.keys_section_title()),
        key_lines,
        true,
    );
}
fn draw_inline_directions_maintenance_inspection(
    frame: &mut Frame<'_>,
    area: Rect,
    overlay_view: DirectionsMaintenanceOverlayView,
) {
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
fn draw_inline_startup_inspection(
    frame: &mut Frame<'_>,
    area: Rect,
    overlay_view: StartupOverlayView,
) {
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
fn draw_inline_session_inspection(
    frame: &mut Frame<'_>,
    area: Rect,
    overlay_view: SessionOverlayView,
    list_state: ListState,
) -> ListState {
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

    let list_state =
        draw_inline_session_list_panel(frame, content_layout[0], list_state, list_view);
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
    list_state
}
fn draw_inline_model_selection_inspection(
    frame: &mut Frame<'_>,
    area: Rect,
    overlay_view: ModelSelectionOverlayView,
) {
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
fn draw_inline_view_selection_inspection(
    frame: &mut Frame<'_>,
    area: Rect,
    overlay_view: ViewSelectionOverlayView,
) {
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
    overlay_view: LanguageSelectionOverlayView,
) {
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
fn draw_inline_supersession_inspection(
    frame: &mut Frame<'_>,
    area: Rect,
    overlay_view: SupersessionOverlayView,
) {
    if !overlay_view.focused_full_viewport {
        let layout = passive_supersession_inspection_layout(&overlay_view, area);
        render_inline_parallel_event_stream(frame, layout.events, overlay_view.event_lines);
        render_inline_titled_panel(
            frame,
            layout.keys,
            Line::from("Command Hints"),
            overlay_view.key_lines,
            true,
        );
        return;
    }

    let layout = supersession_inspection_layout(&overlay_view, area);
    let SupersessionOverlayView {
        focused_full_viewport: _,
        header_lines,
        overview_lines,
        accepted_queue_lines,
        timeline_lines,
        lane_lines,
        compact_lane_lines,
        selected_lane_lines,
        compact_selected_lane_lines,
        event_lines,
        key_lines,
    } = overlay_view;
    let body_lines = take_panel_body_lines(header_lines);
    render_inline_titled_panel(
        frame,
        layout.header,
        inline_overlay_title("Parallel Operations"),
        body_lines,
        true,
    );
    render_inline_titled_panel(
        frame,
        layout.overview,
        Line::from("Overview"),
        overview_lines,
        true,
    );

    if area.width >= 116 {
        let compact_columns = layout.operations.height < 12;
        let operations_layout = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Percentage(17),
                Constraint::Length(1),
                Constraint::Percentage(50),
                Constraint::Length(1),
                Constraint::Min(0),
            ])
            .split(layout.operations);
        render_inline_supersession_panel(
            frame,
            operations_layout[0],
            Line::from("Lifecycle"),
            timeline_lines,
        );
        render_inline_supersession_panel(
            frame,
            operations_layout[2],
            Line::from("Agent Lanes"),
            if compact_columns {
                compact_lane_lines
            } else {
                lane_lines
            },
        );
        render_inline_supersession_panel(
            frame,
            operations_layout[4],
            Line::from("Selected Lane"),
            if compact_columns {
                compact_selected_lane_lines
            } else {
                selected_lane_lines
            },
        );
    } else {
        let lane_height = inline_section_height(&compact_lane_lines, 4)
            .min(layout.operations.height.saturating_sub(2).max(2));
        let operations_layout = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(lane_height), Constraint::Min(2)])
            .split(layout.operations);
        render_inline_supersession_panel(
            frame,
            operations_layout[0],
            Line::from("Agent Lanes"),
            compact_lane_lines,
        );
        let mut compact_detail = compact_selected_lane_lines;
        if !timeline_lines.is_empty() {
            compact_detail.push(Line::styled("LIFECYCLE", AkraTheme::accent()));
            compact_detail.extend(timeline_lines);
        }
        render_inline_supersession_panel(
            frame,
            operations_layout[1],
            Line::from("Selected Lane"),
            compact_detail,
        );
    }

    render_inline_supersession_panel(
        frame,
        layout.accepted_queue,
        Line::from("Accepted Queue"),
        accepted_queue_lines,
    );
    render_inline_parallel_event_stream(frame, layout.events, event_lines);
    render_inline_titled_panel(
        frame,
        layout.keys,
        Line::from("Command Hints"),
        key_lines,
        true,
    );
}

#[derive(Clone, Copy)]
struct SupersessionInspectionLayout {
    header: Rect,
    overview: Rect,
    operations: Rect,
    accepted_queue: Rect,
    events: Rect,
    keys: Rect,
}

fn passive_supersession_inspection_layout(
    overlay_view: &SupersessionOverlayView,
    area: Rect,
) -> SupersessionInspectionLayout {
    let sections = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(1),
            Constraint::Length(inline_section_height(&overlay_view.key_lines, 3)),
        ])
        .split(area);
    SupersessionInspectionLayout {
        header: Rect::default(),
        overview: Rect::default(),
        operations: Rect::default(),
        accepted_queue: Rect::default(),
        events: sections[0],
        keys: sections[1],
    }
}

fn supersession_inspection_layout(
    overlay_view: &SupersessionOverlayView,
    area: Rect,
) -> SupersessionInspectionLayout {
    let header_body_lines = overlay_view.header_lines.len().saturating_sub(1);
    let header_height = (header_body_lines + 1).clamp(2, 3) as u16;
    if area.height <= 18 {
        let sections = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(2),
                Constraint::Length(3),
                Constraint::Length(6),
                Constraint::Length(2),
                Constraint::Min(1),
                Constraint::Length(2),
            ])
            .split(area);
        return SupersessionInspectionLayout {
            header: sections[0],
            overview: sections[1],
            operations: sections[2],
            accepted_queue: sections[3],
            events: sections[4],
            keys: sections[5],
        };
    }
    let overview_height = inline_section_height(&overlay_view.overview_lines, 5);
    let queue_height = inline_section_height(&overlay_view.accepted_queue_lines, 4);
    let key_height = inline_section_height(&overlay_view.key_lines, 3);
    let operations_height = if overlay_view.focused_full_viewport && area.width >= 116 {
        if area.height >= 34 { 16 } else { 13 }
    } else if overlay_view.focused_full_viewport {
        if area.height >= 32 {
            12
        } else if area.height >= 26 {
            10
        } else {
            7
        }
    } else if area.height >= 26 {
        10
    } else {
        7
    };
    let sections = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(header_height),
            Constraint::Length(overview_height),
            Constraint::Length(operations_height),
            Constraint::Length(queue_height),
            Constraint::Min(3),
            Constraint::Length(key_height),
        ])
        .split(area);
    SupersessionInspectionLayout {
        header: sections[0],
        overview: sections[1],
        operations: sections[2],
        accepted_queue: sections[3],
        events: sections[4],
        keys: sections[5],
    }
}

fn render_inline_supersession_panel(
    frame: &mut Frame<'_>,
    area: Rect,
    title: Line<'static>,
    lines: Vec<Line<'static>>,
) {
    let selected_line_index = lines.iter().rposition(|line| {
        let text = line.to_string();
        text.starts_with("> ") || text.starts_with('▌')
    });
    let visible_rows = area.height.saturating_sub(1) as usize;
    let scroll_offset =
        selected_content_scroll_offset(&lines, selected_line_index, area.width, visible_rows)
            .min(u16::MAX as usize) as u16;
    render_inline_scrolled_panel(frame, area, title, lines, scroll_offset);
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

    let live_start = rendered_parallel_event_tail_start_index(lines, visible_rows, width);
    lines[..live_start]
        .iter()
        .map(|line| rendered_parallel_event_line_rows(line, width))
        .sum::<usize>()
        .min(u16::MAX as usize) as u16
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
    count_rendered_inline_rows(std::slice::from_ref(line), width).max(1)
}

pub(super) fn parallel_event_stream_visible_rows(
    overlay_view: &SupersessionOverlayView,
    area: Rect,
) -> usize {
    let layout = if overlay_view.focused_full_viewport {
        supersession_inspection_layout(overlay_view, area)
    } else {
        passive_supersession_inspection_layout(overlay_view, area)
    };
    parallel_event_stream_visible_rows_for_lines(
        &overlay_view.event_lines,
        layout.events.width,
        layout.events,
    )
}
fn draw_inline_queue_inspection(frame: &mut Frame<'_>, area: Rect, overlay_view: QueueOverlayView) {
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
    let mut content_lines = Vec::new();
    content_lines.extend(queue_lines);
    if !proposal_lines.is_empty() {
        content_lines.push(Line::from("Proposals"));
        content_lines.extend(proposal_lines);
    }
    if !note_lines.is_empty() {
        content_lines.push(Line::from("Notes"));
        content_lines.extend(note_lines);
    }
    let header_height = if body_lines.is_empty() {
        1
    } else {
        inline_section_height(&body_lines, 3)
    };
    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(header_height),
            Constraint::Length(inline_section_height(&summary_lines, 3)),
            Constraint::Min(4),
            Constraint::Length(inline_section_height(&key_lines, 4)),
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

    if body_lines.is_empty() {
        frame.render_widget(
            Paragraph::new(inline_overlay_title("Planning Queue")),
            layout[0],
        );
    } else {
        render_inline_titled_panel(
            frame,
            layout[0],
            inline_overlay_title("Planning Queue"),
            body_lines,
            true,
        );
    }
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

fn draw_inline_reviews_inspection(
    frame: &mut Frame<'_>,
    area: Rect,
    overlay_view: ReviewsOverlayView,
) {
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
fn draw_inline_planning_init_inspection(
    frame: &mut Frame<'_>,
    area: Rect,
    overlay_view: PlanningInitOverlayView,
) {
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
fn draw_inline_draft_editor_inspection(
    frame: &mut Frame<'_>,
    area: Rect,
    title: &'static str,
    overlay_view: Option<PlanningDraftEditorOverlayView>,
) {
    let Some(overlay_view) = overlay_view else {
        return;
    };
    // The editor height calculation reserves room for files, status, and keys
    // while still guaranteeing at least one visible editor content row.
    let editor_height = area.height.saturating_sub(14).max(6);
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
fn draw_inline_session_list_panel(
    frame: &mut Frame<'_>,
    area: Rect,
    mut list_state: ListState,
    list_view: OverlayListView,
) -> ListState {
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
        return list_state;
    }
    let list = List::new(
        list_view
            .items
            .into_iter()
            .map(|item| ListItem::new(item.lines)),
    )
    .highlight_style(AkraTheme::selected())
    .highlight_symbol(AkraTheme::list_highlight_symbol());

    list_state.select(list_view.selected_index);
    frame.render_stateful_widget(list, section_layout[1], &mut list_state);
    list_state
}

#[cfg(test)]
mod tests {
    use ratatui::text::Line;

    use super::{event_boundary_scroll_offset, selected_content_scroll_offset};

    #[test]
    fn event_boundary_scroll_offset_keeps_wrapped_event_intact() {
        let lines = vec![Line::from("123456 123456 123456"), Line::from("tail event")];

        assert_eq!(
            event_boundary_scroll_offset(&lines, 10, 3),
            3,
            "an event that does not fully fit in the live suffix must remain durable"
        );
        assert_eq!(
            event_boundary_scroll_offset(&lines, 10, 1),
            3,
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
