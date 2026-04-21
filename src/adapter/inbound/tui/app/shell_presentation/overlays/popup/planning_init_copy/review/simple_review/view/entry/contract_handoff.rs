use super::super::super::super::super::super::PlanningInitOverlayView;
use super::super::super::super::super::super::copy::PlanningSimpleReviewCopy;
use super::super::assembly::assemble_simple_review_overlay_view;
use super::super::chaining::build_simple_review_assembly_contract_for_copy;

pub(super) fn build_simple_review_overlay_view_from_copy(
    copy: PlanningSimpleReviewCopy,
) -> PlanningInitOverlayView {
    assemble_simple_review_overlay_view(build_simple_review_assembly_contract_for_copy(&copy))
}
