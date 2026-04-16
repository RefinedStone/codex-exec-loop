#[cfg(test)]
use super::super::super::planning::status_projection::build_planning_status_surface_projection;
#[cfg(test)]
use super::super::super::planning::build_planner_panel_lines;
#[cfg(test)]
use super::super::{
    Block, Borders, Constraint, ConversationShellFrameView, ConversationShellView, Direction,
    FOOTER_NOTICE_DETAIL_LIMIT, FOOTER_PLANNING_DETAIL_LIMIT, Layout, MAX_SHELL_HEADER_HEIGHT,
    MIN_SHELL_HEADER_HEIGHT, MIN_TRANSCRIPT_PANEL_HEIGHT, Rect, SHELL_FRAME_MARGIN,
    ShellFrontendMode, TranscriptPanelView, block_height_for_lines,
    build_conversation_lines_with_context, build_conversation_scroll_offset,
    build_frontend_summary_line, build_input_block_height, build_input_lines_with_context,
    build_input_title_with_context, build_shell_footer_height,
    build_shell_footer_lines_with_context, build_shell_header_lines_with_context,
    build_shell_title, build_status_title, build_transcript_title_with_context,
    current_live_agent_lines, current_plan_mode_indicator,
};
use super::super::{
    Color, Line, Modifier, NativeTuiApp, ShellCorePresentationContext, Span, StartupState, Style,
    build_session_key_lines, build_session_overlay_content, build_session_warning_lines,
    build_startup_check_lines, build_startup_warning_lines, startup_ascii_art_lines,
};
use super::{SessionOverlayView, StartupOverlayView};

pub(crate) fn build_startup_banner_lines(
    app: &NativeTuiApp,
    max_height: Option<u16>,
) -> Option<Vec<Line<'static>>> {
    let context = ShellCorePresentationContext::from_app(app);
    if !context.startup_banner_is_active() {
        return None;
    }

    let max_height = match max_height {
        Some(0) => return None,
        value => value,
    };

    Some(startup_ascii_art_lines(max_height))
}

#[cfg(test)]
pub(crate) fn build_conversation_shell_view(
    app: &NativeTuiApp,
    mode: ShellFrontendMode,
) -> ConversationShellView {
    let _ = mode;
    let context = ShellCorePresentationContext::from_app(app);
    let plan_mode_indicator = current_plan_mode_indicator(app);
    let (planning_summary_line, planning_notice_line) = context
        .ready_conversation()
        .map(|conversation| {
            let projection = build_planning_status_surface_projection(
                app,
                conversation,
                FOOTER_PLANNING_DETAIL_LIMIT,
                FOOTER_NOTICE_DETAIL_LIMIT,
                false,
            );
            (projection.summary_line, projection.notice_line)
        })
        .unwrap_or((None, None));
    let planner_panel_lines = build_planner_panel_lines(app, FOOTER_NOTICE_DETAIL_LIMIT);
    let mut header_lines = build_shell_header_lines_with_context(&context);
    header_lines.push(build_frontend_summary_line());
    let mut footer_lines = build_shell_footer_lines_with_context(
        &context,
        plan_mode_indicator,
        app.github_review_recent_changes_summary(FOOTER_NOTICE_DETAIL_LIMIT),
        planning_summary_line,
        planning_notice_line,
        planner_panel_lines,
    );
    if mode == ShellFrontendMode::InlineMainBuffer
        && let Some(live_agent_lines) = context
            .ready_conversation()
            .and_then(current_live_agent_lines)
    {
        footer_lines.extend(live_agent_lines);
    }

    ConversationShellView {
        shell_title: build_shell_title(),
        header_lines,
        conversation_lines: build_conversation_lines_with_context(&context),
        status_title: build_status_title(),
        footer_lines,
        input_title: build_input_title_with_context(&context),
        input_lines: build_input_lines_with_context(&context),
    }
}

pub(crate) fn build_startup_overlay_view(app: &NativeTuiApp) -> StartupOverlayView {
    StartupOverlayView {
        header_lines: vec![
            Line::from(vec![
                Span::styled(
                    "Startup Diagnostics",
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw(" / shell inspection"),
            ]),
            Line::from("Inspect readiness without leaving the live shell."),
        ],
        summary_lines: match &app.startup_state {
            StartupState::Idle => vec![
                Line::from("status: idle"),
                Line::from("startup checks have not started yet"),
            ],
            StartupState::Loading => vec![
                Line::from(vec![
                    Span::styled("status: ", Style::default().fg(Color::Gray)),
                    Span::styled("running checks", Style::default().fg(Color::Yellow)),
                ]),
                Line::from("probing codex binary, app-server handshake, account state, and cwd"),
            ],
            StartupState::Ready(diagnostics) => vec![
                Line::from(vec![
                    Span::styled("status: ", Style::default().fg(Color::Gray)),
                    Span::styled(
                        if diagnostics.can_continue() {
                            "ready"
                        } else {
                            "needs attention"
                        },
                        Style::default().fg(if diagnostics.can_continue() {
                            Color::Green
                        } else {
                            Color::Yellow
                        }),
                    ),
                ]),
                Line::from(format!("cwd: {}", diagnostics.cwd)),
            ],
            StartupState::Failed(message) => vec![
                Line::from(vec![
                    Span::styled("status: ", Style::default().fg(Color::Gray)),
                    Span::styled("failed", Style::default().fg(Color::Red)),
                ]),
                Line::from(message.clone()),
            ],
        },
        check_lines: build_startup_check_lines(app),
        warning_lines: build_startup_warning_lines(app),
        key_lines: vec![
            Line::from("Esc/Ctrl+C: close    r: rerun checks"),
            Line::from("Ctrl+o: recent sessions"),
        ],
    }
}

#[cfg(test)]
pub(crate) fn build_conversation_shell_frame_view(
    app: &mut NativeTuiApp,
    mode: ShellFrontendMode,
    area: Rect,
) -> ConversationShellFrameView {
    let _ = mode;
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
    let header_height = block_height_for_lines(
        &header_lines,
        MIN_SHELL_HEADER_HEIGHT,
        MAX_SHELL_HEADER_HEIGHT,
    );
    let footer_height = build_shell_footer_height(&footer_lines);
    let input_height = build_input_block_height(&input_lines);

    let layout = Layout::default()
        .direction(Direction::Vertical)
        .margin(SHELL_FRAME_MARGIN)
        .constraints([
            Constraint::Length(header_height),
            Constraint::Min(MIN_TRANSCRIPT_PANEL_HEIGHT),
            Constraint::Length(footer_height),
            Constraint::Length(input_height),
        ])
        .split(area);
    let transcript_inner = Block::default().borders(Borders::ALL).inner(layout[1]);

    let transcript_view = build_transcript_panel_view(
        app,
        conversation_lines,
        transcript_inner.width,
        transcript_inner.height,
    );

    ConversationShellFrameView {
        shell_title,
        header_lines,
        header_area: layout[0],
        transcript_view,
        transcript_area: layout[1],
        status_title,
        footer_lines,
        footer_area: layout[2],
        input_title,
        input_lines,
        input_area: layout[3],
    }
}

#[cfg(test)]
pub(crate) fn build_transcript_panel_view(
    app: &mut NativeTuiApp,
    lines: Vec<Line<'static>>,
    content_width: u16,
    visible_height: u16,
) -> TranscriptPanelView {
    let max_scroll_offset = build_conversation_scroll_offset(&lines, content_width, visible_height);
    let _ = visible_height;

    TranscriptPanelView {
        title: build_transcript_title_with_context(&ShellCorePresentationContext::from_app(app)),
        lines,
        scroll_offset: max_scroll_offset,
    }
}

pub(crate) fn build_session_overlay_view(app: &NativeTuiApp) -> SessionOverlayView {
    let (list_view, detail_lines) = build_session_overlay_content(app);

    SessionOverlayView {
        header_lines: vec![
            Line::from(vec![
                Span::styled(
                    "Recent Sessions",
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw(" / shell inspection"),
            ]),
            Line::from("Resume a thread without leaving the shell view."),
        ],
        list_view,
        detail_lines,
        warning_lines: build_session_warning_lines(app),
        key_lines: build_session_key_lines(app),
    }
}
