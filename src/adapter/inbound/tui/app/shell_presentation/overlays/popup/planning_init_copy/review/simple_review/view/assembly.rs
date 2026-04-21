use crate::adapter::inbound::tui::app::shell_presentation::overlays::PlanningInitOverlayView;
#[path = "assembly/surface.rs"]
mod surface;
use super::assembly_contract::PlanningSimpleReviewAssemblyContract;
use surface::build_simple_review_overlay_view_from_contract;

pub(super) fn assemble_simple_review_overlay_view(
    contract: PlanningSimpleReviewAssemblyContract,
) -> PlanningInitOverlayView {
    build_simple_review_overlay_view_from_contract(contract)
}
