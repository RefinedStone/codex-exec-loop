use ratatui::text::Line;

use crate::domain::recent_sessions::SessionCatalogTier;
use crate::domain::startup_diagnostics::StartupDiagnostics;
use crate::domain::terminal_bridge_attachment::TerminalBridgeAttachmentProfile;

#[cfg(test)]
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

pub(super) fn startup_overlay_idle_status_line() -> &'static str {
    "status: idle"
}

pub(super) fn startup_overlay_running_checks_label() -> &'static str {
    "running checks"
}

pub(super) fn startup_overlay_readiness_label(can_continue: bool) -> &'static str {
    if can_continue {
        "ready"
    } else {
        "needs attention"
    }
}

pub(super) fn startup_overlay_failed_label() -> &'static str {
    "failed"
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

pub(super) fn startup_attachment_summary_line(diagnostics: &StartupDiagnostics) -> String {
    attachment_profile_summary_line(diagnostics.attachment_profile)
}

pub(super) fn attachment_profile_summary_line(profile: TerminalBridgeAttachmentProfile) -> String {
    format!(
        "attachment: {}  |  recovery: {}",
        profile.mode.label(),
        profile.recovery_anchor.label()
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

pub(super) fn recent_session_status_unsupported(tier: SessionCatalogTier) -> String {
    format!("{}: catalog unsupported", tier.label())
}

pub(super) fn recent_session_status_partial(tier: SessionCatalogTier) -> String {
    format!("{}: partial catalog", tier.label())
}

pub(super) fn recent_session_status_loaded(tier: SessionCatalogTier, count: usize) -> String {
    format!("{}: {} loaded", tier.label(), count)
}

pub(super) fn session_catalog_empty_message_line() -> &'static str {
    "no recent sessions have been recorded yet"
}

pub(super) fn session_catalog_empty_action_hint_line() -> &'static str {
    "Start a new draft with n, then reload the browser with r."
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

pub(super) fn startup_diagnostic_marker(ok: bool) -> &'static str {
    if ok { "[ok]" } else { "[warn]" }
}

pub(super) fn session_catalog_tier_line(tier: SessionCatalogTier) -> String {
    format!("catalog tier: {}", tier.label())
}

pub(super) fn session_catalog_unsupported_message(tier: SessionCatalogTier) -> &'static str {
    match tier {
        SessionCatalogTier::AttachOnly => "this bridge does not expose a recent-session catalog",
        SessionCatalogTier::HandleBasedReattach => {
            "this bridge can reattach by handle, but no queryable session catalog is available"
        }
        SessionCatalogTier::ProviderBackedCatalog => {
            "the provider-backed session catalog is unavailable right now"
        }
    }
}

pub(super) fn session_catalog_unsupported_detail_line(tier: SessionCatalogTier) -> &'static str {
    match tier {
        SessionCatalogTier::AttachOnly => {
            "attach-only bridges can launch or attach without listing prior sessions"
        }
        SessionCatalogTier::HandleBasedReattach => {
            "handle-based reattach keeps a stable recovery anchor even when listing is unsupported"
        }
        SessionCatalogTier::ProviderBackedCatalog => {
            "provider-backed session metadata is not currently queryable"
        }
    }
}

pub(super) fn session_catalog_partial_message(tier: SessionCatalogTier) -> String {
    format!("{} is only partially available", tier.label())
}

pub(super) fn session_catalog_partial_detail_line(detail: &str) -> String {
    detail.to_string()
}

fn inline_diagnostic_status(
    ok: bool,
    ready_label: &'static str,
    blocked_label: &'static str,
) -> &'static str {
    if ok { ready_label } else { blocked_label }
}
