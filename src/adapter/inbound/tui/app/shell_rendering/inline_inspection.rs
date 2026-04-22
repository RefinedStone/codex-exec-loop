use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::Line;
use ratatui::widgets::{List, ListItem, Paragraph, Wrap};

use super::super::shell_presentation::{
    AutomationOverlayView, DirectionsMaintenanceOverlayView, OverlayListView,
    PlanningDraftEditorOverlayView, PlanningInitOverlayView, QueueOverlayView, SessionOverlayView,
    StartupOverlayView, SupersessionOverlayView, build_automation_overlay_view,
    build_directions_maintenance_overlay_view, build_planning_draft_editor_overlay_view,
    build_planning_init_overlay_view, build_queue_overlay_view, build_session_overlay_view,
    build_startup_overlay_view, build_supersession_overlay_view,
};
use super::super::{
    DirectionsMaintenanceOverlayStep, NativeTuiApp, PlanningInitOverlayStep, ShellOverlay,
};
use super::inline_layout::{
    clamp_scroll_offset, inline_section_height, render_inline_scrolled_section,
    render_inline_section, set_cursor_if_visible, split_inline_section, take_panel_body_lines,
};

pub(super) fn draw_inline_shell_inspection(
    frame: &mut Frame<'_>,
    app: &mut NativeTuiApp,
    inspection_area: Rect,
) {
    match app.shell_overlay {
        ShellOverlay::Hidden => {}
        ShellOverlay::Startup => draw_inline_startup_inspection(frame, inspection_area, app),
        ShellOverlay::Sessions => draw_inline_session_inspection(frame, inspection_area, app),
        ShellOverlay::Supersession => {
            draw_inline_supersession_inspection(frame, inspection_area, app)
        }
        ShellOverlay::Queue => draw_inline_queue_inspection(frame, inspection_area, app),
        ShellOverlay::DirectionsMaintenance => {
            draw_inline_directions_maintenance_inspection(frame, inspection_area, app)
        }
        ShellOverlay::Automation => draw_inline_automation_inspection(frame, inspection_area, app),
        ShellOverlay::PlanningInit => {
            draw_inline_planning_init_inspection(frame, inspection_area, app)
        }
    }
}

fn draw_inline_directions_maintenance_inspection(
    frame: &mut Frame<'_>,
    area: Rect,
    app: &NativeTuiApp,
) {
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
        Line::from("Directions / inline inspection"),
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
        Line::from("Diagnostics / inline inspection"),
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
        Line::from("Recent Sessions / inline inspection"),
        body_lines,
        true,
    );

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

fn draw_inline_supersession_inspection(frame: &mut Frame<'_>, area: Rect, app: &NativeTuiApp) {
    let overlay_view = build_supersession_overlay_view(app);
    let SupersessionOverlayView {
        header_lines,
        summary_lines,
        capability_lines,
        pool_lines,
        roster_lines,
        detail_lines,
        distributor_lines,
        key_lines,
    } = overlay_view;
    let body_lines = take_panel_body_lines(header_lines);
    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(inline_section_height(&body_lines, 4)),
            Constraint::Length(inline_section_height(&summary_lines, 8)),
            Constraint::Min(12),
            Constraint::Length(inline_section_height(&key_lines, 4)),
        ])
        .split(area);

    render_inline_section(
        frame,
        layout[0],
        Line::from("Supersession / inline inspection"),
        body_lines,
        true,
    );
    render_inline_section(frame, layout[1], Line::from("Summary"), summary_lines, true);

    let content_layout = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(layout[2]);
    let left_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(inline_section_height(&capability_lines, 8)),
            Constraint::Length(inline_section_height(&pool_lines, 8)),
            Constraint::Min(6),
        ])
        .split(content_layout[0]);
    let right_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(inline_section_height(&detail_lines, 7)),
            Constraint::Min(7),
        ])
        .split(content_layout[1]);

    render_inline_section(
        frame,
        left_layout[0],
        Line::from("Capabilities"),
        capability_lines,
        false,
    );
    render_inline_section(
        frame,
        left_layout[1],
        Line::from("Pool Board"),
        pool_lines,
        false,
    );
    render_inline_section(
        frame,
        left_layout[2],
        Line::from("Agent Roster"),
        roster_lines,
        false,
    );
    render_inline_section(
        frame,
        right_layout[0],
        Line::from("Selected Detail"),
        detail_lines,
        false,
    );
    render_inline_section(
        frame,
        right_layout[1],
        Line::from("Distributor / Queue"),
        distributor_lines,
        false,
    );
    render_inline_section(frame, layout[3], Line::from("Keys"), key_lines, true);
}

fn draw_inline_automation_inspection(frame: &mut Frame<'_>, area: Rect, app: &mut NativeTuiApp) {
    let overlay_view = build_automation_overlay_view(app);
    let AutomationOverlayView {
        header_lines,
        list_view,
        preview_lines,
        status_lines,
        key_lines,
    } = overlay_view;
    let body_lines = take_panel_body_lines(header_lines);
    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(inline_section_height(&body_lines, 4)),
            Constraint::Min(10),
            Constraint::Length(inline_section_height(&status_lines, 11)),
            Constraint::Length(inline_section_height(&key_lines, 6)),
        ])
        .split(area);

    render_inline_section(
        frame,
        layout[0],
        Line::from("Automation Controls / inline inspection"),
        body_lines,
        true,
    );

    let content_layout = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(34), Constraint::Percentage(66)])
        .split(layout[1]);
    let preview_content_area = split_inline_section(content_layout[1])[1];
    let preview_scroll = clamp_scroll_offset(
        app.followup_overlay_ui_state.preview_scroll,
        &preview_lines,
        preview_content_area.width,
        preview_content_area.height,
    );
    app.followup_overlay_ui_state.preview_scroll = preview_scroll;

    draw_inline_automation_list_panel(frame, content_layout[0], app, list_view);
    render_inline_scrolled_section(
        frame,
        content_layout[1],
        Line::from("Preview"),
        preview_lines,
        preview_scroll,
    );
    render_inline_section(
        frame,
        layout[2],
        Line::from("Auto Follow-Up State"),
        status_lines,
        false,
    );
    render_inline_section(frame, layout[3], Line::from("Keys"), key_lines, true);
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
        Line::from("Planning Queue / inline inspection"),
        body_lines,
        true,
    );
    render_inline_section(frame, layout[1], Line::from("Summary"), summary_lines, true);
    render_inline_section(frame, layout[2], Line::from("Queue"), content_lines, false);
    render_inline_section(frame, layout[3], Line::from("Keys"), key_lines, true);
}

fn draw_inline_planning_init_inspection(frame: &mut Frame<'_>, area: Rect, app: &NativeTuiApp) {
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
        Line::from("Planning / inline inspection"),
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
    draw_inline_draft_editor_inspection(frame, area, app, "Planning Draft / inline inspection");
}

fn draw_inline_draft_editor_inspection(
    frame: &mut Frame<'_>,
    area: Rect,
    app: &NativeTuiApp,
    title: &'static str,
) {
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

    render_inline_section(frame, layout[0], Line::from(title), body_lines, true);
    render_inline_section(frame, layout[1], Line::from("Files"), file_lines, true);
    render_inline_scrolled_section(
        frame,
        layout[2],
        Line::from(editor_title),
        editor_lines,
        editor_scroll,
    );
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
    draw_inline_draft_editor_inspection(frame, area, app, "Directions Draft / inline inspection");
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
    .highlight_style(
        Style::default()
            .fg(Color::Black)
            .bg(Color::Cyan)
            .add_modifier(Modifier::BOLD),
    )
    .highlight_symbol(">> ");

    app.session_overlay_ui_state
        .sync_selected_session(list_view.selected_index);
    frame.render_stateful_widget(
        list,
        section_layout[1],
        &mut app.session_overlay_ui_state.list_state,
    );
}

fn draw_inline_automation_list_panel(
    frame: &mut Frame<'_>,
    area: Rect,
    app: &mut NativeTuiApp,
    list_view: OverlayListView,
) {
    let section_layout = split_inline_section(area);
    frame.render_widget(
        Paragraph::new(vec![Line::from("Automation")]),
        section_layout[0],
    );

    if let Some(message_lines) = list_view.message_lines {
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
    .highlight_style(
        Style::default()
            .fg(Color::Black)
            .bg(Color::Cyan)
            .add_modifier(Modifier::BOLD),
    )
    .highlight_symbol(">> ");

    app.followup_overlay_ui_state
        .list_state
        .select(list_view.selected_index);
    frame.render_stateful_widget(
        list,
        section_layout[1],
        &mut app.followup_overlay_ui_state.list_state,
    );
}
