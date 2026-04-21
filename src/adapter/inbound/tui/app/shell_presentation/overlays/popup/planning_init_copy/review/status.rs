#[path = "status/key_lines.rs"]
mod key_lines;
#[path = "status/lines.rs"]
mod lines;

use super::super::super::super::super::super::Line;
use super::super::super::copy::PlanningSimpleReviewCopy;

pub(super) fn build_simple_review_status_lines(
    copy: &PlanningSimpleReviewCopy,
) -> Vec<Line<'static>> {
    lines::build_simple_review_status_lines(copy)
}

pub(super) fn build_simple_review_key_lines(is_turn_budget_editing: bool) -> Vec<Line<'static>> {
    key_lines::build_simple_review_key_lines(is_turn_budget_editing)
}
