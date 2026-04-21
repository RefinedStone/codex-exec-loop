use super::super::super::super::super::super::super::PlanningInitOverlayView;
use super::super::super::assembly::assemble_simple_review_overlay_view;
use super::super::super::assembly_contract::PlanningSimpleReviewAssemblyContract;

pub(super) fn build_simple_review_overlay_view_from_contract(
    contract: PlanningSimpleReviewAssemblyContract,
) -> PlanningInitOverlayView {
    assemble_simple_review_overlay_view(contract)
}
