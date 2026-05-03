use ratatui::text::Line;

use crate::domain::recent_sessions::SessionCatalogTier;
use crate::domain::startup_diagnostics::StartupDiagnostics;
use crate::domain::terminal_bridge_attachment::TerminalBridgeAttachmentProfile;

/*
 * capability_copy.rs는 capability_projection, session_browser, tail/status panel이 공유하는
 * operator-facing 문구 registry다. StartupDiagnostics와 SessionCatalog는 domain 상태를 소유하고,
 * 이 파일은 그 상태가 overlay마다 다른 단어로 번역되는 것을 막아 startup gate와 recent-session
 * capability를 같은 vocabulary로 보여 준다.
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
    /*
     * Loading 상태의 상단 summary는 아직 어떤 probe가 실패했는지 판단하지 않는다.
     * 여러 dependency를 병렬로 여는 구간임을 말해, 사용자가 session browser나 prompt가 잠긴 이유를
     * "검사 중"으로 이해하게 한다.
     */
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
    /*
     * can_continue는 StartupDiagnostics의 필수 gate 네 가지를 접은 결과다. false는 단순 crash만이
     * 아니라 workspace/account/app-server처럼 operator 조치가 필요한 degraded startup도 포함하므로
     * overlay label은 "failed"보다 넓은 "needs attention"으로 둔다.
     */
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
    /*
     * check list의 Loading copy는 summary보다 낮은 해상도의 진행 표식이다. 아직 결과 detail이 없을 때도
     * shell이 Codex binary, app-server, account gate 때문에 session/thread operations를 열지 않았음을
     * 단계별로 보여 준다.
     */
    vec![
        Line::from("checking codex binary"),
        Line::from("opening codex app-server"),
        Line::from("reading account state"),
    ]
}

pub(super) fn startup_diagnostics_summary_line(diagnostics: &StartupDiagnostics) -> String {
    /*
     * inline tail은 full startup check list를 반복할 공간이 없다. 여기서는 prompt submission에 직접
     * 영향을 주는 codex/app-server/account gate만 한 줄로 압축해, 사용자가 현재 막힌 축을 빠르게
     * 비교할 수 있게 한다.
     */
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
    /*
     * attachment/recovery는 pass/fail readiness가 아니라 현재 terminal bridge가 어떤 방식으로
     * 붙었고 다시 붙을 수 있는지를 설명하는 capability다. startup summary에 같이 두면 launch profile과
     * session recovery expectation을 한 줄에서 확인할 수 있다.
     */
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
    /*
     * Empty catalog는 provider failure가 아니라 성공 payload 안의 빈 목록이다. 그래서 error wording 대신
     * 사용자가 다음으로 할 수 있는 새 draft 작성과 browser reload 행동을 안내한다.
     */
    "Start a new draft with n, then reload the browser with r."
}

pub(super) fn session_catalog_not_loaded_message(available: bool) -> &'static str {
    /*
     * available=false는 "아직 요청하지 않음"이 아니라 startup diagnostics가 recent-session capability를
     * 막은 상태다. session browser가 loader 문제처럼 보이지 않도록 gate 원인을 copy 단계에서 분리한다.
     */
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
    /*
     * startup check rows는 popup과 inline inspection의 좁은 폭을 공유한다. verbose 상태 문구 대신
     * bracket marker를 쓰면 title/detail을 보존하면서도 여러 capability를 빠르게 훑을 수 있다.
     */
    if ok { "[ok]" } else { "[warn]" }
}

pub(super) fn session_catalog_tier_line(tier: SessionCatalogTier) -> String {
    format!("catalog tier: {}", tier.label())
}

pub(super) fn session_catalog_unsupported_message(tier: SessionCatalogTier) -> &'static str {
    /*
     * Unsupported catalog는 모두 "목록 없음"처럼 보이지만 UX는 tier마다 다르다. attach-only는 목록 API가
     * 없는 설계이고, handle-based reattach는 복구 anchor만 있으며, provider-backed catalog는 기대한
     * backend가 일시적으로 query 불가능한 상태다.
     */
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
    /*
     * Partial detail은 provider/service가 만든 actionable diagnostic이다. presentation copy가 이를
     * 다시 요약하면 operator가 필요한 원인 문자열을 잃을 수 있어 원문을 그대로 보존한다.
     */
    detail.to_string()
}

fn inline_diagnostic_status(
    ok: bool,
    ready_label: &'static str,
    blocked_label: &'static str,
) -> &'static str {
    if ok { ready_label } else { blocked_label }
}
