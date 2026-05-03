use super::super::super::session_browser::{
    build_session_key_lines, build_session_overlay_content, build_session_warning_lines,
};
use super::super::super::{
    AkraTheme, Line, NativeTuiApp, build_startup_check_lines, build_startup_overlay_summary_lines,
    build_startup_warning_lines,
};
use super::{SessionOverlayView, StartupOverlayView};

// Startup popup은 app startup diagnostics를 renderer-facing section snapshot으로 낮춘다.
// renderer는 이 DTO만 보고 header/summary/checks/warnings/footer panel을 그리며 app state를 다시 읽지 않는다.
pub(crate) fn build_startup_overlay_view(app: &NativeTuiApp) -> StartupOverlayView {
    // 같은 Ctrl+o shortcut이라도 parallel mode에서는 recent sessions가 아니라 supersession board로 향한다.
    // footer copy를 여기서 계산해 rendering layer가 navigation policy를 추측하지 않게 한다.
    let ctrl_o_label = if app.parallel_mode_enabled() {
        "Ctrl+o: supersession board"
    } else {
        "Ctrl+o: recent sessions"
    };

    StartupOverlayView {
        // header는 startup diagnostics가 live shell 위의 inspection surface라는 위치를 고정한다.
        header_lines: vec![
            AkraTheme::title_line("Startup Diagnostics", " / shell inspection"),
            Line::from("Inspect readiness without leaving the live shell."),
        ],
        // summary/check/warning groups는 startup projection layer가 이미 우선순위를 정한 read model이다.
        summary_lines: build_startup_overlay_summary_lines(app),
        check_lines: build_startup_check_lines(app),
        warning_lines: build_startup_warning_lines(app),
        key_lines: vec![
            AkraTheme::key_line("Esc/Ctrl+C: close    r: rerun checks"),
            AkraTheme::key_line(ctrl_o_label),
        ],
    }
}

// Session popup은 session_browser의 list/detail projection을 modal chrome에 싣는 얇은 assembly boundary다.
// list selection policy와 warning/key copy는 session_browser가 소유하고, 이 builder는 renderer field mapping만 맡는다.
pub(crate) fn build_session_overlay_view(app: &NativeTuiApp) -> SessionOverlayView {
    // list와 detail을 한 helper에서 같이 뽑아 cursor/selection mismatch가 한 frame 안에 섞이지 않게 한다.
    let (list_view, detail_lines) = build_session_overlay_content(app);

    SessionOverlayView {
        // header copy는 session resume이 별도 route가 아니라 현재 shell 위 inspection임을 유지한다.
        header_lines: vec![
            AkraTheme::title_line("Recent Sessions", " / shell inspection"),
            Line::from("Resume a thread without leaving the shell view."),
        ],
        list_view,
        detail_lines,
        warning_lines: build_session_warning_lines(app),
        key_lines: build_session_key_lines(app),
    }
}
