use ratatui::text::Line;

pub(super) fn build_simple_review_editing_status_lines(
    turn_budget_buffer: &str,
) -> Vec<Line<'static>> {
    vec![Line::from(format!(
        "current state: editing turn budget / value: {} / controls: Enter saves, Esc/Ctrl+C cancels",
        turn_budget_buffer
    ))]
}
