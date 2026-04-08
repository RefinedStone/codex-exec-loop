use std::rc::Rc;

use super::shell_presentation::{
    ConversationShellFrameView, ConversationShellView, FollowupTemplateOverlayView,
    OverlayListView, SessionOverlayView, StartupOverlayView, build_conversation_shell_frame_view,
    build_conversation_shell_view, build_followup_template_overlay_view,
    build_session_overlay_view, build_startup_overlay_view, build_transcript_panel_view,
};
use super::*;

pub(super) fn draw(frame: &mut Frame<'_>, app: &mut NativeTuiApp, mode: ShellFrontendMode) {
    draw_conversation_shell(frame, app, mode);

    match (mode, app.shell_overlay) {
        (_, ShellOverlay::Hidden) => {}
        (ShellFrontendMode::InlineMainBuffer, _) => draw_inline_shell_inspection(frame, app, mode),
        (_, ShellOverlay::Startup) => draw_startup_overlay(frame, app),
        (_, ShellOverlay::Sessions) => draw_session_overlay(frame, app),
        (_, ShellOverlay::FollowupTemplates) => draw_followup_template_overlay(frame, app),
    }

    if app.is_exit_confirmation_visible() {
        draw_exit_confirmation(frame);
    }
}

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

fn draw_conversation_shell(frame: &mut Frame<'_>, app: &mut NativeTuiApp, mode: ShellFrontendMode) {
    match mode {
        ShellFrontendMode::InlineMainBuffer => draw_inline_conversation_shell(frame, app, mode),
        ShellFrontendMode::AlternateScreen => {
            draw_framed_conversation_shell(frame, app, mode);
        }
    }
}

fn draw_inline_conversation_shell(
    frame: &mut Frame<'_>,
    app: &mut NativeTuiApp,
    mode: ShellFrontendMode,
) {
    let layout = build_inline_shell_layout(app, mode, frame.area());
    let shell_view = build_conversation_shell_view(app, mode);
    let ConversationShellView {
        shell_title,
        header_lines,
        conversation_lines,
        status_title,
        footer_lines,
        input_title,
        input_lines,
    } = shell_view;

    let transcript_view = build_transcript_panel_view(
        app,
        mode,
        conversation_lines,
        layout[1].width,
        layout[1].height.saturating_sub(1).max(1),
    );

    render_inline_section(frame, layout[0], shell_title, header_lines, true);
    render_inline_scrolled_section(
        frame,
        layout[1],
        transcript_view.title,
        transcript_view.lines,
        transcript_view.scroll_offset,
    );
    render_inline_section(frame, layout[2], status_title, footer_lines, true);
    render_inline_section(frame, layout[3], input_title, input_lines, false);
}

fn build_inline_shell_layout(
    app: &NativeTuiApp,
    mode: ShellFrontendMode,
    area: Rect,
) -> Rc<[Rect]> {
    let shell_view = build_conversation_shell_view(app, mode);
    let header_height = inline_section_height(&shell_view.header_lines, MAX_SHELL_HEADER_HEIGHT);
    let footer_height = inline_section_height(&shell_view.footer_lines, MAX_SHELL_STATUS_HEIGHT);
    let input_height = inline_section_height(&shell_view.input_lines, MAX_COMPOSER_HEIGHT);

    Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(header_height),
            Constraint::Min(MIN_TRANSCRIPT_PANEL_HEIGHT.saturating_sub(2).max(6)),
            Constraint::Length(footer_height),
            Constraint::Length(input_height),
        ])
        .split(area)
}

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
}

fn inline_section_height(lines: &[Line<'_>], max_height: u16) -> u16 {
    lines
        .len()
        .saturating_add(1)
        .max(2)
        .min(max_height as usize) as u16
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

fn take_panel_title(
    mut header_lines: Vec<Line<'static>>,
    fallback: &str,
) -> (Line<'static>, Vec<Line<'static>>) {
    let title = if header_lines.is_empty() {
        Line::from(fallback.to_string())
    } else {
        header_lines.remove(0)
    };
    (title, header_lines)
}

fn draw_inline_shell_inspection(
    frame: &mut Frame<'_>,
    app: &mut NativeTuiApp,
    mode: ShellFrontendMode,
) {
    let inspection_area = build_inline_shell_layout(app, mode, frame.area())[1];
    frame.render_widget(Clear, inspection_area);

    match app.shell_overlay {
        ShellOverlay::Hidden => {}
        ShellOverlay::Startup => draw_inline_startup_inspection(frame, inspection_area, app),
        ShellOverlay::Sessions => draw_inline_session_inspection(frame, inspection_area, app),
        ShellOverlay::FollowupTemplates => {
            draw_inline_followup_template_inspection(frame, inspection_area, app)
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
    let (title, body_lines) = take_panel_title(header_lines, "Startup Diagnostics");
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

    render_inline_section(frame, layout[0], title, body_lines, true);
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
    let (title, body_lines) = take_panel_title(header_lines, "Recent Sessions");
    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(inline_section_height(&body_lines, 4)),
            Constraint::Min(8),
            Constraint::Length(inline_section_height(&warning_lines, 5)),
            Constraint::Length(inline_section_height(&key_lines, 4)),
        ])
        .split(area);

    render_inline_section(frame, layout[0], title, body_lines, true);

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
    let (title, body_lines) = take_panel_title(header_lines, "Follow-Up Templates");
    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(inline_section_height(&body_lines, 4)),
            Constraint::Min(10),
            Constraint::Length(inline_section_height(&status_lines, 11)),
            Constraint::Length(inline_section_height(&key_lines, 6)),
        ])
        .split(area);

    render_inline_section(frame, layout[0], title, body_lines, true);

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
mod tests {
    use std::sync::Arc;

    use anyhow::Result;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    use super::*;
    use crate::application::port::outbound::codex_app_server_port::{
        AppServerStartupContext, CodexAppServerPort,
    };
    use crate::application::port::outbound::followup_template_port::{
        FollowupTemplatePort, WorkspaceFollowupTemplateRecord,
    };
    use crate::application::service::conversation_service::ConversationService;
    use crate::application::service::followup_template_service::FollowupTemplateService;
    use crate::application::service::session_service::SessionService;
    use crate::application::service::startup_service::StartupService;
    use crate::domain::conversation::{ConversationSnapshot, ConversationStreamEvent};
    use crate::domain::recent_sessions::RecentSessions;
    use crate::domain::session_summary::SessionSummary;
    use crate::domain::startup_diagnostics::StartupDiagnostics;

    #[test]
    fn centered_rect_clamps_percentages_above_hundred() {
        let area = Rect::new(4, 2, 80, 24);

        assert_eq!(centered_rect(140, 120, area), area);
    }

    #[test]
    fn inline_main_buffer_rendering_avoids_box_borders() {
        let mut terminal = Terminal::new(TestBackend::new(80, 24)).expect("test terminal");
        let mut app = make_test_app();

        terminal
            .draw(|frame| draw(frame, &mut app, ShellFrontendMode::InlineMainBuffer))
            .expect("inline render succeeds");

        let rendered = format!("{}", terminal.backend());
        let rendered_lines = rendered.lines().collect::<Vec<_>>();
        let inline_shell_line = rendered_lines
            .iter()
            .position(|line| line.contains("Shell / Ctrl+t new draft"))
            .expect("inline shell title should render");
        let header_line = rendered_lines
            .iter()
            .position(|line| line.contains("thread:"))
            .expect("header content should render");

        assert!(rendered.contains("Shell / Ctrl+t new draft"));
        assert!(rendered.contains("Transcript"));
        assert_ne!(inline_shell_line, header_line);
        assert!(!rendered.contains("┌"));
        assert!(!rendered.contains("│"));
    }

    #[test]
    fn alternate_screen_rendering_keeps_bordered_frame() {
        let mut terminal = Terminal::new(TestBackend::new(80, 24)).expect("test terminal");
        let mut app = make_test_app();

        terminal
            .draw(|frame| draw(frame, &mut app, ShellFrontendMode::AlternateScreen))
            .expect("alternate render succeeds");

        let rendered = format!("{}", terminal.backend());

        assert!(rendered.contains("Shell / Ctrl+t new draft"));
        assert!(rendered.contains("Transcript"));
        assert!(rendered.contains("┌"));
        assert!(rendered.contains("│"));
    }

    #[test]
    fn inline_startup_inspection_replaces_transcript_panel() {
        let mut terminal = Terminal::new(TestBackend::new(96, 28)).expect("test terminal");
        let mut app = make_test_app();
        app.startup_state = StartupState::Ready(sample_startup_diagnostics());
        app.shell_overlay = ShellOverlay::Startup;

        terminal
            .draw(|frame| draw(frame, &mut app, ShellFrontendMode::InlineMainBuffer))
            .expect("inline inspection render succeeds");

        let rendered = format!("{}", terminal.backend());

        assert!(rendered.contains("Startup Diagnostics / shell inspection"));
        assert!(rendered.contains("Checks"));
        assert!(rendered.contains("schema snapshot: snapshot.json"));
        assert!(!rendered.contains("Transcript /"));
        assert!(!rendered.contains("┌"));
    }

    #[test]
    fn inline_sessions_inspection_renders_browser_panels_without_popup_frame() {
        let mut terminal = Terminal::new(TestBackend::new(96, 28)).expect("test terminal");
        let mut app = make_test_app();
        app.startup_state = StartupState::Ready(sample_startup_diagnostics());
        app.session_state = SessionState::Ready(RecentSessions {
            items: vec![sample_session("thread-1"), sample_session("thread-2")],
            warnings: vec!["cache is stale".to_string()],
            next_cursor: None,
        });
        app.shell_overlay = ShellOverlay::Sessions;

        terminal
            .draw(|frame| draw(frame, &mut app, ShellFrontendMode::InlineMainBuffer))
            .expect("inline session inspection render succeeds");

        let rendered = format!("{}", terminal.backend());

        assert!(rendered.contains("Recent Sessions / shell inspection"));
        assert!(rendered.contains("Threads"));
        assert!(rendered.contains("Selected Session"));
        assert!(rendered.contains("Session Warnings"));
        assert!(!rendered.contains("Transcript /"));
        assert!(!rendered.contains("┌"));
    }

    #[test]
    fn inline_followup_inspection_renders_preview_inside_shell_frame() {
        let mut terminal = Terminal::new(TestBackend::new(96, 28)).expect("test terminal");
        let mut app = make_test_app();
        app.show_followup_template_overlay();

        terminal
            .draw(|frame| draw(frame, &mut app, ShellFrontendMode::InlineMainBuffer))
            .expect("inline followup inspection render succeeds");

        let rendered = format!("{}", terminal.backend());

        assert!(rendered.contains("Follow-Up Templates / shell inspection"));
        assert!(rendered.contains("Template List"));
        assert!(rendered.contains("Preview"));
        assert!(rendered.contains("auto follow-up: on"));
        assert!(!rendered.contains("Transcript /"));
        assert!(!rendered.contains("┌"));
    }

    struct FakeCodexAppServerPort;

    impl CodexAppServerPort for FakeCodexAppServerPort {
        fn load_startup_context(&self) -> Result<AppServerStartupContext> {
            Ok(AppServerStartupContext {
                initialize_detail: "ok".to_string(),
                account_detail: "ok".to_string(),
                account_ok: true,
                warnings: Vec::new(),
            })
        }

        fn load_recent_sessions(&self, _limit: usize) -> Result<RecentSessions> {
            Ok(RecentSessions {
                items: Vec::new(),
                warnings: Vec::new(),
                next_cursor: None,
            })
        }

        fn load_conversation_snapshot(&self, thread_id: &str) -> Result<ConversationSnapshot> {
            Ok(ConversationSnapshot {
                thread_id: thread_id.to_string(),
                title: "Loaded thread".to_string(),
                cwd: "/tmp/root".to_string(),
                messages: Vec::new(),
                warnings: Vec::new(),
                runtime_notices: Vec::new(),
            })
        }

        fn run_new_thread_stream(
            &self,
            _cwd: &str,
            _prompt: &str,
            _event_sender: std::sync::mpsc::Sender<ConversationStreamEvent>,
        ) -> Result<()> {
            Ok(())
        }

        fn run_turn_stream(
            &self,
            _thread_id: &str,
            _prompt: &str,
            _event_sender: std::sync::mpsc::Sender<ConversationStreamEvent>,
        ) -> Result<()> {
            Ok(())
        }
    }

    struct FakeFollowupTemplatePort;

    impl FollowupTemplatePort for FakeFollowupTemplatePort {
        fn load_workspace_templates(
            &self,
            _workspace_dir: &str,
        ) -> Result<Vec<WorkspaceFollowupTemplateRecord>> {
            Ok(Vec::new())
        }
    }

    fn make_test_app() -> NativeTuiApp {
        let codex_port = Arc::new(FakeCodexAppServerPort);
        let followup_port = Arc::new(FakeFollowupTemplatePort);
        NativeTuiApp::new(
            StartupService::new(codex_port.clone()),
            SessionService::new(codex_port.clone()),
            ConversationService::new(codex_port),
            FollowupTemplateService::new(followup_port),
        )
    }

    fn sample_startup_diagnostics() -> StartupDiagnostics {
        StartupDiagnostics {
            cwd: "/tmp/root".to_string(),
            codex_binary_ok: true,
            codex_binary_detail: "codex".to_string(),
            workspace_ok: true,
            workspace_path: "/tmp/root".to_string(),
            workspace_detail: "workspace found".to_string(),
            initialize_ok: true,
            initialize_detail: "app-server initialize ok".to_string(),
            account_ok: true,
            account_detail: "account ok".to_string(),
            warnings: Vec::new(),
            schema_snapshot: "snapshot.json".to_string(),
        }
    }

    fn sample_session(id: &str) -> SessionSummary {
        SessionSummary {
            id: id.to_string(),
            name: Some(format!("Session {id}")),
            preview: "Preview line".to_string(),
            cwd: "/tmp/root".to_string(),
            source: "native".to_string(),
            model_provider: "openai".to_string(),
            updated_at_epoch: 1_700_000_000,
            status_type: "ready".to_string(),
            path: format!("/tmp/root/{id}.json"),
            git_branch: Some("feature/demo".to_string()),
        }
    }
}
