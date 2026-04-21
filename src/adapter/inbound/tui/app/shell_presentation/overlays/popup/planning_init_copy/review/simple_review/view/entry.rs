#[path = "entry/contract_handoff.rs"]
mod contract_handoff;

use super::super::super::super::super::PlanningInitOverlayView;
use super::super::super::super::super::copy::PlanningSimpleReviewCopy;

pub(super) fn build_simple_review_overlay_view(
    copy: PlanningSimpleReviewCopy,
) -> PlanningInitOverlayView {
    contract_handoff::build_simple_review_overlay_view_from_copy(copy)
}
