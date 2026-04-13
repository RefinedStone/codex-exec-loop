use std::rc::Rc;

use ratatui::layout::Position;

#[cfg(test)]
use super::shell_presentation::{
    ConversationShellFrameView, build_conversation_shell_frame_view,
    build_input_prompt_cursor_offset,
};
use super::shell_presentation::{
    FollowupTemplateOverlayView, OverlayListView, PlanningDraftEditorOverlayView,
    PlanningInitOverlayView, QueueOverlayView, SessionOverlayView, StartupOverlayView,
    build_followup_template_overlay_view, build_inline_prompt_cursor_offset,
    build_inline_tail_lines, build_planning_draft_editor_overlay_view,
    build_planning_init_overlay_view, build_queue_overlay_view, build_session_overlay_view,
    build_startup_overlay_view, startup_screen_is_active,
};
use super::*;

const MAX_INLINE_INSPECTION_TAIL_HEIGHT: u16 = 6;

pub(super) fn prepare_render_state(app: &mut NativeTuiApp, mode: ShellFrontendMode, area: Rect) {
    let _ = mode;
    if app.shell_overlay != ShellOverlay::PlanningInit
        || app.planning_init_overlay_ui_state.step() != PlanningInitOverlayStep::ManualEditor
    {
        return;
    }

    let tail_lines = build_inline_tail_lines(app);
    let inspection_area = build_inline_terminal_flow_layout(app, area, &tail_lines)[0];
    let editor_content_height = inspection_area
        .height
        .saturating_sub(14)
        .max(6)
        .saturating_sub(1)
        .max(1);
    app.planning_draft_editor_ui_state
        .sync_editor_scroll(editor_content_height);
}

pub(super) fn draw(frame: &mut Frame<'_>, app: &mut NativeTuiApp, mode: ShellFrontendMode) {
    let _ = mode;
    let frame_area = frame.area();
    let tail_lines = build_inline_tail_lines(app);
    let layout = build_inline_terminal_flow_layout(app, frame_area, &tail_lines);

    draw_inline_conversation_shell(frame, app, tail_lines, &layout);

    if app.shell_overlay != ShellOverlay::Hidden {
        draw_inline_shell_inspection(frame, app, layout[0]);
    }

    if app.is_exit_confirmation_visible() {
        draw_exit_confirmation(frame);
    }
}

#[cfg(test)]
#[allow(dead_code)]
fn draw_session_list_panel(
    frame: &mut Frame<'_>,
    area: Rect,
    app: &mut NativeTuiApp,
    list_view: OverlayListView,
) {
    if let Some(message_lines) = list_view.message_lines {
        let widget = Paragraph::new(message_lines)
            .block(Block::default().borders(Borders::ALL).title("Threads"))
            .wrap(Wrap { trim: true });
        frame.render_widget(widget, area);
        return;
    }

    let list = List::new(
        list_view
            .items
            .into_iter()
            .map(|item| ListItem::new(item.lines)),
    )
    .block(Block::default().borders(Borders::ALL).title("Threads"))
    .highlight_style(
        Style::default()
            .fg(Color::Black)
            .bg(Color::Cyan)
            .add_modifier(Modifier::BOLD),
    )
    .highlight_symbol(">> ");

    app.session_overlay_ui_state
        .sync_selected_session(list_view.selected_index);
    frame.render_stateful_widget(list, area, &mut app.session_overlay_ui_state.list_state);
}

#[cfg(test)]
#[allow(dead_code)]
fn draw_session_detail_panel(frame: &mut Frame<'_>, area: Rect, lines: Vec<Line<'static>>) {
    let detail = Paragraph::new(lines)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title("Selected Session"),
        )
        .wrap(Wrap { trim: false });
    frame.render_widget(detail, area);
}

fn draw_inline_conversation_shell(
    frame: &mut Frame<'_>,
    app: &mut NativeTuiApp,
    tail_lines: Vec<Line<'static>>,
    layout: &Rc<[Rect]>,
) {
    let frame_area = frame.area();
    frame.render_widget(Clear, frame_area);
    if app.shell_overlay == ShellOverlay::Hidden && !app.is_exit_confirmation_visible() {
        let tail_area = if startup_screen_is_active(app) {
            frame_area
        } else {
            inline_body_top_render_area(frame_area, &tail_lines)
        };
        render_inline_body(frame, tail_area, tail_lines, false);
        set_cursor_if_visible(
            frame,
            tail_area,
            build_inline_prompt_cursor_offset(app, tail_area.width),
        );
        return;
    }

    render_inline_body(
        frame,
        inline_body_render_area(layout[1], &tail_lines),
        tail_lines,
        false,
    );
}

fn build_inline_terminal_flow_layout(
    app: &NativeTuiApp,
    area: Rect,
    tail_lines: &[Line<'_>],
) -> Rc<[Rect]> {
    let tail_max_height =
        if app.shell_overlay == ShellOverlay::Hidden && !app.is_exit_confirmation_visible() {
            MAX_INLINE_TAIL_HEIGHT
        } else {
            MAX_INLINE_INSPECTION_TAIL_HEIGHT
        };
    let tail_height = inline_body_height(&tail_lines, area.width, tail_max_height);

    Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(MIN_TRANSCRIPT_PANEL_HEIGHT.saturating_sub(2).max(6)),
            Constraint::Length(tail_height),
        ])
        .split(area)
}

#[cfg(test)]
#[allow(dead_code)]
fn draw_framed_conversation_shell(
    frame: &mut Frame<'_>,
    app: &mut NativeTuiApp,
    mode: ShellFrontendMode,
) {
    let area = frame.area();
    frame.render_widget(Clear, area);
    let shell_frame_view = build_conversation_shell_frame_view(app, mode, area);
    let ConversationShellFrameView {
        shell_title,
        header_lines,
        header_area,
        transcript_view,
        transcript_area,
        status_title,
        footer_lines,
        footer_area,
        input_title,
        input_lines,
        input_area,
    } = shell_frame_view;

    let header = Paragraph::new(header_lines)
        .block(Block::default().borders(Borders::ALL).title(shell_title))
        .wrap(Wrap { trim: true });
    frame.render_widget(header, header_area);

    let conversation = Paragraph::new(transcript_view.lines)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(transcript_view.title),
        )
        .scroll((transcript_view.scroll_offset, 0))
        .wrap(Wrap { trim: false });
    frame.render_widget(conversation, transcript_area);

    let footer = Paragraph::new(footer_lines)
        .block(Block::default().borders(Borders::ALL).title(status_title))
        .wrap(Wrap { trim: true });
    frame.render_widget(footer, footer_area);

    let input = Paragraph::new(input_lines)
        .block(Block::default().borders(Borders::ALL).title(input_title))
        .wrap(Wrap { trim: false });
    frame.render_widget(input, input_area);

    if app.shell_overlay == ShellOverlay::Hidden && !app.is_exit_confirmation_visible() {
        let input_content_area = Block::default().borders(Borders::ALL).inner(input_area);
        set_cursor_if_visible(
            frame,
            input_content_area,
            build_input_prompt_cursor_offset(app, input_content_area.width),
        );
    }
}

fn inline_section_height(lines: &[Line<'_>], max_height: u16) -> u16 {
    lines
        .len()
        .saturating_add(1)
        .max(2)
        .min(max_height as usize) as u16
}

fn inline_body_height(lines: &[Line<'_>], width: u16, max_height: u16) -> u16 {
    count_rendered_inline_rows(lines, width)
        .max(1)
        .min(max_height as usize) as u16
}

fn inline_body_render_area(area: Rect, lines: &[Line<'_>]) -> Rect {
    let body_height = inline_body_height(lines, area.width, area.height);
    let y = area.y + area.height.saturating_sub(body_height);
    Rect::new(area.x, y, area.width, body_height)
}

fn inline_body_top_render_area(area: Rect, lines: &[Line<'_>]) -> Rect {
    let body_height = inline_body_height(lines, area.width, area.height);
    Rect::new(area.x, area.y, area.width, body_height)
}

fn count_rendered_inline_rows(lines: &[Line<'_>], width: u16) -> usize {
    if width == 0 {
        return 0;
    }

    lines
        .iter()
        .map(|line| {
            let line_width = line.width();
            if line_width == 0 {
                1
            } else {
                line_width.div_ceil(width as usize)
            }
        })
        .sum()
}

fn split_inline_section(area: Rect) -> Rc<[Rect]> {
    Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Min(1)])
        .split(area)
}

fn render_inline_section(
    frame: &mut Frame<'_>,
    area: Rect,
    title: Line<'static>,
    lines: Vec<Line<'static>>,
    trim: bool,
) {
    let section_layout = split_inline_section(area);
    frame.render_widget(Paragraph::new(vec![title]), section_layout[0]);
    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim }), section_layout[1]);
}

fn render_inline_body(frame: &mut Frame<'_>, area: Rect, lines: Vec<Line<'static>>, trim: bool) {
    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim }), area);
}

fn set_cursor_if_visible(frame: &mut Frame<'_>, area: Rect, offset: Option<(u16, u16)>) {
    let Some((cursor_x, cursor_y)) = offset else {
        return;
    };
    if area.width == 0 || area.height == 0 {
        return;
    }

    let clamped_x = cursor_x.min(area.width.saturating_sub(1));
    let clamped_y = cursor_y.min(area.height.saturating_sub(1));
    frame.set_cursor_position(Position::new(area.x + clamped_x, area.y + clamped_y));
}

fn render_inline_scrolled_section(
    frame: &mut Frame<'_>,
    area: Rect,
    title: Line<'static>,
    lines: Vec<Line<'static>>,
    scroll_offset: u16,
) {
    let section_layout = split_inline_section(area);
    frame.render_widget(Paragraph::new(vec![title]), section_layout[0]);
    frame.render_widget(
        Paragraph::new(lines)
            .scroll((scroll_offset, 0))
            .wrap(Wrap { trim: false }),
        section_layout[1],
    );
}

fn take_panel_body_lines(mut header_lines: Vec<Line<'static>>) -> Vec<Line<'static>> {
    if !header_lines.is_empty() {
        header_lines.remove(0);
    }
    header_lines
}

fn draw_inline_shell_inspection(
    frame: &mut Frame<'_>,
    app: &mut NativeTuiApp,
    inspection_area: Rect,
) {
    match app.shell_overlay {
        ShellOverlay::Hidden => {}
        ShellOverlay::Startup => draw_inline_startup_inspection(frame, inspection_area, app),
        ShellOverlay::Sessions => draw_inline_session_inspection(frame, inspection_area, app),
        ShellOverlay::Queue => draw_inline_queue_inspection(frame, inspection_area, app),
        ShellOverlay::FollowupTemplates => {
            draw_inline_followup_template_inspection(frame, inspection_area, app)
        }
        ShellOverlay::PlanningInit => {
            draw_inline_planning_init_inspection(frame, inspection_area, app)
        }
    }
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

fn draw_inline_followup_template_inspection(
    frame: &mut Frame<'_>,
    area: Rect,
    app: &mut NativeTuiApp,
) {
    let overlay_view = build_followup_template_overlay_view(app);
    let FollowupTemplateOverlayView {
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
        Line::from("Follow-Up Templates / inline inspection"),
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

    draw_inline_followup_template_list_panel(frame, content_layout[0], app, list_view);
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
        Line::from("Planning Draft / inline inspection"),
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
    let editor_content_area = split_inline_section(layout[2])[1];
    set_cursor_if_visible(frame, editor_content_area, editor_cursor_offset);
    render_inline_section(frame, layout[3], Line::from("Status"), status_lines, true);
    render_inline_section(frame, layout[4], Line::from("Keys"), key_lines, true);
}

#[cfg(test)]
#[allow(dead_code)]
fn draw_startup_overlay(frame: &mut Frame<'_>, app: &NativeTuiApp) {
    let overlay_view = build_startup_overlay_view(app);
    let StartupOverlayView {
        header_lines,
        summary_lines,
        check_lines,
        warning_lines,
        key_lines,
    } = overlay_view;
    let popup_area = centered_rect(78, 72, frame.area());
    frame.render_widget(Clear, popup_area);

    let layout = Layout::default()
        .direction(Direction::Vertical)
        .margin(1)
        .constraints([
            Constraint::Length(3),
            Constraint::Length(7),
            Constraint::Min(10),
            Constraint::Length(6),
            Constraint::Length(3),
        ])
        .split(popup_area);

    let header = Paragraph::new(header_lines)
        .block(Block::default().borders(Borders::ALL).title("Diagnostics"));
    frame.render_widget(header, layout[0]);

    frame.render_widget(
        Paragraph::new(summary_lines)
            .block(Block::default().borders(Borders::ALL).title("Startup"))
            .wrap(Wrap { trim: true }),
        layout[1],
    );

    frame.render_widget(
        List::new(check_lines).block(Block::default().borders(Borders::ALL).title("Checks")),
        layout[2],
    );

    frame.render_widget(
        Paragraph::new(warning_lines)
            .block(Block::default().borders(Borders::ALL).title("Warnings"))
            .wrap(Wrap { trim: true }),
        layout[3],
    );

    frame.render_widget(
        Paragraph::new(key_lines).block(Block::default().borders(Borders::ALL).title("Keys")),
        layout[4],
    );
}

#[cfg(test)]
#[allow(dead_code)]
fn draw_session_overlay(frame: &mut Frame<'_>, app: &mut NativeTuiApp) {
    let overlay_view = build_session_overlay_view(app);
    let SessionOverlayView {
        header_lines,
        list_view,
        detail_lines,
        warning_lines,
        key_lines,
    } = overlay_view;
    let popup_area = centered_rect(90, 78, frame.area());
    frame.render_widget(Clear, popup_area);

    let layout = Layout::default()
        .direction(Direction::Vertical)
        .margin(1)
        .constraints([
            Constraint::Length(block_height_for_lines(&header_lines, 3, 4)),
            Constraint::Min(12),
            Constraint::Length(block_height_for_lines(&warning_lines, 4, 6)),
            Constraint::Length(block_height_for_lines(&key_lines, 3, 5)),
        ])
        .split(popup_area);

    let header = Paragraph::new(header_lines)
        .block(Block::default().borders(Borders::ALL).title("Sessions"));
    frame.render_widget(header, layout[0]);

    let content_layout = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(42), Constraint::Percentage(58)])
        .split(layout[1]);

    draw_session_list_panel(frame, content_layout[0], app, list_view);
    draw_session_detail_panel(frame, content_layout[1], detail_lines);

    frame.render_widget(
        Paragraph::new(warning_lines)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title("Session Warnings"),
            )
            .wrap(Wrap { trim: true }),
        layout[2],
    );

    frame.render_widget(
        Paragraph::new(key_lines).block(Block::default().borders(Borders::ALL).title("Keys")),
        layout[3],
    );
}

#[cfg(test)]
#[allow(dead_code)]
fn draw_followup_template_overlay(frame: &mut Frame<'_>, app: &mut NativeTuiApp) {
    let overlay_view = build_followup_template_overlay_view(app);
    let FollowupTemplateOverlayView {
        header_lines,
        list_view,
        preview_lines,
        status_lines,
        key_lines,
    } = overlay_view;
    let popup_area = centered_rect(92, 82, frame.area());
    frame.render_widget(Clear, popup_area);

    let layout = Layout::default()
        .direction(Direction::Vertical)
        .margin(1)
        .constraints([
            Constraint::Length(4),
            Constraint::Min(14),
            Constraint::Length(block_height_for_lines(&status_lines, 6, 11)),
            Constraint::Length(block_height_for_lines(&key_lines, 5, 7)),
        ])
        .split(popup_area);

    let header = Paragraph::new(header_lines)
        .block(Block::default().borders(Borders::ALL).title("Templates"));
    frame.render_widget(header, layout[0]);

    let content_layout = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(34), Constraint::Percentage(66)])
        .split(layout[1]);

    let preview_scroll = clamp_scroll_offset(
        app.followup_overlay_ui_state.preview_scroll,
        &preview_lines,
        content_layout[1].width.saturating_sub(2),
        content_layout[1].height.saturating_sub(2),
    );
    app.followup_overlay_ui_state.preview_scroll = preview_scroll;

    draw_followup_template_list_panel(frame, content_layout[0], app, list_view);
    frame.render_widget(
        Paragraph::new(preview_lines)
            .block(Block::default().borders(Borders::ALL).title("Preview"))
            .scroll((preview_scroll, 0))
            .wrap(Wrap { trim: false }),
        content_layout[1],
    );

    frame.render_widget(
        Paragraph::new(status_lines)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title("Auto Follow-Up State"),
            )
            .wrap(Wrap { trim: false }),
        layout[2],
    );

    frame.render_widget(
        Paragraph::new(key_lines).block(Block::default().borders(Borders::ALL).title("Keys")),
        layout[3],
    );
}

#[cfg(test)]
#[allow(dead_code)]
fn draw_planning_init_overlay(frame: &mut Frame<'_>, app: &mut NativeTuiApp) {
    if app.planning_init_overlay_ui_state.step() == PlanningInitOverlayStep::ManualEditor {
        draw_planning_draft_editor_overlay(frame, app);
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
    let popup_area = centered_rect(86, 70, frame.area());
    frame.render_widget(Clear, popup_area);

    let layout = Layout::default()
        .direction(Direction::Vertical)
        .margin(1)
        .constraints([
            Constraint::Length(block_height_for_lines(&header_lines, 3, 4)),
            Constraint::Length(block_height_for_lines(&summary_lines, 4, 6)),
            Constraint::Min(8),
            Constraint::Length(block_height_for_lines(&status_lines, 4, 6)),
            Constraint::Length(block_height_for_lines(&key_lines, 3, 5)),
        ])
        .split(popup_area);

    frame.render_widget(
        Paragraph::new(header_lines)
            .block(Block::default().borders(Borders::ALL).title("Planning")),
        layout[0],
    );
    frame.render_widget(
        Paragraph::new(summary_lines)
            .block(Block::default().borders(Borders::ALL).title("Summary"))
            .wrap(Wrap { trim: true }),
        layout[1],
    );
    frame.render_widget(
        Paragraph::new(option_lines)
            .block(Block::default().borders(Borders::ALL).title("Options"))
            .wrap(Wrap { trim: false }),
        layout[2],
    );
    frame.render_widget(
        Paragraph::new(status_lines)
            .block(Block::default().borders(Borders::ALL).title("Status"))
            .wrap(Wrap { trim: true }),
        layout[3],
    );
    frame.render_widget(
        Paragraph::new(key_lines).block(Block::default().borders(Borders::ALL).title("Keys")),
        layout[4],
    );
}

#[cfg(test)]
#[allow(dead_code)]
fn draw_planning_draft_editor_overlay(frame: &mut Frame<'_>, app: &mut NativeTuiApp) {
    let popup_area = centered_rect(92, 82, frame.area());
    frame.render_widget(Clear, popup_area);

    let editor_height = popup_area.height.saturating_sub(14).max(8);
    let Some(overlay_view) =
        build_planning_draft_editor_overlay_view(app, editor_height.saturating_sub(2).max(1))
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

    let layout = Layout::default()
        .direction(Direction::Vertical)
        .margin(1)
        .constraints([
            Constraint::Length(block_height_for_lines(&header_lines, 3, 4)),
            Constraint::Min(editor_height),
            Constraint::Length(block_height_for_lines(&status_lines, 5, 8)),
            Constraint::Length(block_height_for_lines(&key_lines, 4, 6)),
        ])
        .split(popup_area);

    frame.render_widget(
        Paragraph::new(header_lines)
            .block(Block::default().borders(Borders::ALL).title("Planning")),
        layout[0],
    );

    let content_layout = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(28), Constraint::Percentage(72)])
        .split(layout[1]);

    frame.render_widget(
        Paragraph::new(file_lines)
            .block(Block::default().borders(Borders::ALL).title("Files"))
            .wrap(Wrap { trim: false }),
        content_layout[0],
    );
    frame.render_widget(
        Paragraph::new(editor_lines)
            .block(Block::default().borders(Borders::ALL).title(editor_title))
            .scroll((editor_scroll, 0))
            .wrap(Wrap { trim: false }),
        content_layout[1],
    );
    let editor_content_area = Block::default()
        .borders(Borders::ALL)
        .inner(content_layout[1]);
    set_cursor_if_visible(frame, editor_content_area, editor_cursor_offset);

    frame.render_widget(
        Paragraph::new(status_lines)
            .block(Block::default().borders(Borders::ALL).title("Status"))
            .wrap(Wrap { trim: false }),
        layout[2],
    );
    frame.render_widget(
        Paragraph::new(key_lines).block(Block::default().borders(Borders::ALL).title("Keys")),
        layout[3],
    );
}

fn draw_exit_confirmation(frame: &mut Frame<'_>) {
    let popup_area = centered_rect(42, 22, frame.area());
    frame.render_widget(Clear, popup_area);

    let popup = Paragraph::new(vec![
        Line::from("You are already at the shell home."),
        Line::from("Exit codex-exec-loop?"),
        Line::from(""),
        Line::from("y: exit    n: stay"),
    ])
    .block(Block::default().borders(Borders::ALL).title("Confirm Exit"))
    .wrap(Wrap { trim: true });

    frame.render_widget(popup, popup_area);
}

#[cfg(test)]
#[allow(dead_code)]
fn draw_followup_template_list_panel(
    frame: &mut Frame<'_>,
    area: Rect,
    app: &mut NativeTuiApp,
    list_view: OverlayListView,
) {
    if let Some(message_lines) = list_view.message_lines {
        let widget = Paragraph::new(message_lines)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title("Template List"),
            )
            .wrap(Wrap { trim: true });
        frame.render_widget(widget, area);
        return;
    }

    let list = List::new(
        list_view
            .items
            .into_iter()
            .map(|item| ListItem::new(item.lines)),
    )
    .block(
        Block::default()
            .borders(Borders::ALL)
            .title("Template List"),
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
    frame.render_stateful_widget(list, area, &mut app.followup_overlay_ui_state.list_state);
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

fn draw_inline_followup_template_list_panel(
    frame: &mut Frame<'_>,
    area: Rect,
    app: &mut NativeTuiApp,
    list_view: OverlayListView,
) {
    let section_layout = split_inline_section(area);
    frame.render_widget(
        Paragraph::new(vec![Line::from("Template List")]),
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

fn clamp_scroll_offset(
    current_scroll: u16,
    lines: &[Line<'static>],
    content_width: u16,
    visible_height: u16,
) -> u16 {
    current_scroll.min(build_conversation_scroll_offset(
        lines,
        content_width,
        visible_height,
    ))
}

fn centered_rect(horizontal_percent: u16, vertical_percent: u16, area: Rect) -> Rect {
    let horizontal_percent = horizontal_percent.min(100);
    let vertical_percent = vertical_percent.min(100);
    let vertical_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100u16.saturating_sub(vertical_percent)) / 2),
            Constraint::Percentage(vertical_percent),
            Constraint::Percentage((100u16.saturating_sub(vertical_percent)) / 2),
        ])
        .split(area);

    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100u16.saturating_sub(horizontal_percent)) / 2),
            Constraint::Percentage(horizontal_percent),
            Constraint::Percentage((100u16.saturating_sub(horizontal_percent)) / 2),
        ])
        .split(vertical_layout[1])[1]
}

#[cfg(test)]
#[path = "shell_rendering_tests.rs"]
mod tests;
