use ratatui::text::Line;

use crate::domain::recent_sessions::SessionCatalogTier;
use crate::domain::startup_diagnostics::StartupDiagnostics;
use crate::domain::terminal_bridge_attachment::TerminalBridgeAttachmentProfile;

/*
 * 학습 주석: capability_copy.rs는 startup overlay, session browser, inline tail이 공유하는 짧은 상태 문구 registry입니다.
 * 도메인 값 자체는 startup_diagnostics/recent_sessions에 있고, 이 파일은 같은 상태가 화면마다 다른 단어로 번역되는 일을 막습니다.
 */
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
    // 학습 주석: loading summary는 probe가 여러 dependency를 동시에 확인한다는 점을 한 줄로 압축합니다.
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
    // 학습 주석: can_continue=false는 hard failure만이 아니라 operator attention이 필요한 degraded startup도 포함합니다.
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
    // 학습 주석: startup check loading copy는 shell이 왜 아직 session/thread operations를 열지 않는지 단계별로 보여 줍니다.
    vec![
        Line::from("checking codex binary"),
        Line::from("opening codex app-server"),
        Line::from("reading account state"),
    ]
}

pub(super) fn startup_diagnostics_summary_line(diagnostics: &StartupDiagnostics) -> String {
    // 학습 주석: inline diagnostics는 detail list 대신 codex/app-server/account gate만 빠르게 비교하게 합니다.
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
    // 학습 주석: attachment/recovery는 startup readiness와 별개로 "어떻게 복귀 가능한가"를 설명하는 bridge capability입니다.
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
    // 학습 주석: empty catalog는 error가 아니라 아직 history가 없는 상태라 새 draft와 browser reload 행동을 안내합니다.
    "Start a new draft with n, then reload the browser with r."
}

pub(super) fn session_catalog_not_loaded_message(available: bool) -> &'static str {
    // 학습 주석: available=false는 catalog request 전 대기가 아니라 startup gate가 막은 상태임을 명확히 구분합니다.
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
    // 학습 주석: compact lists use bracket markers instead of verbose labels to keep diagnostics scannable in narrow panes.
    if ok { "[ok]" } else { "[warn]" }
}

pub(super) fn session_catalog_tier_line(tier: SessionCatalogTier) -> String {
    format!("catalog tier: {}", tier.label())
}

pub(super) fn session_catalog_unsupported_message(tier: SessionCatalogTier) -> &'static str {
    // 학습 주석: catalog unsupported copy는 attach-only, handle reattach, provider catalog 실패가 서로 다른 UX를 뜻하므로 tier별로 나눕니다.
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
    // 학습 주석: partial detail은 catalog provider가 준 실행 가능한 진단이므로 문구를 그대로 보존합니다.
    detail.to_string()
}

fn inline_diagnostic_status(
    ok: bool,
    ready_label: &'static str,
    blocked_label: &'static str,
) -> &'static str {
    if ok { ready_label } else { blocked_label }
}
