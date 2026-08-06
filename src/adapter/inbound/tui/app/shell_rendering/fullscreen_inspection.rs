use super::super::parallel_stream_view::ParallelLiveStreamModel;
use super::super::shell_presentation::{
    ActivityOverlayView, DirectionsMaintenanceOverlayView, HelpOverlayView,
    LanguageSelectionOverlayView, ModelSelectionOverlayView, OverlayListView,
    ParallelPeekOverlayView, PlanningDraftEditorOverlayView, PlanningInitOverlayView,
    QueueOverlayView, ReviewsOverlayView, SessionOverlayView, StartupOverlayView,
    SupersessionOverlayView, ViewSelectionOverlayView, WorkCenterOverlayView,
};
use super::super::{AkraTheme, ParallelPeekOverlayStep, TuiLanguage};
use super::fullscreen_layout::{
    FullscreenAppendOnlyStream, FullscreenAppendOnlyStreamTitle, FullscreenScrolledPanel,
    FullscreenTitledPanel, count_wrapped_rows, fullscreen_section_height, set_cursor_if_visible,
    split_fullscreen_section, take_panel_body_lines,
};
use super::{ApprovalFullscreenScreenModel, FullscreenInspectionFrameModel};
use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::text::Line;
use ratatui::widgets::{List, ListItem, ListState, Paragraph, Wrap};

const PARALLEL_EVENT_STREAM_TITLE: &str = "Parallel Event Stream";

// Focused views replace the transcript area without leaving the fullscreen
// application. Their presentation models remain independent from terminal I/O.
fn fullscreen_overlay_title(name: &'static str) -> Line<'static> {
    AkraTheme::title_line(name, " / focused view")
}

fn render_fullscreen_titled_panel(
    frame: &mut Frame<'_>,
    area: Rect,
    title: Line<'static>,
    lines: Vec<Line<'static>>,
    trim: bool,
) {
    FullscreenTitledPanel::new(title, lines, trim).render(frame, area);
}

fn render_fullscreen_scrolled_panel(
    frame: &mut Frame<'_>,
    area: Rect,
    title: Line<'static>,
    lines: Vec<Line<'static>>,
    scroll_offset: u16,
) {
    FullscreenScrolledPanel::new(title, lines, scroll_offset).render(frame, area);
}

pub(super) fn draw_fullscreen_shell_inspection(
    frame: &mut Frame<'_>,
    inspection_area: Rect,
    model: FullscreenInspectionFrameModel,
) -> Option<ListState> {
    match model {
        FullscreenInspectionFrameModel::Conversation => {}
        FullscreenInspectionFrameModel::Supersession(view) => {
            draw_fullscreen_supersession_inspection(frame, inspection_area, view)
        }
        FullscreenInspectionFrameModel::Startup {
            view,
            warning_scroll_offset,
        } => {
            draw_fullscreen_startup_inspection(frame, inspection_area, view, warning_scroll_offset)
        }
        FullscreenInspectionFrameModel::Sessions { view, list_state } => {
            return Some(draw_fullscreen_session_inspection(
                frame,
                inspection_area,
                view,
                list_state,
            ));
        }
        FullscreenInspectionFrameModel::ModelSelection(view) => {
            draw_fullscreen_model_selection_inspection(frame, inspection_area, view)
        }
        FullscreenInspectionFrameModel::ViewSelection(view) => {
            draw_fullscreen_view_selection_inspection(frame, inspection_area, view)
        }
        FullscreenInspectionFrameModel::LanguageSelection(view) => {
            draw_fullscreen_language_selection_inspection(frame, inspection_area, view)
        }
        FullscreenInspectionFrameModel::ParallelPeek {
            view,
            step,
            scroll_from_bottom,
        } => draw_fullscreen_parallel_peek_inspection(
            frame,
            inspection_area,
            view,
            step,
            scroll_from_bottom,
        ),
        FullscreenInspectionFrameModel::WorkCenter(view) => {
            draw_fullscreen_work_center_inspection(frame, inspection_area, view)
        }
        FullscreenInspectionFrameModel::Activity(view) => {
            draw_fullscreen_activity_inspection(frame, inspection_area, view)
        }
        FullscreenInspectionFrameModel::Help {
            language,
            view,
            scroll_offset,
        } => draw_fullscreen_help_inspection(frame, inspection_area, language, view, scroll_offset),
        FullscreenInspectionFrameModel::Reviews(view) => {
            draw_fullscreen_reviews_inspection(frame, inspection_area, view)
        }
        FullscreenInspectionFrameModel::Queue(view) => {
            draw_fullscreen_queue_inspection(frame, inspection_area, view)
        }
        FullscreenInspectionFrameModel::Directions(view) => {
            draw_fullscreen_directions_maintenance_inspection(frame, inspection_area, view)
        }
        FullscreenInspectionFrameModel::PlanningInit(view) => {
            draw_fullscreen_planning_init_inspection(frame, inspection_area, view)
        }
        FullscreenInspectionFrameModel::DraftEditor { title, view } => {
            draw_fullscreen_draft_editor_inspection(frame, inspection_area, title, view)
        }
        FullscreenInspectionFrameModel::Approval(model) => {
            draw_fullscreen_approval_inspection(frame, inspection_area, model)
        }
    }
    None
}

fn draw_fullscreen_work_center_inspection(
    frame: &mut Frame<'_>,
    area: Rect,
    view: WorkCenterOverlayView,
) {
    let WorkCenterOverlayView {
        header_lines,
        summary_lines,
        item_lines,
        detail_lines,
        key_lines,
    } = view;
    let body_lines = take_panel_body_lines(header_lines);
    if area.height <= 18 {
        // The ordinary fullscreen shell reserves a dense status/composer tail. A
        // compact Work Center therefore removes per-section title rows and
        // spends every remaining row on the five authority summaries, selected
        // detail, and keys. This keeps the whole control surface visible in an
        // 11-row inspection area without hiding the shell's live status rail.
        let mut lines = vec![fullscreen_overlay_title("Work Center")];
        lines.extend(summary_lines);
        lines.extend(item_lines);
        lines.extend(detail_lines);
        lines.extend(key_lines);
        frame.render_widget(Paragraph::new(lines), area);
        return;
    }
    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(fullscreen_section_height(&body_lines, 3)),
            Constraint::Length(fullscreen_section_height(&summary_lines, 3)),
            Constraint::Min(6),
            Constraint::Length(fullscreen_section_height(&detail_lines, 4)),
            Constraint::Length(fullscreen_section_height(&key_lines, 4)),
        ])
        .split(area);

    render_fullscreen_titled_panel(
        frame,
        layout[0],
        fullscreen_overlay_title("Work Center"),
        body_lines,
        true,
    );
    render_fullscreen_titled_panel(
        frame,
        layout[1],
        Line::from("Overview"),
        summary_lines,
        true,
    );
    render_fullscreen_titled_panel(frame, layout[2], Line::from("Work"), item_lines, false);
    render_fullscreen_titled_panel(
        frame,
        layout[3],
        Line::from("Selected Detail"),
        detail_lines,
        true,
    );
    render_fullscreen_titled_panel(frame, layout[4], Line::from("Keys"), key_lines, true);
}

fn draw_fullscreen_activity_inspection(
    frame: &mut Frame<'_>,
    area: Rect,
    view: ActivityOverlayView,
) {
    let ActivityOverlayView {
        header_lines,
        detail_title,
        detail_lines,
        key_lines,
        ..
    } = view;
    let desired_header_height = count_wrapped_rows(&header_lines, area.width)
        .saturating_add(1)
        .min(usize::from(u16::MAX)) as u16;
    let key_height = fullscreen_section_height(&key_lines, 4).min(area.height.saturating_sub(4));
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

    render_fullscreen_titled_panel(
        frame,
        layout[0],
        fullscreen_overlay_title("Activity"),
        header_lines,
        false,
    );
    render_fullscreen_scrolled_panel(frame, layout[1], detail_title, detail_lines, 0);
    render_fullscreen_titled_panel(frame, layout[2], Line::from("Keys"), key_lines, true);
}

fn draw_fullscreen_approval_inspection(
    frame: &mut Frame<'_>,
    area: Rect,
    model: ApprovalFullscreenScreenModel,
) {
    if !model.available {
        render_fullscreen_titled_panel(
            frame,
            area,
            fullscreen_overlay_title("Approval"),
            vec![Line::from("The approval request is no longer available.")],
            true,
        );
        return;
    }
    let ApprovalFullscreenScreenModel {
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
    render_fullscreen_titled_panel(
        frame,
        layout[0],
        fullscreen_overlay_title("Approval Required"),
        header_lines,
        true,
    );
    render_fullscreen_scrolled_panel(
        frame,
        layout[1],
        Line::from(format!(
            "Requested Details / {visible_start}-{visible_end} of {rendered_detail_rows}",
        )),
        detail_lines,
        scroll_offset,
    );
    render_fullscreen_titled_panel(frame, layout[2], Line::from("Decision"), key_lines, true);
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

fn draw_fullscreen_parallel_peek_inspection(
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
            Constraint::Length(fullscreen_section_height(&body_lines, 4)),
            Constraint::Min(6),
            Constraint::Length(fullscreen_section_height(&status_lines, 4)),
            Constraint::Length(fullscreen_section_height(&key_lines, 4)),
        ])
        .split(area);

    render_fullscreen_titled_panel(
        frame,
        layout[0],
        fullscreen_overlay_title("Parallel Peek"),
        body_lines,
        true,
    );
    match step {
        ParallelPeekOverlayStep::AgentList => render_fullscreen_scrolled_panel(
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
            render_fullscreen_scrolled_panel(
                frame,
                layout[1],
                Line::from("Conversation Preview"),
                conversation_lines,
                scroll_offset,
            );
        }
    }
    render_fullscreen_titled_panel(frame, layout[2], Line::from("Status"), status_lines, true);
    render_fullscreen_titled_panel(frame, layout[3], Line::from("Keys"), key_lines, true);
}

fn inline_preview_scroll_offset(area: Rect, line_count: usize, scroll_from_bottom: usize) -> u16 {
    let visible_body_height = area.height.saturating_sub(1) as usize;
    let max_scroll = line_count.saturating_sub(visible_body_height);
    max_scroll
        .saturating_sub(scroll_from_bottom)
        .min(u16::MAX as usize) as u16
}

fn draw_fullscreen_help_inspection(
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
    let key_height = count_wrapped_rows(&key_lines, area.width)
        .saturating_add(1)
        .clamp(2, 4) as u16;
    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(fullscreen_section_height(&body_lines, 4)),
            Constraint::Min(2),
            Constraint::Length(key_height),
        ])
        .split(area);

    render_fullscreen_titled_panel(
        frame,
        layout[0],
        AkraTheme::title_line(
            language.shell_commands_panel_title(),
            language.shell_command_help_context(),
        ),
        body_lines,
        true,
    );
    render_fullscreen_scrolled_panel(
        frame,
        layout[1],
        Line::from(language.commands_section_title()),
        command_lines,
        scroll_offset,
    );
    render_fullscreen_titled_panel(
        frame,
        layout[2],
        Line::from(language.keys_section_title()),
        key_lines,
        true,
    );
}
fn draw_fullscreen_directions_maintenance_inspection(
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
            Constraint::Length(fullscreen_section_height(&body_lines, 4)),
            Constraint::Length(fullscreen_section_height(&summary_lines, 5)),
            Constraint::Min(8),
            Constraint::Length(fullscreen_section_height(&status_lines, 5)),
            Constraint::Length(fullscreen_section_height(&key_lines, 4)),
        ])
        .split(area);

    render_fullscreen_titled_panel(
        frame,
        layout[0],
        fullscreen_overlay_title("Directions"),
        body_lines,
        true,
    );
    render_fullscreen_titled_panel(frame, layout[1], Line::from("Summary"), summary_lines, true);
    render_fullscreen_titled_panel(frame, layout[2], Line::from("Options"), option_lines, false);
    render_fullscreen_titled_panel(frame, layout[3], Line::from("Status"), status_lines, true);
    render_fullscreen_titled_panel(frame, layout[4], Line::from("Keys"), key_lines, true);
}
fn draw_fullscreen_startup_inspection(
    frame: &mut Frame<'_>,
    area: Rect,
    overlay_view: StartupOverlayView,
    warning_scroll_offset: u16,
) {
    let StartupOverlayView {
        header_lines,
        summary_lines,
        check_lines,
        warning_lines,
        key_lines,
    } = overlay_view;
    let body_lines = take_panel_body_lines(header_lines);
    if !warning_lines.is_empty() {
        /*
         * Attention diagnostics are the target of the ribbon's Ctrl+D action.
         * Keep their raw, terminal-safe payload and the recovery keys above the
         * longer prerequisite list so the action remains truthful in the
         * bounded fullscreen inspection viewport.
         */
        let layout = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(fullscreen_section_height(&body_lines, 4)),
                Constraint::Length(fullscreen_section_height(&summary_lines, 2)),
                Constraint::Length(fullscreen_section_height(&warning_lines, 5)),
                Constraint::Length(fullscreen_section_height(&key_lines, 4)),
                Constraint::Min(2),
            ])
            .split(area);
        render_fullscreen_titled_panel(
            frame,
            layout[0],
            fullscreen_overlay_title("Diagnostics"),
            body_lines,
            true,
        );
        render_fullscreen_titled_panel(
            frame,
            layout[1],
            Line::from("Startup"),
            summary_lines,
            true,
        );
        render_fullscreen_scrolled_panel(
            frame,
            layout[2],
            Line::from("Warnings"),
            warning_lines,
            warning_scroll_offset,
        );
        render_fullscreen_titled_panel(frame, layout[3], Line::from("Keys"), key_lines, true);
        render_fullscreen_titled_panel(frame, layout[4], Line::from("Checks"), check_lines, false);
        return;
    }
    let check_height = fullscreen_section_height(&check_lines, 10).max(4);
    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(fullscreen_section_height(&body_lines, 4)),
            Constraint::Length(fullscreen_section_height(&summary_lines, 4)),
            Constraint::Min(check_height),
            Constraint::Length(fullscreen_section_height(&warning_lines, 5)),
            Constraint::Length(fullscreen_section_height(&key_lines, 4)),
        ])
        .split(area);

    render_fullscreen_titled_panel(
        frame,
        layout[0],
        fullscreen_overlay_title("Diagnostics"),
        body_lines,
        true,
    );
    render_fullscreen_titled_panel(frame, layout[1], Line::from("Startup"), summary_lines, true);
    render_fullscreen_titled_panel(frame, layout[2], Line::from("Checks"), check_lines, false);
    render_fullscreen_titled_panel(
        frame,
        layout[3],
        Line::from("Warnings"),
        warning_lines,
        true,
    );
    render_fullscreen_titled_panel(frame, layout[4], Line::from("Keys"), key_lines, true);
}
fn draw_fullscreen_session_inspection(
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
            Constraint::Length(fullscreen_section_height(&body_lines, 4)),
            Constraint::Min(8),
            Constraint::Length(fullscreen_section_height(&warning_lines, 5)),
            Constraint::Length(fullscreen_section_height(&key_lines, 4)),
        ])
        .split(area);

    render_fullscreen_titled_panel(
        frame,
        layout[0],
        fullscreen_overlay_title("Recent Sessions"),
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
        draw_fullscreen_session_list_panel(frame, content_layout[0], list_state, list_view);
    render_fullscreen_titled_panel(
        frame,
        content_layout[1],
        Line::from("Selected Session"),
        detail_lines,
        false,
    );

    render_fullscreen_titled_panel(
        frame,
        layout[2],
        Line::from("Session Warnings"),
        warning_lines,
        true,
    );
    render_fullscreen_titled_panel(frame, layout[3], Line::from("Keys"), key_lines, true);
    list_state
}
fn draw_fullscreen_model_selection_inspection(
    frame: &mut Frame<'_>,
    area: Rect,
    overlay_view: ModelSelectionOverlayView,
) {
    let ModelSelectionOverlayView {
        header_lines,
        selection_title,
        selection_lines,
        summary_lines,
        key_lines,
    } = overlay_view;
    let body_lines = take_panel_body_lines(header_lines);
    let summary_height = count_wrapped_rows(&summary_lines, area.width).clamp(1, 2) as u16;
    let key_height = count_wrapped_rows(&key_lines, area.width).clamp(1, 2) as u16;
    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(fullscreen_section_height(&body_lines, 4)),
            Constraint::Min(6),
            Constraint::Length(summary_height),
            Constraint::Length(key_height),
        ])
        .split(area);

    render_fullscreen_titled_panel(
        frame,
        layout[0],
        AkraTheme::title_line("Model setup", ""),
        body_lines,
        true,
    );
    render_fullscreen_titled_panel(frame, layout[1], selection_title, selection_lines, true);
    frame.render_widget(
        Paragraph::new(summary_lines).wrap(Wrap { trim: true }),
        layout[2],
    );
    frame.render_widget(
        Paragraph::new(key_lines).wrap(Wrap { trim: true }),
        layout[3],
    );
}
fn draw_fullscreen_view_selection_inspection(
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
            Constraint::Length(fullscreen_section_height(&body_lines, 4)),
            Constraint::Min(8),
            Constraint::Length(fullscreen_section_height(&status_lines, 4)),
            Constraint::Length(fullscreen_section_height(&key_lines, 3)),
        ])
        .split(area);

    render_fullscreen_titled_panel(
        frame,
        layout[0],
        fullscreen_overlay_title("Select Conversation View"),
        body_lines,
        true,
    );
    render_fullscreen_titled_panel(frame, layout[1], Line::from("Views"), mode_lines, false);
    render_fullscreen_titled_panel(frame, layout[2], Line::from("Status"), status_lines, true);
    render_fullscreen_titled_panel(frame, layout[3], Line::from("Keys"), key_lines, true);
}
fn draw_fullscreen_language_selection_inspection(
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
            Constraint::Length(fullscreen_section_height(&body_lines, 4)),
            Constraint::Min(7),
            Constraint::Length(fullscreen_section_height(&status_lines, 4)),
            Constraint::Length(fullscreen_section_height(&key_lines, 3)),
        ])
        .split(area);

    render_fullscreen_titled_panel(
        frame,
        layout[0],
        fullscreen_overlay_title("Select Language"),
        body_lines,
        true,
    );
    render_fullscreen_titled_panel(
        frame,
        layout[1],
        Line::from("Languages"),
        language_lines,
        false,
    );
    render_fullscreen_titled_panel(frame, layout[2], Line::from("Status"), status_lines, true);
    render_fullscreen_titled_panel(frame, layout[3], Line::from("Keys"), key_lines, true);
}
fn draw_fullscreen_supersession_inspection(
    frame: &mut Frame<'_>,
    area: Rect,
    overlay_view: SupersessionOverlayView,
) {
    if !overlay_view.focused_full_viewport {
        let layout = passive_supersession_inspection_layout(&overlay_view, area);
        render_fullscreen_parallel_event_stream(frame, layout.events, overlay_view.event_stream);
        render_fullscreen_titled_panel(
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
        event_stream,
        key_lines,
    } = overlay_view;
    let body_lines = take_panel_body_lines(header_lines);
    render_fullscreen_titled_panel(
        frame,
        layout.header,
        fullscreen_overlay_title("Parallel Operations"),
        body_lines,
        true,
    );
    render_fullscreen_titled_panel(
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
        render_fullscreen_supersession_panel(
            frame,
            operations_layout[0],
            Line::from("Lifecycle"),
            timeline_lines,
        );
        render_fullscreen_supersession_panel(
            frame,
            operations_layout[2],
            Line::from("Agent Lanes"),
            if compact_columns {
                compact_lane_lines
            } else {
                lane_lines
            },
        );
        render_fullscreen_supersession_panel(
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
        let lane_height = fullscreen_section_height(&compact_lane_lines, 4)
            .min(layout.operations.height.saturating_sub(2).max(2));
        let operations_layout = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(lane_height), Constraint::Min(2)])
            .split(layout.operations);
        render_fullscreen_supersession_panel(
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
        render_fullscreen_supersession_panel(
            frame,
            operations_layout[1],
            Line::from("Selected Lane"),
            compact_detail,
        );
    }

    render_fullscreen_supersession_panel(
        frame,
        layout.accepted_queue,
        Line::from("Accepted Queue"),
        accepted_queue_lines,
    );
    render_fullscreen_parallel_event_stream(frame, layout.events, event_stream);
    render_fullscreen_titled_panel(
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
            Constraint::Length(fullscreen_section_height(&overlay_view.key_lines, 3)),
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
    let overview_height = fullscreen_section_height(&overlay_view.overview_lines, 5);
    let queue_height = fullscreen_section_height(&overlay_view.accepted_queue_lines, 4);
    let key_height = fullscreen_section_height(&overlay_view.key_lines, 3);
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

fn render_fullscreen_supersession_panel(
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
    render_fullscreen_scrolled_panel(frame, area, title, lines, scroll_offset);
}

fn render_fullscreen_parallel_event_stream(
    frame: &mut Frame<'_>,
    area: Rect,
    stream: ParallelLiveStreamModel,
) {
    let stream = stream.into_render_parts();
    let title = if stream.title_visible {
        FullscreenAppendOnlyStreamTitle::Visible(Line::from(PARALLEL_EVENT_STREAM_TITLE))
    } else {
        // A continuation of the focused event stream remains data-only; adding
        // panel chrome between chunks would make one logical stream look split.
        FullscreenAppendOnlyStreamTitle::Hidden
    };
    FullscreenAppendOnlyStream::new(title, stream.lines, stream.scroll_offset).render(frame, area);
}

fn rendered_line_rows(line: &Line<'_>, width: u16) -> usize {
    count_wrapped_rows(std::slice::from_ref(line), width).max(1)
}

pub(super) fn parallel_event_stream_area(
    overlay_view: &SupersessionOverlayView,
    area: Rect,
) -> Rect {
    let layout = if overlay_view.focused_full_viewport {
        supersession_inspection_layout(overlay_view, area)
    } else {
        passive_supersession_inspection_layout(overlay_view, area)
    };
    layout.events
}
fn draw_fullscreen_queue_inspection(
    frame: &mut Frame<'_>,
    area: Rect,
    overlay_view: QueueOverlayView,
) {
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
    // preserve vertical space in the fullscreen inspection.
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
        fullscreen_section_height(&body_lines, 3)
    };
    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(header_height),
            Constraint::Length(fullscreen_section_height(&summary_lines, 3)),
            Constraint::Min(4),
            Constraint::Length(fullscreen_section_height(&key_lines, 4)),
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
            Paragraph::new(fullscreen_overlay_title("Planning Queue")),
            layout[0],
        );
    } else {
        render_fullscreen_titled_panel(
            frame,
            layout[0],
            fullscreen_overlay_title("Planning Queue"),
            body_lines,
            true,
        );
    }
    render_fullscreen_titled_panel(frame, layout[1], Line::from("Summary"), summary_lines, true);
    render_fullscreen_scrolled_panel(
        frame,
        layout[2],
        Line::from("Queue"),
        content_lines,
        content_scroll_offset,
    );
    render_fullscreen_titled_panel(frame, layout[3], Line::from("Keys"), key_lines, true);
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
            let selected_start = count_wrapped_rows(&content_lines[..selected_index], width);
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

fn draw_fullscreen_reviews_inspection(
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
            Constraint::Length(fullscreen_section_height(&body_lines, 4)),
            Constraint::Length(fullscreen_section_height(&summary_lines, 5)),
            Constraint::Min(8),
            Constraint::Length(fullscreen_section_height(&key_lines, 3)),
        ])
        .split(area);

    render_fullscreen_titled_panel(
        frame,
        layout[0],
        fullscreen_overlay_title("Review Center"),
        body_lines,
        true,
    );
    render_fullscreen_titled_panel(frame, layout[1], Line::from("Summary"), summary_lines, true);
    let content_layout = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(34),
            Constraint::Percentage(33),
            Constraint::Percentage(33),
        ])
        .split(layout[2]);
    render_fullscreen_titled_panel(
        frame,
        content_layout[0],
        Line::from("Active Thread"),
        current_thread_lines,
        false,
    );
    render_fullscreen_titled_panel(
        frame,
        content_layout[1],
        Line::from("Inbox"),
        inbox_lines,
        false,
    );
    render_fullscreen_titled_panel(
        frame,
        content_layout[2],
        Line::from("Recent History"),
        history_lines,
        false,
    );
    render_fullscreen_titled_panel(frame, layout[3], Line::from("Keys"), key_lines, true);
}
fn draw_fullscreen_planning_init_inspection(
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
            Constraint::Length(fullscreen_section_height(&body_lines, 4)),
            Constraint::Length(fullscreen_section_height(&summary_lines, 5)),
            Constraint::Min(8),
            Constraint::Length(fullscreen_section_height(&status_lines, 5)),
            Constraint::Length(fullscreen_section_height(&key_lines, 4)),
        ])
        .split(area);

    render_fullscreen_titled_panel(
        frame,
        layout[0],
        fullscreen_overlay_title("Planning"),
        body_lines,
        true,
    );
    render_fullscreen_titled_panel(frame, layout[1], Line::from("Summary"), summary_lines, true);
    render_fullscreen_titled_panel(frame, layout[2], Line::from("Options"), option_lines, false);
    render_fullscreen_titled_panel(frame, layout[3], Line::from("Status"), status_lines, true);
    render_fullscreen_titled_panel(frame, layout[4], Line::from("Keys"), key_lines, true);
}
fn draw_fullscreen_draft_editor_inspection(
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
            Constraint::Length(fullscreen_section_height(&body_lines, 4)),
            Constraint::Length(fullscreen_section_height(&file_lines, 5)),
            Constraint::Min(editor_height),
            Constraint::Length(fullscreen_section_height(&status_lines, 6)),
            Constraint::Length(fullscreen_section_height(&key_lines, 5)),
        ])
        .split(area);

    render_fullscreen_titled_panel(
        frame,
        layout[0],
        fullscreen_overlay_title(title),
        body_lines,
        true,
    );
    render_fullscreen_titled_panel(frame, layout[1], Line::from("Files"), file_lines, true);
    render_fullscreen_scrolled_panel(
        frame,
        layout[2],
        Line::from(editor_title),
        editor_lines,
        editor_scroll,
    );
    // Cursor placement happens after rendering because the editor section title
    // consumes the first row of the split section.
    let editor_content_area = split_fullscreen_section(layout[2])[1];
    set_cursor_if_visible(frame, editor_content_area, editor_cursor_offset);
    render_fullscreen_titled_panel(frame, layout[3], Line::from("Status"), status_lines, true);
    render_fullscreen_titled_panel(frame, layout[4], Line::from("Keys"), key_lines, true);
}
fn draw_fullscreen_session_list_panel(
    frame: &mut Frame<'_>,
    area: Rect,
    mut list_state: ListState,
    list_view: OverlayListView,
) -> ListState {
    let section_layout = split_fullscreen_section(area);
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
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::buffer::Buffer;
    use ratatui::text::Line;

    use super::*;

    fn lines(label: &str, count: usize) -> Vec<Line<'static>> {
        (0..count)
            .map(|index| Line::from(format!("{label} {index}")))
            .collect()
    }

    fn section(label: &str) -> Vec<Line<'static>> {
        lines(label, 3)
    }

    fn buffer_text(buffer: &Buffer) -> String {
        if buffer.area.width == 0 {
            return String::new();
        }
        buffer
            .content
            .chunks(usize::from(buffer.area.width))
            .map(|cells| {
                cells
                    .iter()
                    .map(|cell| cell.symbol())
                    .collect::<String>()
                    .trim_end()
                    .to_string()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn render_model(
        model: FullscreenInspectionFrameModel,
        width: u16,
        height: u16,
    ) -> (String, Option<usize>) {
        let mut terminal =
            Terminal::new(TestBackend::new(width, height)).expect("inspection test terminal");
        let mut selected_index = None;
        terminal
            .draw(|frame| {
                selected_index = draw_fullscreen_shell_inspection(frame, frame.area(), model)
                    .and_then(|state| state.selected());
            })
            .expect("inspection model should render");
        (buffer_text(terminal.backend().buffer()), selected_index)
    }

    fn parallel_stream(dense: bool) -> ParallelLiveStreamModel {
        let status_lines = if dense {
            lines("parallel event", 12)
        } else {
            vec![Line::from("parallel ready")]
        };
        let mut stream =
            ParallelLiveStreamModel::pending_viewport(&Default::default(), status_lines);
        if dense {
            stream.finalize_pending_geometry(Rect::new(0, 0, 20, 2));
        }
        stream
    }

    fn supersession_view(
        focused_full_viewport: bool,
        dense_stream: bool,
    ) -> SupersessionOverlayView {
        SupersessionOverlayView {
            focused_full_viewport,
            header_lines: section("parallel header"),
            overview_lines: section("parallel overview"),
            accepted_queue_lines: section("accepted queue"),
            timeline_lines: section("timeline"),
            lane_lines: lines("lane", 10),
            compact_lane_lines: section("compact lane"),
            selected_lane_lines: lines("selected lane", 8),
            compact_selected_lane_lines: section("compact selected lane"),
            event_stream: parallel_stream(dense_stream),
            key_lines: section("parallel key"),
        }
    }

    fn queue_view(header_has_body: bool) -> QueueOverlayView {
        QueueOverlayView {
            header_lines: if header_has_body {
                section("queue header")
            } else {
                vec![Line::from("queue header")]
            },
            summary_lines: section("queue summary"),
            queue_lines: lines("queued task", 8),
            proposal_lines: section("proposed task"),
            note_lines: section("queue note"),
            selected_content_line_index: Some(7),
            key_lines: section("queue key"),
        }
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

    #[test]
    fn fullscreen_inspection_renders_every_overlay_contract() {
        let (conversation, selected) =
            render_model(FullscreenInspectionFrameModel::Conversation, 120, 40);
        assert!(conversation.trim().is_empty());
        assert_eq!(selected, None);

        let cases = vec![
            (
                FullscreenInspectionFrameModel::WorkCenter(WorkCenterOverlayView {
                    header_lines: section("work header"),
                    summary_lines: section("work summary"),
                    item_lines: lines("work item", 8),
                    detail_lines: section("work detail"),
                    key_lines: section("work key"),
                }),
                "Work Center",
            ),
            (
                FullscreenInspectionFrameModel::Activity(ActivityOverlayView {
                    header_lines: section("activity header"),
                    card_rows: Vec::new(),
                    detail_title: Line::from("Activity Detail"),
                    detail_lines: lines("activity detail", 12),
                    key_lines: section("activity key"),
                    current_page_cursor: Default::default(),
                    next_page_cursor: None,
                }),
                "Activity Detail",
            ),
            (
                FullscreenInspectionFrameModel::Approval(ApprovalFullscreenScreenModel {
                    available: true,
                    header_lines: section("approval header"),
                    detail_lines: lines("approval detail", 12),
                    key_lines: section("approval key"),
                    scroll_offset: 1,
                    visible_start: 2,
                    visible_end: 8,
                    rendered_detail_rows: 12,
                }),
                "Approval Required",
            ),
            (
                FullscreenInspectionFrameModel::ParallelPeek {
                    view: ParallelPeekOverlayView {
                        header_lines: section("peek header"),
                        agent_lines: lines("agent", 10),
                        conversation_lines: lines("conversation", 12),
                        status_lines: section("peek status"),
                        key_lines: section("peek key"),
                    },
                    step: ParallelPeekOverlayStep::AgentList,
                    scroll_from_bottom: 0,
                },
                "Active Agents",
            ),
            (
                FullscreenInspectionFrameModel::ParallelPeek {
                    view: ParallelPeekOverlayView {
                        header_lines: section("peek header"),
                        agent_lines: lines("agent", 10),
                        conversation_lines: lines("conversation", 24),
                        status_lines: section("peek status"),
                        key_lines: section("peek key"),
                    },
                    step: ParallelPeekOverlayStep::ConversationPreview,
                    scroll_from_bottom: 2,
                },
                "Conversation Preview",
            ),
            (
                FullscreenInspectionFrameModel::Help {
                    language: TuiLanguage::English,
                    view: HelpOverlayView {
                        header_lines: section("help header"),
                        command_lines: lines("help command", 18),
                        key_lines: section("help key"),
                    },
                    scroll_offset: 2,
                },
                "Commands",
            ),
            (
                FullscreenInspectionFrameModel::Directions(DirectionsMaintenanceOverlayView {
                    header_lines: section("directions header"),
                    summary_lines: section("directions summary"),
                    option_lines: lines("directions option", 10),
                    status_lines: section("directions status"),
                    key_lines: section("directions key"),
                }),
                "Directions",
            ),
            (
                FullscreenInspectionFrameModel::Startup {
                    view: StartupOverlayView {
                        header_lines: section("startup header"),
                        summary_lines: section("startup summary"),
                        check_lines: lines("startup check", 12),
                        warning_lines: Vec::new(),
                        key_lines: section("startup key"),
                    },
                    warning_scroll_offset: 0,
                },
                "Diagnostics",
            ),
            (
                FullscreenInspectionFrameModel::Sessions {
                    view: SessionOverlayView {
                        header_lines: section("session header"),
                        list_view: OverlayListView {
                            message_lines: None,
                            items: Vec::new(),
                            selected_index: None,
                        },
                        detail_lines: section("session detail"),
                        warning_lines: section("session warning"),
                        key_lines: section("session key"),
                    },
                    list_state: ListState::default(),
                },
                "Recent Sessions",
            ),
            (
                FullscreenInspectionFrameModel::ModelSelection(ModelSelectionOverlayView {
                    header_lines: section("model header"),
                    selection_title: Line::from("Choose model"),
                    selection_lines: lines("model", 12),
                    summary_lines: section("model summary"),
                    key_lines: section("model key"),
                }),
                "Model setup",
            ),
            (
                FullscreenInspectionFrameModel::ViewSelection(ViewSelectionOverlayView {
                    header_lines: section("view header"),
                    mode_lines: lines("view mode", 8),
                    status_lines: section("view status"),
                    key_lines: section("view key"),
                }),
                "Select Conversation View",
            ),
            (
                FullscreenInspectionFrameModel::LanguageSelection(LanguageSelectionOverlayView {
                    header_lines: section("language header"),
                    language_lines: lines("language", 8),
                    status_lines: section("language status"),
                    key_lines: section("language key"),
                }),
                "Select Language",
            ),
            (
                FullscreenInspectionFrameModel::Supersession(supersession_view(true, false)),
                "Parallel Operations",
            ),
            (
                FullscreenInspectionFrameModel::Queue(queue_view(true)),
                "Planning Queue",
            ),
            (
                FullscreenInspectionFrameModel::Reviews(ReviewsOverlayView {
                    header_lines: section("reviews header"),
                    summary_lines: section("reviews summary"),
                    current_thread_reviews: Vec::new(),
                    inbox_reviews: Vec::new(),
                    history_reviews: Vec::new(),
                    key_lines: section("reviews key"),
                }),
                "Review Center",
            ),
            (
                FullscreenInspectionFrameModel::PlanningInit(PlanningInitOverlayView {
                    header_lines: section("planning header"),
                    summary_lines: section("planning summary"),
                    option_lines: lines("planning option", 10),
                    status_lines: section("planning status"),
                    key_lines: section("planning key"),
                }),
                "Planning",
            ),
            (
                FullscreenInspectionFrameModel::DraftEditor {
                    title: "Draft Editor",
                    view: Some(PlanningDraftEditorOverlayView {
                        header_lines: section("draft header"),
                        file_lines: section("draft file"),
                        editor_title: "Active Document".to_string(),
                        editor_lines: lines("draft content", 14),
                        editor_scroll: 2,
                        editor_cursor_offset: Some((1, 1)),
                        status_lines: section("draft status"),
                        key_lines: section("draft key"),
                    }),
                },
                "Draft Editor",
            ),
        ];

        for (model, expected) in cases {
            let (screen, _) = render_model(model, 120, 40);
            assert!(
                screen.contains(expected),
                "expected `{expected}` in rendered inspection:\n{screen}"
            );
        }
    }

    #[test]
    fn fullscreen_inspection_renders_compact_and_degraded_branches() {
        let compact_models = [
            FullscreenInspectionFrameModel::WorkCenter(WorkCenterOverlayView {
                header_lines: section("compact work header"),
                summary_lines: section("compact work summary"),
                item_lines: section("compact work item"),
                detail_lines: section("compact work detail"),
                key_lines: section("compact work key"),
            }),
            FullscreenInspectionFrameModel::Startup {
                view: StartupOverlayView {
                    header_lines: section("warning header"),
                    summary_lines: section("warning summary"),
                    check_lines: lines("warning check", 8),
                    warning_lines: lines("attention warning", 8),
                    key_lines: section("warning key"),
                },
                warning_scroll_offset: 2,
            },
            FullscreenInspectionFrameModel::Approval(ApprovalFullscreenScreenModel {
                available: false,
                header_lines: Vec::new(),
                detail_lines: Vec::new(),
                key_lines: Vec::new(),
                scroll_offset: 0,
                visible_start: 0,
                visible_end: 0,
                rendered_detail_rows: 0,
            }),
            FullscreenInspectionFrameModel::Sessions {
                view: SessionOverlayView {
                    header_lines: section("degraded session header"),
                    list_view: OverlayListView {
                        message_lines: Some(section("session unavailable")),
                        items: Vec::new(),
                        selected_index: None,
                    },
                    detail_lines: section("degraded session detail"),
                    warning_lines: section("degraded session warning"),
                    key_lines: section("degraded session key"),
                },
                list_state: ListState::default(),
            },
            FullscreenInspectionFrameModel::Queue(queue_view(false)),
            FullscreenInspectionFrameModel::DraftEditor {
                title: "Unavailable Draft",
                view: None,
            },
        ];

        for model in compact_models {
            render_model(model, 80, 16);
        }

        let (passive, _) = render_model(
            FullscreenInspectionFrameModel::Supersession(supersession_view(false, false)),
            100,
            24,
        );
        assert!(passive.contains(PARALLEL_EVENT_STREAM_TITLE));

        let (wide_compact, _) = render_model(
            FullscreenInspectionFrameModel::Supersession(supersession_view(true, true)),
            140,
            18,
        );
        assert!(wide_compact.contains("Parallel Operations"));
        assert!(!wide_compact.contains(PARALLEL_EVENT_STREAM_TITLE));

        let (narrow, _) = render_model(
            FullscreenInspectionFrameModel::Supersession(supersession_view(true, false)),
            100,
            30,
        );
        assert!(narrow.contains("Selected Lane"));

        assert_eq!(
            inline_preview_scroll_offset(Rect::new(0, 0, 80, 4), 12, 2),
            7
        );
        assert_eq!(
            wrapped_approval_panel_height(&lines("wrapped", 2), 80, 6),
            6
        );
    }
}
