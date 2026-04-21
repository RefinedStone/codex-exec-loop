use ratatui::text::Line;

pub(super) fn build_simple_review_non_editing_status_lines() -> Vec<Line<'static>> {
    vec![
        Line::from("next action: Enter or Ctrl+P promotes the staged simple scaffold."),
        Line::from("alternate action: Esc closes this review and leaves the staged draft on disk."),
        Line::from(
            "advanced action: D opens detail-mode authoring without promoting the simple scaffold.",
        ),
    ]
}
