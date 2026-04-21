#[path = "simple_review/view.rs"]
mod view;

use super::super::super::super::PlanningInitOverlayView;
use super::super::super::copy::PlanningSimpleReviewCopy;

pub(super) fn build_simple_review_overlay_view(
    copy: PlanningSimpleReviewCopy,
) -> PlanningInitOverlayView {
    view::build_simple_review_overlay_view(copy)
}
