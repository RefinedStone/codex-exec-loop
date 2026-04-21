use super::super::super::super::super::options;
use super::PlanningSimpleReviewCopy;
use crate::adapter::inbound::tui::app::Line;

pub(super) fn collect_simple_review_option_lines(
    copy: &PlanningSimpleReviewCopy,
) -> Vec<Line<'static>> {
    options::build_simple_review_option_lines(copy)
}
