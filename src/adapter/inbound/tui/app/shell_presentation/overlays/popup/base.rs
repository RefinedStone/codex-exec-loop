// 학습 주석: session browser helpers는 session overlay의 list/detail/warning/key copy를 app state에서 뽑아 옵니다.
// base popup builder는 그 결과를 SessionOverlayView field에 배치하는 조립 역할만 맡습니다.
use super::super::super::session_browser::{
    build_session_key_lines, build_session_overlay_content, build_session_warning_lines,
};
// 학습 주석: startup popup은 app의 startup diagnostics projection을 여러 line group으로 나눠 사용합니다.
// AkraTheme/Line은 이 projection을 ratatui renderer가 받을 presentation object로 만드는 데 필요합니다.
use super::super::super::{
    AkraTheme, Line, NativeTuiApp, build_startup_check_lines, build_startup_overlay_summary_lines,
    build_startup_warning_lines,
};
// 학습 주석: popup view DTO들은 popup/views.rs에 정의되어 있고, 이 파일의 builders가 해당 DTO를 채웁니다.
use super::{SessionOverlayView, StartupOverlayView};

// 학습 주석: build_startup_overlay_view는 NativeTuiApp의 startup diagnostics 상태를 popup renderer용 DTO로 낮춥니다.
// shell_rendering/popup_frame.rs는 이 DTO의 sections를 받아 header/summary/checks/warnings/keys panel을 그립니다.
pub(crate) fn build_startup_overlay_view(app: &NativeTuiApp) -> StartupOverlayView {
    // 학습 주석: Ctrl+o는 parallel mode 상태에 따라 이동 대상이 달라집니다. 같은 key라도 recent sessions가 아니라
    // supersession board로 열릴 수 있으므로 footer copy를 app state와 동기화합니다.
    let ctrl_o_label = if app.parallel_mode_enabled() {
        "Ctrl+o: supersession board"
    } else {
        "Ctrl+o: recent sessions"
    };
    // 학습 주석: StartupOverlayView는 startup inspection의 모든 line group을 한 snapshot으로 담습니다.
    // renderer가 app을 다시 읽지 않아도 한 frame 안에서 일관된 상태를 그릴 수 있습니다.
    StartupOverlayView {
        // 학습 주석: header는 popup 제목과 보조 설명입니다. theme title helper로 다른 modal title과 스타일을 맞춥니다.
        header_lines: vec![
            // 학습 주석: "shell inspection"은 startup diagnostics가 shell을 떠나는 route가 아니라 overlay임을 나타냅니다.
            AkraTheme::title_line("Startup Diagnostics", " / shell inspection"),
            // 학습 주석: readiness 상태를 live shell 위에서 점검한다는 user-facing contract입니다.
            Line::from("Inspect readiness without leaving the live shell."),
        ],
        // 학습 주석: summary_lines는 workspace/session/runtime readiness를 짧은 overview로 줄인 영역입니다.
        summary_lines: build_startup_overlay_summary_lines(app),
        // 학습 주석: check_lines는 startup checks 각각의 상세 상태를 보여 주는 본문 영역입니다.
        check_lines: build_startup_check_lines(app),
        // 학습 주석: warning_lines는 startup 과정에서 사용자가 조치해야 할 문제나 degraded 상태를 별도 영역으로 둡니다.
        warning_lines: build_startup_warning_lines(app),
        // 학습 주석: key_lines는 popup frame footer에 들어가는 조작 안내입니다. close/rerun은 항상 있고,
        // Ctrl+o 설명은 위에서 계산한 parallel-mode aware label을 사용합니다.
        key_lines: vec![
            // 학습 주석: r은 startup diagnostics를 다시 실행하는 action이라 close shortcut과 같은 줄에 둡니다.
            AkraTheme::key_line("Esc/Ctrl+C: close    r: rerun checks"),
            // 학습 주석: ctrl_o_label은 current app mode에 맞는 navigation target을 사용자에게 알려 줍니다.
            AkraTheme::key_line(ctrl_o_label),
        ],
    }
}

// 학습 주석: build_session_overlay_view는 session browser projection을 popup renderer용 DTO로 감쌉니다.
// session_browser module이 content policy를 만들고, 이 builder는 header와 field mapping을 담당합니다.
pub(crate) fn build_session_overlay_view(app: &NativeTuiApp) -> SessionOverlayView {
    // 학습 주석: session overlay content는 좌측 list snapshot과 우측 detail lines로 나뉩니다.
    // helper가 app.session_overlay_ui_state, loaded sessions, selection을 함께 읽어 일관된 pair를 반환합니다.
    let (list_view, detail_lines) = build_session_overlay_content(app);

    // 학습 주석: SessionOverlayView는 renderer가 header/list/detail/warnings/keys panels를 그릴 때 필요한 모든
    // line collections를 갖는 presentation snapshot입니다.
    SessionOverlayView {
        // 학습 주석: header는 recent session browser가 live shell 위의 inspection surface임을 알려 줍니다.
        header_lines: vec![
            // 학습 주석: title helper를 사용해 startup/help/directions overlay와 같은 visual language를 공유합니다.
            AkraTheme::title_line("Recent Sessions", " / shell inspection"),
            // 학습 주석: session resume이 modal navigation이 아니라 현재 shell view 안에서 일어남을 설명합니다.
            Line::from("Resume a thread without leaving the shell view."),
        ],
        // 학습 주석: list_view는 sessions 목록, cursor, empty/loading message를 포함하는 shared overlay list DTO입니다.
        list_view,
        // 학습 주석: detail_lines는 선택된 session의 요약/metadata를 오른쪽 detail panel에 보여 줍니다.
        detail_lines,
        // 학습 주석: warning_lines는 session loading 실패나 stale state처럼 list/detail과 분리해 보여야 할 메시지입니다.
        warning_lines: build_session_warning_lines(app),
        // 학습 주석: key_lines는 current session browser state에 맞는 navigation/confirm shortcuts를 제공합니다.
        key_lines: build_session_key_lines(app),
    }
}
