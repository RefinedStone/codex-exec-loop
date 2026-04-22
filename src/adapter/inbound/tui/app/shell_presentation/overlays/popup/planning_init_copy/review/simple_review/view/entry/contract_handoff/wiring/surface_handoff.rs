#[path = "surface_handoff/delegation.rs"]
mod delegation;

use super::super::super::super::super::super::super::super::PlanningInitOverlayView;
use super::super::super::super::super::super::super::super::copy::PlanningSimpleReviewCopy;

pub(super) fn build_simple_review_overlay_view_from_copy(
    copy: PlanningSimpleReviewCopy,
) -> PlanningInitOverlayView {
    delegation::build_simple_review_overlay_view_from_copy(copy)
}
