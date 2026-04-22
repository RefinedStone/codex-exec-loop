#[path = "lines/editing.rs"]
mod editing;
#[path = "lines/first_error_tail.rs"]
mod first_error_tail;
#[path = "lines/non_editing.rs"]
mod non_editing;
#[path = "lines/prefix.rs"]
mod prefix;

use ratatui::text::Line;

use crate::adapter::inbound::tui::app::shell_presentation::overlays::popup::planning::copy::PlanningSimpleReviewCopy;

pub(super) fn build_simple_review_status_lines(
    copy: &PlanningSimpleReviewCopy,
) -> Vec<Line<'static>> {
    let mut status_lines = prefix::build_simple_review_status_prefix_lines(
        copy.validation_ok,
        &copy.max_auto_turns_label,
    );
    if copy.is_turn_budget_editing {
        status_lines.extend(editing::build_simple_review_editing_status_lines(
            copy.turn_budget_buffer.as_str(),
        ));
    } else {
        status_lines.extend(non_editing::build_simple_review_non_editing_status_lines());
    }
    status_lines.extend(first_error_tail::build_simple_review_first_error_tail_line(
        copy.first_error.as_deref(),
    ));
    status_lines
}
