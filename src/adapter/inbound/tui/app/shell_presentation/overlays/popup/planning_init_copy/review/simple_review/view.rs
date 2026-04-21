#[path = "view/assembly.rs"]
mod assembly;
#[path = "view/assembly_contract.rs"]
mod assembly_contract;
#[path = "view/chaining.rs"]
mod chaining;
#[path = "view/sections.rs"]
mod sections;

use super::super::super::super::super::PlanningInitOverlayView;
use super::super::super::super::copy::PlanningSimpleReviewCopy;
use assembly::assemble_simple_review_overlay_view;
use chaining::build_simple_review_assembly_contract_for_copy;

pub(super) fn build_simple_review_overlay_view(
    copy: PlanningSimpleReviewCopy,
) -> PlanningInitOverlayView {
    assemble_simple_review_overlay_view(build_simple_review_assembly_contract_for_copy(&copy))
}
