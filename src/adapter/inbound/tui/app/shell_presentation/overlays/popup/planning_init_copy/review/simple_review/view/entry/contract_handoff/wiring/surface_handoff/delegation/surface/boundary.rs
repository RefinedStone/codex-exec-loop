use super::super::super::super::super::super::super::super::super::super::super::PlanningInitOverlayView;
use super::super::super::super::super::super::super::super::super::super::super::copy::PlanningSimpleReviewCopy;
use super::delegation;

pub(super) fn build_simple_review_overlay_view_from_copy(
    copy: PlanningSimpleReviewCopy,
) -> PlanningInitOverlayView {
    delegation::build_simple_review_overlay_view_from_copy(copy)
}
