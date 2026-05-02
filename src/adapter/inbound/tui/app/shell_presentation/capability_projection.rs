// 학습 주석: `use`는 긴 모듈 경로의 이름을 현재 파일로 가져와 아래 코드에서 짧게 쓰도록 합니다.
use crate::adapter::inbound::tui::shell_chrome::SessionState;
// 학습 주석: `use`는 긴 모듈 경로의 이름을 현재 파일로 가져와 아래 코드에서 짧게 쓰도록 합니다.
use crate::domain::recent_sessions::SessionCatalog;

// 학습 주석: `use`는 긴 모듈 경로의 이름을 현재 파일로 가져와 아래 코드에서 짧게 쓰도록 합니다.
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
// 학습 주석: `use`는 긴 모듈 경로의 이름을 현재 파일로 가져와 아래 코드에서 짧게 쓰도록 합니다.
use super::{AkraTheme, Line, NativeTuiApp, Span, StartupState};

// 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
pub(super) fn build_startup_overlay_summary_lines(app: &NativeTuiApp) -> Vec<Line<'static>> {
    // 학습 주석: `match`는 enum이나 값의 모양을 모든 경우로 나누어 처리하는 Rust의 핵심 분기 표현식입니다.
    match &app.startup_state {
        // 학습 주석: `=>` 왼쪽은 매칭될 패턴이고 오른쪽은 그 패턴일 때 실행할 처리입니다.
        StartupState::Idle => vec![
            // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
            Line::from(startup_overlay_idle_status_line()),
            // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
            Line::from(startup_probe_not_started_line()),
        ],
        // 학습 주석: `=>` 왼쪽은 매칭될 패턴이고 오른쪽은 그 패턴일 때 실행할 처리입니다.
        StartupState::Loading => vec![
            // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
            Line::from(vec![
                // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
                Span::styled("status: ", AkraTheme::muted()),
                // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
                Span::styled(startup_overlay_running_checks_label(), AkraTheme::warning()),
            ]),
            // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
            Line::from(startup_probe_loading_summary_line()),
        ],
        // 학습 주석: `=>` 왼쪽은 매칭될 패턴이고 오른쪽은 그 패턴일 때 실행할 처리입니다.
        StartupState::Ready(diagnostics) => vec![
            // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
            Line::from(vec![
                // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
                Span::styled("status: ", AkraTheme::muted()),
                // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
                Span::styled(
                    startup_overlay_readiness_label(diagnostics.can_continue()),
                    // 학습 주석: `if`는 조건이 참일 때만 분기를 실행하며, Rust에서는 조건식이 반드시 bool 값을 내야 합니다.
                    if diagnostics.can_continue() {
                        // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
                        AkraTheme::success()
                    } else {
                        // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
                        AkraTheme::warning()
                    },
                ),
            ]),
            // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
            Line::from(format!("cwd: {}", diagnostics.cwd)),
            // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
            Line::from(attachment_profile_summary_line(
                diagnostics.attachment_profile,
            )),
        ],
        // 학습 주석: `=>` 왼쪽은 매칭될 패턴이고 오른쪽은 그 패턴일 때 실행할 처리입니다.
        StartupState::Failed(message) => vec![
            // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
            Line::from(vec![
                // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
                Span::styled("status: ", AkraTheme::muted()),
                // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
                Span::styled(startup_overlay_failed_label(), AkraTheme::danger()),
            ]),
            // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
            Line::from(message.clone()),
        ],
    }
}

// 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
pub(super) fn build_startup_check_lines(app: &NativeTuiApp) -> Vec<Line<'static>> {
    build_startup_check_lines_from_state(&app.startup_state)
}

// 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
pub(super) fn build_startup_check_lines_from_state(
    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
    startup_state: &StartupState,
) -> Vec<Line<'static>> {
    // 학습 주석: `match`는 enum이나 값의 모양을 모든 경우로 나누어 처리하는 Rust의 핵심 분기 표현식입니다.
    match startup_state {
        // 학습 주석: `=>` 왼쪽은 매칭될 패턴이고 오른쪽은 그 패턴일 때 실행할 처리입니다.
        StartupState::Idle => vec![Line::from(startup_check_not_started_line())],
        // 학습 주석: `=>` 왼쪽은 매칭될 패턴이고 오른쪽은 그 패턴일 때 실행할 처리입니다.
        StartupState::Loading => startup_check_loading_lines(),
        // 학습 주석: `=>` 왼쪽은 매칭될 패턴이고 오른쪽은 그 패턴일 때 실행할 처리입니다.
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
            // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
            Line::from(format!("schema snapshot: {}", diagnostics.schema_snapshot)),
        ],
        // 학습 주석: `=>` 왼쪽은 매칭될 패턴이고 오른쪽은 그 패턴일 때 실행할 처리입니다.
        StartupState::Failed(message) => vec![Line::from(format!("startup error: {message}"))],
    }
}

// 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
pub(super) fn build_startup_warning_lines(app: &NativeTuiApp) -> Vec<Line<'static>> {
    build_startup_warning_lines_from_state(&app.startup_state)
}

// 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
pub(super) fn build_startup_warning_lines_from_state(
    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
    startup_state: &StartupState,
) -> Vec<Line<'static>> {
    // 학습 주석: `match`는 enum이나 값의 모양을 모든 경우로 나누어 처리하는 Rust의 핵심 분기 표현식입니다.
    match startup_state {
        // 학습 주석: `=>` 왼쪽은 매칭될 패턴이고 오른쪽은 그 패턴일 때 실행할 처리입니다.
        StartupState::Ready(diagnostics) if !diagnostics.warnings.is_empty() => diagnostics
            // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
            .warnings
            // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
            .iter()
            // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
            .cloned()
            // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
            .map(Line::from)
            // 학습 주석: 점으로 이어지는 메서드 체인은 앞 단계의 결과를 받아 다음 변환이나 검사를 계속 수행합니다.
            .collect::<Vec<_>>(),
        // 학습 주석: `=>` 왼쪽은 매칭될 패턴이고 오른쪽은 그 패턴일 때 실행할 처리입니다.
        StartupState::Failed(message) => vec![Line::from(message.clone())],
        // 학습 주석: `=>` 왼쪽은 매칭될 패턴이고 오른쪽은 그 패턴일 때 실행할 처리입니다.
        _ => vec![Line::from("no warnings")],
    }
}

// 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
pub(super) fn recent_session_status_label(app: &NativeTuiApp) -> String {
    // 학습 주석: `if`는 조건이 참일 때만 분기를 실행하며, Rust에서는 조건식이 반드시 bool 값을 내야 합니다.
    if !app.can_open_session_list() {
        // 학습 주석: `return`은 현재 함수 실행을 즉시 끝내고 호출자에게 값을 돌려줍니다.
        return match &app.startup_state {
            // 학습 주석: `=>` 왼쪽은 매칭될 패턴이고 오른쪽은 그 패턴일 때 실행할 처리입니다.
            StartupState::Loading => recent_session_status_waiting_for_startup().to_string(),
            // 학습 주석: `=>` 왼쪽은 매칭될 패턴이고 오른쪽은 그 패턴일 때 실행할 처리입니다.
            StartupState::Ready(_) | StartupState::Failed(_) => {
                recent_session_status_blocked_by_startup().to_string()
            }
            // 학습 주석: `=>` 왼쪽은 매칭될 패턴이고 오른쪽은 그 패턴일 때 실행할 처리입니다.
            StartupState::Idle => recent_session_status_not_requested().to_string(),
        };
    }

    // 학습 주석: `match`는 enum이나 값의 모양을 모든 경우로 나누어 처리하는 Rust의 핵심 분기 표현식입니다.
    match &app.session_state {
        // 학습 주석: `=>` 왼쪽은 매칭될 패턴이고 오른쪽은 그 패턴일 때 실행할 처리입니다.
        SessionState::Idle => recent_session_status_ready_to_load().to_string(),
        // 학습 주석: `=>` 왼쪽은 매칭될 패턴이고 오른쪽은 그 패턴일 때 실행할 처리입니다.
        SessionState::Loading => recent_session_status_loading().to_string(),
        // 학습 주석: `=>` 왼쪽은 매칭될 패턴이고 오른쪽은 그 패턴일 때 실행할 처리입니다.
        SessionState::Failed(_) => recent_session_status_load_failed().to_string(),
        // 학습 주석: `=>` 왼쪽은 매칭될 패턴이고 오른쪽은 그 패턴일 때 실행할 처리입니다.
        SessionState::Ready(catalog) => match catalog {
            // 학습 주석: `=>` 왼쪽은 매칭될 패턴이고 오른쪽은 그 패턴일 때 실행할 처리입니다.
            SessionCatalog::Unsupported(status) => recent_session_status_unsupported(status.tier),
            // 학습 주석: `=>` 왼쪽은 매칭될 패턴이고 오른쪽은 그 패턴일 때 실행할 처리입니다.
            SessionCatalog::Partial(status) => recent_session_status_partial(status.tier),
            // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
            SessionCatalog::Ready {
                tier,
                recent_sessions,
                // 학습 주석: `=>` 왼쪽은 매칭될 패턴이고 오른쪽은 그 패턴일 때 실행할 처리입니다.
            } => recent_session_status_loaded(*tier, recent_sessions.items.len()),
        },
    }
}

// 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
fn diagnostic_item(title: &str, ok: bool, detail: &str) -> Line<'static> {
    // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
    let marker = startup_diagnostic_marker(ok);
    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
    Line::from(format!("{marker} {title}: {detail}"))
}
