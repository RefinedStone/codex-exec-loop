use crate::adapter::inbound::tui::shell_chrome::SessionState;
use crate::domain::recent_sessions::SessionCatalog;

use super::capability_copy::{
    attachment_profile_summary_line, recent_session_status_blocked_by_startup,
    recent_session_status_load_failed, recent_session_status_loaded, recent_session_status_loading,
    recent_session_status_not_requested, recent_session_status_partial,
    recent_session_status_ready_to_load, recent_session_status_unsupported,
    recent_session_status_waiting_for_startup, startup_check_loading_lines,
    startup_check_not_started_line, startup_diagnostic_marker, startup_overlay_failed_label,
    startup_overlay_idle_status_line, startup_overlay_readiness_label,
    startup_overlay_running_checks_label, startup_probe_loading_summary_line,
    startup_probe_not_started_line,
};
use super::{AkraTheme, Line, NativeTuiApp, Span, StartupState};

/*
 * capability_projection은 NativeTuiApp의 runtime capability 상태를 renderer-ready Line/String으로
 * 접는 계층이다. capability_copy가 문구 자체를 소유하고, 이 파일은 StartupState/SessionState 같은
 * app state를 읽어 어떤 문구와 색을 선택할지 결정한다. shell_core, popup_frame, inline_inspection은
 * 이 projection 결과만 받아 배치한다.
 */
pub(super) fn build_startup_overlay_summary_lines(app: &NativeTuiApp) -> Vec<Line<'static>> {
    /*
     * startup overlay 상단 요약은 상세 check list보다 먼저 보이는 상태 헤더다.
     * Idle/Loading/Ready/Failed를 각기 다른 두 줄 요약으로 접어, 사용자가 현재 probe가
     * 시작 전인지, 실행 중인지, 계속 가능한지, 완전히 실패했는지 즉시 구분하게 한다.
     */
    match &app.startup_state {
        StartupState::Idle => vec![
            Line::from(startup_overlay_idle_status_line()),
            Line::from(startup_probe_not_started_line()),
        ],
        StartupState::Loading => vec![
            Line::from(vec![
                Span::styled("status: ", AkraTheme::muted()),
                Span::styled(startup_overlay_running_checks_label(), AkraTheme::warning()),
            ]),
            Line::from(startup_probe_loading_summary_line()),
        ],
        StartupState::Ready(diagnostics) => vec![
            Line::from(vec![
                Span::styled("status: ", AkraTheme::muted()),
                Span::styled(
                    startup_overlay_readiness_label(diagnostics.can_continue()),
                    if diagnostics.can_continue() {
                        AkraTheme::success()
                    } else {
                        AkraTheme::warning()
                    },
                ),
            ]),
            /*
             * cwd와 attachment profile은 startup check가 끝난 뒤의 execution context다.
             * 세부 diagnostics list와 별개로 상단에 고정해 operator가 현재 thread 연결 방식을 빠르게 확인한다.
             */
            Line::from(format!("cwd: {}", diagnostics.cwd)),
            Line::from(attachment_profile_summary_line(
                diagnostics.attachment_profile,
            )),
        ],
        StartupState::Failed(message) => vec![
            Line::from(vec![
                Span::styled("status: ", AkraTheme::muted()),
                Span::styled(startup_overlay_failed_label(), AkraTheme::danger()),
            ]),
            Line::from(message.clone()),
        ],
    }
}

pub(super) fn build_startup_check_lines(app: &NativeTuiApp) -> Vec<Line<'static>> {
    /*
     * public facade는 app 전체를 받지만 실제 projection은 StartupState만 필요하다.
     * 테스트와 다른 renderer가 state 단위 helper를 직접 재사용할 수 있도록 아래 함수로 위임한다.
     */
    build_startup_check_lines_from_state(&app.startup_state)
}

pub(super) fn build_startup_check_lines_from_state(
    startup_state: &StartupState,
) -> Vec<Line<'static>> {
    /*
     * startup check list는 summary보다 자세한 capability inventory다.
     * Ready 상태에서는 startup_service가 수집한 각 probe 결과를 같은 marker format으로 정렬해 보여 준다.
     */
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
            /*
             * attachment mode와 recovery anchor는 pass/fail probe가 아니라 선택된 launch profile이다.
             * 그래도 capability panel에서 함께 보여야 startup 이후 session recovery 동작을 예측할 수 있다.
             */
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
    /*
     * warning projection도 StartupState만 필요하다. app facade를 제공해 shell_presentation의 외부 API는
     * NativeTuiApp 중심으로 유지하고, tests는 from_state helper를 호출할 수 있게 한다.
     */
    build_startup_warning_lines_from_state(&app.startup_state)
}

pub(super) fn build_startup_warning_lines_from_state(
    startup_state: &StartupState,
) -> Vec<Line<'static>> {
    /*
     * warnings는 Ready diagnostics의 부가 신호다. 실패 상태는 warning bucket이 아니라 실패 메시지를
     * 직접 보여 주고, 나머지 상태는 operator가 볼 수 있는 "no warnings" placeholder를 유지한다.
     */
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
    /*
     * recent session status는 startup gate와 session loader state가 함께 결정한다.
     * shell header는 긴 SessionCatalog enum을 직접 알 필요 없이 이 label 하나만 받아 표시한다.
     */
    if !app.can_open_session_list() {
        /*
         * startup이 아직 session list를 열 수 없는 상태면 SessionState보다 startup gate가 우선한다.
         * Loading은 기다리는 중, Ready/Failed인데 열 수 없으면 blocked, Idle은 아직 probe 전으로 구분한다.
         */
        return match &app.startup_state {
            StartupState::Loading => recent_session_status_waiting_for_startup().to_string(),
            StartupState::Ready(_) | StartupState::Failed(_) => {
                recent_session_status_blocked_by_startup().to_string()
            }
            StartupState::Idle => recent_session_status_not_requested().to_string(),
        };
    }

    /*
     * startup gate를 통과한 뒤에는 shell_chrome의 SessionState가 source of truth다.
     * Ready 안에서도 catalog tier가 Unsupported/Partial/Ready로 갈라지므로 capability_copy의
     * tier-aware 문구를 사용한다.
     */
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
    /*
     * startup check rows share a compact marker/title/detail format.
     * marker selection stays in capability_copy so icon/copy conventions remain centralized.
     */
    let marker = startup_diagnostic_marker(ok);
    Line::from(format!("{marker} {title}: {detail}"))
}
