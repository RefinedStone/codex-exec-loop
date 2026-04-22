use ratatui::text::Line;

use crate::domain::startup_diagnostics::StartupDiagnostics;

pub(super) fn thread_history_loading_header_line() -> &'static str {
    "Reading thread history from codex app-server."
}

pub(super) fn thread_history_loading_status_line() -> &'static str {
    "status: waiting for thread history from codex app-server"
}

pub(super) fn startup_probe_not_started_line() -> &'static str {
    "startup checks have not started yet"
}

pub(super) fn startup_probe_loading_summary_line() -> &'static str {
    "probing codex binary, app-server handshake, account state, and cwd"
}

pub(super) fn startup_preparing_status_line() -> &'static str {
    "status: preparing startup checks"
}

pub(super) fn startup_initializing_status_line() -> &'static str {
    "status: initializing codex shell"
}

pub(super) fn startup_check_not_started_line() -> &'static str {
    "startup check has not started"
}

pub(super) fn startup_check_loading_lines() -> Vec<Line<'static>> {
    vec![
        Line::from("checking codex binary"),
        Line::from("opening codex app-server"),
        Line::from("reading account state"),
    ]
}

pub(super) fn startup_diagnostics_summary_line(diagnostics: &StartupDiagnostics) -> String {
    format!(
        "diagnostics: codex {}  |  app-server {}  |  account {}",
        inline_diagnostic_status(diagnostics.codex_binary_ok, "ok", "check"),
        inline_diagnostic_status(diagnostics.initialize_ok, "ok", "check"),
        inline_diagnostic_status(diagnostics.account_ok, "ok", "attention"),
    )
}

pub(super) fn recent_session_status_waiting_for_startup() -> &'static str {
    "waiting for startup checks"
}

pub(super) fn recent_session_status_blocked_by_startup() -> &'static str {
    "blocked by startup diagnostics"
}

pub(super) fn recent_session_status_not_requested() -> &'static str {
    "not requested yet"
}

pub(super) fn recent_session_status_ready_to_load() -> &'static str {
    "ready to load"
}

pub(super) fn recent_session_status_loading() -> &'static str {
    "loading from codex app-server"
}

pub(super) fn recent_session_status_load_failed() -> &'static str {
    "load failed"
}

pub(super) fn session_catalog_not_loaded_message(available: bool) -> &'static str {
    if available {
        "session list has not loaded yet"
    } else {
        "recent sessions unlock after startup diagnostics pass"
    }
}

pub(super) fn session_catalog_not_loaded_detail_line(available: bool) -> &'static str {
    if available {
        "session details are not available yet"
    } else {
        "startup diagnostics must pass before recent-session detail is available"
    }
}

pub(super) fn session_catalog_loading_message() -> &'static str {
    "loading recent sessions from codex app-server"
}

pub(super) fn session_catalog_waiting_detail_line() -> &'static str {
    "waiting for session list response"
}

pub(super) fn session_catalog_empty_provider_line() -> &'static str {
    "codex app-server has not returned any recent sessions yet"
}

pub(super) fn session_catalog_warning_waiting_line() -> &'static str {
    "waiting for app-server response"
}

pub(super) fn session_catalog_warning_blocked_line() -> &'static str {
    "recent sessions remain unavailable until startup diagnostics succeed"
}

fn inline_diagnostic_status(
    ok: bool,
    ready_label: &'static str,
    blocked_label: &'static str,
) -> &'static str {
    if ok { ready_label } else { blocked_label }
}
