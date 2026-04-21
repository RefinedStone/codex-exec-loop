use crate::adapter::inbound::tui::app::shell_presentation::overlays::PlanningInitOverlayView;
use super::assembly_contract::PlanningSimpleReviewAssemblyContract;

pub(super) fn assemble_simple_review_overlay_view(
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
