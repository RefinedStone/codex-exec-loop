use super::super::super::super::super::super::super::super::PlanningInitOverlayView;
use super::super::super::super::super::super::super::super::copy::PlanningSimpleReviewCopy;
use super::surface;

pub(super) fn build_simple_review_overlay_view_from_copy(
    copy: PlanningSimpleReviewCopy,
) -> PlanningInitOverlayView {
    surface::build_simple_review_overlay_view_from_copy(copy)
}
