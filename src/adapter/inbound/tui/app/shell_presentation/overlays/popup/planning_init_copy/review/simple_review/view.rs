#[path = "view/assembly.rs"]
mod assembly;
#[path = "view/assembly_contract.rs"]
mod assembly_contract;
#[path = "view/sections.rs"]
mod sections;

use super::super::super::super::super::PlanningInitOverlayView;
use super::super::super::super::copy::PlanningSimpleReviewCopy;
use assembly::assemble_simple_review_overlay_view;
use assembly_contract::build_simple_review_assembly_contract;
use sections::collect_simple_review_overlay_sections;

pub(super) fn build_simple_review_overlay_view(
    copy: PlanningSimpleReviewCopy,
) -> PlanningInitOverlayView {
    assemble_simple_review_overlay_view(build_simple_review_assembly_contract(
        collect_simple_review_overlay_sections(&copy),
    ))
}
