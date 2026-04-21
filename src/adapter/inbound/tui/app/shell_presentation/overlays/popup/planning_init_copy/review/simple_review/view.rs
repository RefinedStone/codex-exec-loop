#[path = "view/assembly.rs"]
mod assembly;
#[path = "view/assembly_contract.rs"]
mod assembly_contract;
#[path = "view/chaining.rs"]
mod chaining;
#[path = "view/entry.rs"]
mod entry;
#[path = "view/sections.rs"]
mod sections;

use super::super::super::super::super::PlanningInitOverlayView;
use super::super::super::super::copy::PlanningSimpleReviewCopy;

pub(super) fn build_simple_review_overlay_view(
    copy: PlanningSimpleReviewCopy,
) -> PlanningInitOverlayView {
    entry::build_simple_review_overlay_view(copy)
}
