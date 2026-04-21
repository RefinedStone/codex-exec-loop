#[path = "view/assembly.rs"]
mod assembly;
#[path = "view/sections.rs"]
mod sections;

use super::super::super::super::super::PlanningInitOverlayView;
use super::super::super::super::copy::PlanningSimpleReviewCopy;
use assembly::assemble_simple_review_overlay_view;
use sections::collect_simple_review_overlay_sections;

pub(super) fn build_simple_review_overlay_view(
    copy: PlanningSimpleReviewCopy,
) -> PlanningInitOverlayView {
    assemble_simple_review_overlay_view(collect_simple_review_overlay_sections(&copy))
}
