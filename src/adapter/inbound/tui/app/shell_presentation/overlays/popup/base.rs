use super::super::super::session_browser::{
    build_session_key_lines, build_session_overlay_content, build_session_warning_lines,
};
use super::super::super::{
    AkraTheme, Line, NativeTuiApp, build_startup_check_lines, build_startup_overlay_summary_lines,
    build_startup_warning_lines,
};
use super::{SessionOverlayView, StartupOverlayView};

pub(crate) fn build_startup_overlay_view(app: &NativeTuiApp) -> StartupOverlayView {
    let ctrl_o_label = if app.parallel_mode_enabled() {
        "Ctrl+o: supersession board"
    } else {
        "Ctrl+o: recent sessions"
    };
    StartupOverlayView {
        header_lines: vec![
            AkraTheme::title_line("Startup Diagnostics", " / shell inspection"),
            Line::from("Inspect readiness without leaving the live shell."),
        ],
        summary_lines: build_startup_overlay_summary_lines(app),
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
            AkraTheme::title_line("Recent Sessions", " / shell inspection"),
            Line::from("Resume a thread without leaving the shell view."),
        ],
        list_view,
        detail_lines,
        warning_lines: build_session_warning_lines(app),
        key_lines: build_session_key_lines(app),
    }
}
