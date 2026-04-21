#[path = "contract_handoff/contract.rs"]
mod contract;

use super::super::super::super::super::super::PlanningInitOverlayView;
use super::super::super::super::super::super::copy::PlanningSimpleReviewCopy;
use super::super::assembly::assemble_simple_review_overlay_view;

pub(super) fn build_simple_review_overlay_view_from_copy(
    copy: PlanningSimpleReviewCopy,
) -> PlanningInitOverlayView {
    assemble_simple_review_overlay_view(contract::build_simple_review_assembly_contract_from_copy(
        &copy,
    ))
}
