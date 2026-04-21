use super::super::super::super::super::super::Line;
use super::super::super::copy::PlanningSimpleReviewCopy;

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
