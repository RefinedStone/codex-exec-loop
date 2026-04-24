use crate::adapter::inbound::tui::app::{AkraTheme, Line};

pub(super) fn build_simple_review_key_lines(is_turn_budget_editing: bool) -> Vec<Line<'static>> {
    if is_turn_budget_editing {
        return vec![
            AkraTheme::key_line("next action: type the new turn budget directly."),
            AkraTheme::key_line(
                "controls: Enter saves  |  Esc/Ctrl+C cancels  |  Backspace deletes",
            ),
            AkraTheme::key_line("validation: use a whole number greater than 0, or type infinite."),
        ];
    }

    vec![
        AkraTheme::key_line("Enter or Ctrl+P promotes the staged scaffold."),
        AkraTheme::key_line(
            "D opens detail-mode authoring. Ctrl+L edits turn budget. Ctrl+E inspects or edits the draft.",
        ),
        AkraTheme::key_line("Esc/Ctrl+C closes this review."),
    ]
}
