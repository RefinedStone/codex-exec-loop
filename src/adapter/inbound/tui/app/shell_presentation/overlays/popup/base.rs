use super::super::super::session_browser::{
    build_session_key_lines, build_session_overlay_content, build_session_warning_lines,
};
use super::super::super::{
    Color, Line, Modifier, NativeTuiApp, Span, StartupState, Style, build_startup_check_lines,
    build_startup_warning_lines,
};
use super::super::{SessionOverlayView, StartupOverlayView};

pub(crate) fn build_startup_overlay_view(app: &NativeTuiApp) -> StartupOverlayView {
    let ctrl_o_label = if app.parallel_mode_enabled() {
        "Ctrl+o: supersession board"
    } else {
        "Ctrl+o: recent sessions"
    };
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
            Line::from(ctrl_o_label),
        ],
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
