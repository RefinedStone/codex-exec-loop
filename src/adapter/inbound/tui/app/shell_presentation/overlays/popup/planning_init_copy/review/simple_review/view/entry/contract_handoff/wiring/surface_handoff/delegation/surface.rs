#[path = "surface/boundary.rs"]
mod boundary;
#[path = "surface/delegation.rs"]
mod delegation;
use super::super::super::super::super::super::super::super::super::super::PlanningInitOverlayView;
use super::super::super::super::super::super::super::super::super::super::copy::PlanningSimpleReviewCopy;

pub(super) fn build_simple_review_overlay_view_from_copy(
    copy: PlanningSimpleReviewCopy,
) -> PlanningInitOverlayView {
    boundary::build_simple_review_overlay_view_from_copy(copy)
}
