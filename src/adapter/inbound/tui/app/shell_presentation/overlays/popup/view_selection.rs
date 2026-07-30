use super::super::super::{AkraTheme, ConversationViewMode, Line, VIEW_SELECTION_MODE_OPTIONS};
use super::super::option_lines::overlay_option_line;
use super::ViewSelectionOverlayView;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ViewSelectionFrameInput {
    pub(crate) current_mode: ConversationViewMode,
    pub(crate) selected_mode_index: usize,
}

pub(crate) fn build_view_selection_overlay_view(
    input: ViewSelectionFrameInput,
) -> ViewSelectionOverlayView {
    let mode_lines = VIEW_SELECTION_MODE_OPTIONS
        .iter()
        .enumerate()
        .map(|(index, option)| {
            let mode_label = option.mode.label();
            let detail = with_current_suffix(option.detail, input.current_mode == option.mode);
            overlay_option_line(
                &(index + 1).to_string(),
                mode_label,
                &detail,
                input.selected_mode_index == index,
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
            Line::from(format!("current: {}", input.current_mode.label())),
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
