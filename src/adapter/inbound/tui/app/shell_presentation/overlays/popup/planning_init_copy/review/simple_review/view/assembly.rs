use crate::adapter::inbound::tui::app::shell_presentation::overlays::PlanningInitOverlayView;
use super::sections::PlanningSimpleReviewOverlaySections;

pub(super) fn assemble_simple_review_overlay_view(
    sections: PlanningSimpleReviewOverlaySections,
) -> PlanningInitOverlayView {
    PlanningInitOverlayView {
        header_lines: sections.header_lines,
        summary_lines: sections.summary_lines,
        option_lines: sections.option_lines,
        status_lines: sections.status_view.status_lines,
        key_lines: sections.status_view.key_lines,
    }
}
