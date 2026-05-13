use super::super::super::{AkraTheme, Line, NativeTuiApp, VIEW_SELECTION_MODE_OPTIONS};
use super::super::option_lines::overlay_option_line;
use super::ViewSelectionOverlayView;

pub(crate) fn build_view_selection_overlay_view(app: &NativeTuiApp) -> ViewSelectionOverlayView {
    let state = &app.view_selection_overlay_ui_state;
    let mode_lines = VIEW_SELECTION_MODE_OPTIONS
        .iter()
        .enumerate()
        .map(|(index, option)| {
            let mode_label = option.mode.label();
            let detail =
                with_current_suffix(option.detail, app.conversation_view_mode == option.mode);
            overlay_option_line(
                &(index + 1).to_string(),
                mode_label,
                &detail,
                state.selected_mode_index() == index,
                false,
            )
        })
        .collect();

    ViewSelectionOverlayView {
        header_lines: vec![
            AkraTheme::title_line("Select Conversation View", " / inline inspection"),
            Line::from("Choose how much tool and status transcript detail remains visible."),
        ],
        mode_lines,
        status_lines: vec![
            Line::from(format!("current: {}", app.conversation_view_mode.label())),
            Line::from("Codex and Codex Commentary stay visible in every view."),
        ],
        key_lines: vec![
            AkraTheme::key_line("Enter/1-3: apply    j/k or Up/Down: move"),
            AkraTheme::key_line("Esc/Ctrl+C: close"),
        ],
    }
}

fn with_current_suffix(detail: &str, current: bool) -> String {
    if current {
        format!("{detail}  (current)")
    } else {
        detail.to_string()
    }
}
