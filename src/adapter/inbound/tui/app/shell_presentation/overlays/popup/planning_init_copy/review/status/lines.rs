use ratatui::text::Line;

use crate::adapter::inbound::tui::app::shell_presentation::overlays::popup::planning::copy::PlanningSimpleReviewCopy;

pub(super) fn build_simple_review_status_lines(
    copy: &PlanningSimpleReviewCopy,
) -> Vec<Line<'static>> {
    let mut status_lines = vec![
        Line::from(format!(
            "validation state: {}",
            if copy.validation_ok {
                "ok"
            } else {
                "needs attention"
            }
        )),
        Line::from(format!("turn budget: {}", copy.max_auto_turns_label)),
    ];
    if copy.is_turn_budget_editing {
        status_lines.push(Line::from(format!(
            "current state: editing turn budget / value: {} / controls: Enter saves, Esc/Ctrl+C cancels",
            copy.turn_budget_buffer
        )));
    } else {
        status_lines.push(Line::from(
            "next action: Enter or Ctrl+P promotes the staged simple scaffold.",
        ));
        status_lines.push(Line::from(
            "alternate action: Esc closes this review and leaves the staged draft on disk.",
        ));
        status_lines.push(Line::from(
            "advanced action: D opens detail-mode authoring without promoting the simple scaffold.",
        ));
    }
    if let Some(first_error) = copy.first_error.as_deref() {
        status_lines.push(Line::from(format!("first validation error: {first_error}")));
    }
    status_lines
}
