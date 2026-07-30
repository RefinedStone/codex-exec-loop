use super::super::super::{AkraTheme, LANGUAGE_SELECTION_OPTIONS, Line, TuiLanguage};
use super::super::option_lines::overlay_option_line;
use super::LanguageSelectionOverlayView;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct LanguageSelectionFrameInput {
    pub(crate) current_language: TuiLanguage,
    pub(crate) selected_language_index: usize,
}

pub(crate) fn build_language_selection_overlay_view(
    input: LanguageSelectionFrameInput,
) -> LanguageSelectionOverlayView {
    let language_lines = LANGUAGE_SELECTION_OPTIONS
        .iter()
        .enumerate()
        .map(|(index, option)| {
            let detail =
                with_current_suffix(option.detail, input.current_language == option.language);
            overlay_option_line(
                &(index + 1).to_string(),
                option.label,
                &detail,
                input.selected_language_index == index,
                false,
            )
        })
        .collect();

    LanguageSelectionOverlayView {
        header_lines: vec![
            AkraTheme::title_line("Select Language", " / inline inspection"),
            Line::from("Choose the language used for TUI-generated system messages."),
        ],
        language_lines,
        status_lines: vec![
            Line::from(format!("current: {}", input.current_language.label())),
            Line::from("User prompts, task titles, and runtime payloads are kept as written."),
        ],
        key_lines: vec![
            AkraTheme::key_line("Enter/1-2: apply    j/k or Up/Down: move"),
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
