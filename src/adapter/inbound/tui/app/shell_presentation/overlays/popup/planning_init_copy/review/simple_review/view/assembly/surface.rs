use super::super::super::super::super::super::super::PlanningInitOverlayView;
use super::super::assembly_contract::PlanningSimpleReviewAssemblyContract;

pub(super) fn build_simple_review_overlay_view_from_contract(
    contract: PlanningSimpleReviewAssemblyContract,
) -> PlanningInitOverlayView {
    PlanningInitOverlayView {
        header_lines: contract.header_lines,
        summary_lines: contract.summary_lines,
        option_lines: contract.option_lines,
        status_lines: contract.status_lines,
        key_lines: contract.key_lines,
    }
}
