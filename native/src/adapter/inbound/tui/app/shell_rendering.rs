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

    match app.shell_overlay {
        ShellOverlay::Hidden => {}
        ShellOverlay::Startup => draw_startup_overlay(frame, app),
        ShellOverlay::Sessions => draw_session_overlay(frame, app),
        ShellOverlay::FollowupTemplates => draw_followup_template_overlay(frame, app),
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
    let area = frame.area();
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
    let header_height = inline_section_height(&header_lines, MAX_SHELL_HEADER_HEIGHT);
    let footer_height = inline_section_height(&footer_lines, MAX_SHELL_STATUS_HEIGHT);
    let input_height = inline_section_height(&input_lines, MAX_COMPOSER_HEIGHT);

    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(header_height),
            Constraint::Min(MIN_TRANSCRIPT_PANEL_HEIGHT.saturating_sub(2).max(6)),
            Constraint::Length(footer_height),
            Constraint::Length(input_height),
        ])
        .split(area);

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

fn draw_startup_overlay(frame: &mut Frame<'_>, app: &NativeTuiApp) {
    let overlay_view = build_startup_overlay_view(app);
    let StartupOverlayView {
        header_lines,
        summary_lines,
        check_items,
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
        List::new(check_items).block(Block::default().borders(Borders::ALL).title("Checks")),
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
            .position(|line| line.contains("Inline Shell"))
            .expect("inline shell title should render");
        let header_line = rendered_lines
            .iter()
            .position(|line| line.contains("Conversation Shell"))
            .expect("header content should render");

        assert!(rendered.contains("Inline Shell"));
        assert!(rendered.contains("History"));
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

        assert!(rendered.contains("Shell"));
        assert!(rendered.contains("Transcript"));
        assert!(rendered.contains("┌"));
        assert!(rendered.contains("│"));
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
}
