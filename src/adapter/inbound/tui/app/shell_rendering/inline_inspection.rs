use super::super::shell_presentation::{
    DirectionsMaintenanceOverlayView, HelpOverlayView, ModelSelectionOverlayView, OverlayListView,
    ParallelPeekOverlayView, PlanningDraftEditorOverlayView, PlanningInitOverlayView,
    QueueOverlayView, SessionOverlayView, StartupOverlayView, SupersessionOverlayView,
    ViewSelectionOverlayView, build_directions_maintenance_overlay_view, build_help_overlay_view,
    build_model_selection_overlay_view, build_parallel_peek_overlay_view,
    build_planning_draft_editor_overlay_view, build_planning_init_overlay_view,
    build_queue_overlay_view, build_session_overlay_view, build_startup_overlay_view,
    build_supersession_overlay_view, build_view_selection_overlay_view,
};
use super::super::{
    AkraTheme, DirectionsMaintenanceOverlayStep, NativeTuiApp, ParallelPeekOverlayStep,
    PlanningInitOverlayStep, ShellOverlay,
};
use super::inline_layout::{
    inline_section_height, render_inline_scrolled_section, render_inline_section,
    set_cursor_if_visible, split_inline_section, take_panel_body_lines,
};
use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::text::Line;
use ratatui::widgets::{List, ListItem, Paragraph, Wrap};

// Inline inspection renders shell overlays inside the app-server main buffer.
// It reuses presentation view builders, then maps those view models into
// frameless sections that can replace the transcript area without popup chrome.
fn inline_overlay_title(name: &'static str) -> Line<'static> {
    AkraTheme::title_line(name, " / inline inspection")
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
        ShellOverlay::Supersession => {
            draw_inline_supersession_inspection(frame, inspection_area, app)
        }
        ShellOverlay::ParallelPeek => {
            draw_inline_parallel_peek_inspection(frame, inspection_area, app)
        }
        ShellOverlay::Help => draw_inline_help_inspection(frame, inspection_area),
        ShellOverlay::Queue => draw_inline_queue_inspection(frame, inspection_area, app),
        ShellOverlay::DirectionsMaintenance => {
            draw_inline_directions_maintenance_inspection(frame, inspection_area, app)
        }
        ShellOverlay::PlanningInit => {
            draw_inline_planning_init_inspection(frame, inspection_area, app)
        }
    }
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

    render_inline_section(
        frame,
        layout[0],
        inline_overlay_title("Parallel Peek"),
        body_lines,
        true,
    );
    match step {
        ParallelPeekOverlayStep::AgentList => render_inline_scrolled_section(
            frame,
            layout[1],
            Line::from("Active Agents"),
            agent_lines,
            0,
        ),
        ParallelPeekOverlayStep::ConversationPreview => render_inline_scrolled_section(
            frame,
            layout[1],
            Line::from("Conversation Preview"),
            conversation_lines,
            0,
        ),
    }
    render_inline_section(frame, layout[2], Line::from("Status"), status_lines, true);
    render_inline_section(frame, layout[3], Line::from("Keys"), key_lines, true);
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

    render_inline_section(
        frame,
        layout[0],
        inline_overlay_title("Shell Commands"),
        body_lines,
        true,
    );
    render_inline_section(
        frame,
        layout[1],
        Line::from("Commands"),
        command_lines,
        false,
    );
    render_inline_section(frame, layout[2], Line::from("Keys"), key_lines, true);
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

    render_inline_section(
        frame,
        layout[0],
        inline_overlay_title("Directions"),
        body_lines,
        true,
    );
    render_inline_section(frame, layout[1], Line::from("Summary"), summary_lines, true);
    render_inline_section(frame, layout[2], Line::from("Options"), option_lines, false);
    render_inline_section(frame, layout[3], Line::from("Status"), status_lines, true);
    render_inline_section(frame, layout[4], Line::from("Keys"), key_lines, true);
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

    render_inline_section(
        frame,
        layout[0],
        inline_overlay_title("Diagnostics"),
        body_lines,
        true,
    );
    render_inline_section(frame, layout[1], Line::from("Startup"), summary_lines, true);
    render_inline_section(frame, layout[2], Line::from("Checks"), check_lines, false);
    render_inline_section(
        frame,
        layout[3],
        Line::from("Warnings"),
        warning_lines,
        true,
    );
    render_inline_section(frame, layout[4], Line::from("Keys"), key_lines, true);
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

    render_inline_section(
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
    render_inline_section(
        frame,
        content_layout[1],
        Line::from("Selected Session"),
        detail_lines,
        false,
    );

    render_inline_section(
        frame,
        layout[2],
        Line::from("Session Warnings"),
        warning_lines,
        true,
    );
    render_inline_section(frame, layout[3], Line::from("Keys"), key_lines, true);
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

    render_inline_section(
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
    render_inline_section(
        frame,
        picker_layout[0],
        Line::from("Models"),
        model_lines,
        false,
    );
    render_inline_section(
        frame,
        picker_layout[1],
        Line::from("Think Level"),
        effort_lines,
        false,
    );
    render_inline_section(frame, layout[2], Line::from("Status"), status_lines, true);
    render_inline_section(frame, layout[3], Line::from("Keys"), key_lines, true);
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

    render_inline_section(
        frame,
        layout[0],
        inline_overlay_title("Select Conversation View"),
        body_lines,
        true,
    );
    render_inline_section(frame, layout[1], Line::from("Views"), mode_lines, false);
    render_inline_section(frame, layout[2], Line::from("Status"), status_lines, true);
    render_inline_section(frame, layout[3], Line::from("Keys"), key_lines, true);
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

    render_inline_section(
        frame,
        layout[0],
        inline_overlay_title("Parallel Mode"),
        body_lines,
        true,
    );
    render_inline_section(
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

    render_inline_section(
        frame,
        status_layout[0],
        Line::from("Distributor"),
        capability_lines,
        false,
    );
    render_inline_section(
        frame,
        status_layout[1],
        Line::from("Pool"),
        pool_lines,
        false,
    );
    render_inline_section(
        frame,
        status_layout[2],
        Line::from("Orchestrator"),
        roster_lines,
        false,
    );
    let stream_visible_rows = layout[3].height.saturating_sub(1) as usize;
    let stream_scroll_offset = detail_lines
        .len()
        .saturating_sub(stream_visible_rows)
        .min(u16::MAX as usize) as u16;
    render_inline_scrolled_section(
        frame,
        layout[3],
        Line::from("Parallel Event Stream"),
        detail_lines,
        stream_scroll_offset,
    );
    render_inline_section(
        frame,
        layout[4],
        Line::from("Command Hints"),
        key_lines,
        true,
    );
}
fn draw_inline_queue_inspection(frame: &mut Frame<'_>, area: Rect, app: &NativeTuiApp) {
    let overlay_view = build_queue_overlay_view(app);
    let QueueOverlayView {
        header_lines,
        summary_lines,
        queue_lines,
        proposal_lines,
        note_lines,
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

    render_inline_section(
        frame,
        layout[0],
        inline_overlay_title("Planning Queue"),
        body_lines,
        true,
    );
    render_inline_section(frame, layout[1], Line::from("Summary"), summary_lines, true);
    render_inline_section(frame, layout[2], Line::from("Queue"), content_lines, false);
    render_inline_section(frame, layout[3], Line::from("Keys"), key_lines, true);
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

    render_inline_section(
        frame,
        layout[0],
        inline_overlay_title("Planning"),
        body_lines,
        true,
    );
    render_inline_section(frame, layout[1], Line::from("Summary"), summary_lines, true);
    render_inline_section(frame, layout[2], Line::from("Options"), option_lines, false);
    render_inline_section(frame, layout[3], Line::from("Status"), status_lines, true);
    render_inline_section(frame, layout[4], Line::from("Keys"), key_lines, true);
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

    render_inline_section(
        frame,
        layout[0],
        inline_overlay_title(title),
        body_lines,
        true,
    );
    render_inline_section(frame, layout[1], Line::from("Files"), file_lines, true);
    render_inline_scrolled_section(
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
    render_inline_section(frame, layout[3], Line::from("Status"), status_lines, true);
    render_inline_section(frame, layout[4], Line::from("Keys"), key_lines, true);
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
