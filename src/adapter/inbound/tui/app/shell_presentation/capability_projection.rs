use crate::adapter::inbound::tui::shell_chrome::SessionState;
use crate::domain::recent_sessions::SessionCatalog;

use super::capability_copy::{
    attachment_profile_summary_line, recent_session_status_blocked_by_startup,
    recent_session_status_load_failed, recent_session_status_loaded, recent_session_status_loading,
    recent_session_status_not_requested, recent_session_status_partial,
    recent_session_status_ready_to_load, recent_session_status_unsupported,
    recent_session_status_waiting_for_startup, startup_check_loading_lines,
    startup_check_not_started_line, startup_probe_loading_summary_line,
    startup_probe_not_started_line,
};
use super::{Color, Line, NativeTuiApp, Span, StartupState, Style};

pub(super) fn build_startup_overlay_summary_lines(app: &NativeTuiApp) -> Vec<Line<'static>> {
    match &app.startup_state {
        StartupState::Idle => vec![
            Line::from("status: idle"),
            Line::from(startup_probe_not_started_line()),
        ],
        StartupState::Loading => vec![
            Line::from(vec![
                Span::styled("status: ", Style::default().fg(Color::Gray)),
                Span::styled("running checks", Style::default().fg(Color::Yellow)),
            ]),
            Line::from(startup_probe_loading_summary_line()),
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
            Line::from(attachment_profile_summary_line(
                diagnostics.attachment_profile,
            )),
        ],
        StartupState::Failed(message) => vec![
            Line::from(vec![
                Span::styled("status: ", Style::default().fg(Color::Gray)),
                Span::styled("failed", Style::default().fg(Color::Red)),
            ]),
            Line::from(message.clone()),
        ],
    }
}

pub(super) fn build_startup_check_lines(app: &NativeTuiApp) -> Vec<Line<'static>> {
    build_startup_check_lines_from_state(&app.startup_state)
}

pub(super) fn build_startup_check_lines_from_state(
    startup_state: &StartupState,
) -> Vec<Line<'static>> {
    match startup_state {
        StartupState::Idle => vec![Line::from(startup_check_not_started_line())],
        StartupState::Loading => startup_check_loading_lines(),
        StartupState::Ready(diagnostics) => vec![
            diagnostic_item(
                "codex binary",
                diagnostics.codex_binary_ok,
                &diagnostics.codex_binary_detail,
            ),
            diagnostic_item(
                "workspace",
                diagnostics.workspace_ok,
                &diagnostics.workspace_detail,
            ),
            diagnostic_item(
                "app-server initialize",
                diagnostics.initialize_ok,
                &diagnostics.initialize_detail,
            ),
            diagnostic_item(
                "attachment mode",
                true,
                diagnostics.attachment_profile.mode.label(),
            ),
            diagnostic_item(
                "recovery anchor",
                true,
                diagnostics.attachment_profile.recovery_anchor.label(),
            ),
            diagnostic_item(
                "account/read",
                diagnostics.account_ok,
                &diagnostics.account_detail,
            ),
            Line::from(format!("schema snapshot: {}", diagnostics.schema_snapshot)),
        ],
        StartupState::Failed(message) => vec![Line::from(format!("startup error: {message}"))],
    }
}

pub(super) fn build_startup_warning_lines(app: &NativeTuiApp) -> Vec<Line<'static>> {
    build_startup_warning_lines_from_state(&app.startup_state)
}

pub(super) fn build_startup_warning_lines_from_state(
    startup_state: &StartupState,
) -> Vec<Line<'static>> {
    match startup_state {
        StartupState::Ready(diagnostics) if !diagnostics.warnings.is_empty() => diagnostics
            .warnings
            .iter()
            .cloned()
            .map(Line::from)
            .collect::<Vec<_>>(),
        StartupState::Failed(message) => vec![Line::from(message.clone())],
        _ => vec![Line::from("no warnings")],
    }
}

pub(super) fn recent_session_status_label(app: &NativeTuiApp) -> String {
    if !app.can_open_session_list() {
        return match &app.startup_state {
            StartupState::Loading => recent_session_status_waiting_for_startup().to_string(),
            StartupState::Ready(_) | StartupState::Failed(_) => {
                recent_session_status_blocked_by_startup().to_string()
            }
            StartupState::Idle => recent_session_status_not_requested().to_string(),
        };
    }

    match &app.session_state {
        SessionState::Idle => recent_session_status_ready_to_load().to_string(),
        SessionState::Loading => recent_session_status_loading().to_string(),
        SessionState::Failed(_) => recent_session_status_load_failed().to_string(),
        SessionState::Ready(catalog) => match catalog {
            SessionCatalog::Unsupported(status) => recent_session_status_unsupported(status.tier),
            SessionCatalog::Partial(status) => recent_session_status_partial(status.tier),
            SessionCatalog::Ready {
                tier,
                recent_sessions,
            } => recent_session_status_loaded(*tier, recent_sessions.items.len()),
        },
    }
}

fn diagnostic_item(title: &str, ok: bool, detail: &str) -> Line<'static> {
    let marker = if ok { "[ok]" } else { "[warn]" };
    Line::from(format!("{marker} {title}: {detail}"))
}
