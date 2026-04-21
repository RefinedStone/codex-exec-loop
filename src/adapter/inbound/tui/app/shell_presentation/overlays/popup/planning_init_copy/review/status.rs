#[path = "status/key_lines.rs"]
mod key_lines;
#[path = "status/lines.rs"]
mod lines;
#[path = "status/view.rs"]
mod view;

use super::super::super::super::super::super::Line;
use super::super::super::copy::PlanningSimpleReviewCopy;

pub(super) struct PlanningSimpleReviewStatusView {
    pub(super) status_lines: Vec<Line<'static>>,
    pub(super) key_lines: Vec<Line<'static>>,
}

pub(super) fn build_simple_review_status_view(
    copy: &PlanningSimpleReviewCopy,
) -> PlanningSimpleReviewStatusView {
    view::build_simple_review_status_view(copy)
}
