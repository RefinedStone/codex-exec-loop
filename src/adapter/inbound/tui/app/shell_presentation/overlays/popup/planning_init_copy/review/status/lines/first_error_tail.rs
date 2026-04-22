use ratatui::text::Line;

pub(super) fn build_simple_review_first_error_tail_line(
    first_error: Option<&str>,
) -> Option<Line<'static>> {
    first_error.map(|message| Line::from(format!("first validation error: {message}")))
}
