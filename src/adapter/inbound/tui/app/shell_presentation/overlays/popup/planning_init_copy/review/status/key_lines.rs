use ratatui::text::Line;

pub(super) fn build_simple_review_key_lines(is_turn_budget_editing: bool) -> Vec<Line<'static>> {
    if is_turn_budget_editing {
        return vec![
            Line::from("next action: type the new turn budget directly."),
            Line::from("controls: Enter saves  |  Esc/Ctrl+C cancels  |  Backspace deletes"),
            Line::from("validation: use a whole number greater than 0, or type infinite."),
        ];
    }

    vec![
        Line::from("Enter or Ctrl+P promotes the staged scaffold."),
        Line::from(
            "D opens detail-mode authoring. Ctrl+L edits turn budget. Ctrl+E inspects or edits the draft.",
        ),
        Line::from("Esc/Ctrl+C closes this review."),
    ]
}
