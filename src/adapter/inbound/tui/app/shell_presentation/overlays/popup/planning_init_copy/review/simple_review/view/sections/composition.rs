use super::PlanningSimpleReviewStatusView;
use super::header_summary::PlanningSimpleReviewHeaderSummarySections;
use super::option_status::PlanningSimpleReviewOptionStatusSections;
use crate::adapter::inbound::tui::app::Line;

pub(in super::super) struct PlanningSimpleReviewOverlaySections {
    pub(in super::super) header_lines: Vec<Line<'static>>,
    pub(in super::super) summary_lines: Vec<Line<'static>>,
    pub(in super::super) option_lines: Vec<Line<'static>>,
    pub(in super::super) status_view: PlanningSimpleReviewStatusView,
}

pub(super) fn compose_simple_review_overlay_sections(
    header_summary_sections: PlanningSimpleReviewHeaderSummarySections,
    option_status_sections: PlanningSimpleReviewOptionStatusSections,
) -> PlanningSimpleReviewOverlaySections {
    PlanningSimpleReviewOverlaySections {
        header_lines: header_summary_sections.header_lines,
        summary_lines: header_summary_sections.summary_lines,
        option_lines: option_status_sections.option_lines,
        status_view: option_status_sections.status_view,
    }
}
