#[path = "contract_handoff/assembly.rs"]
mod assembly;
#[path = "contract_handoff/contract.rs"]
mod contract;
#[path = "contract_handoff/wiring.rs"]
mod wiring;

use super::super::super::super::super::super::PlanningInitOverlayView;
use super::super::super::super::super::super::copy::PlanningSimpleReviewCopy;

pub(super) fn build_simple_review_overlay_view_from_copy(
    copy: PlanningSimpleReviewCopy,
) -> PlanningInitOverlayView {
    wiring::build_simple_review_overlay_view_from_copy(copy)
}
