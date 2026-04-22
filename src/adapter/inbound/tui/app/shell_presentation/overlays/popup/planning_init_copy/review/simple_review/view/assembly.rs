use super::super::super::status::PlanningSimpleReviewStatusView;
use crate::adapter::inbound::tui::app::Line;
use crate::adapter::inbound::tui::app::shell_presentation::overlays::PlanningInitOverlayView;

pub(super) struct PlanningSimpleReviewOverlaySections {
    pub(super) header_lines: Vec<Line<'static>>,
    pub(super) summary_lines: Vec<Line<'static>>,
    pub(super) option_lines: Vec<Line<'static>>,
    pub(super) status_view: PlanningSimpleReviewStatusView,
}

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
