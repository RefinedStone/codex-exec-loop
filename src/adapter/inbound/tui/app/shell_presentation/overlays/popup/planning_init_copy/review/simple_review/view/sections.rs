#[path = "sections/header_summary.rs"]
mod header_summary;
#[path = "sections/option_status.rs"]
mod option_status;

pub(super) use super::super::super::super::super::copy::PlanningSimpleReviewCopy;
pub(super) use super::super::super::status::PlanningSimpleReviewStatusView;
use crate::adapter::inbound::tui::app::Line;
use header_summary::collect_simple_review_header_summary_sections;
use option_status::collect_simple_review_option_status_sections;

pub(super) struct PlanningSimpleReviewOverlaySections {
    pub(super) header_lines: Vec<Line<'static>>,
    pub(super) summary_lines: Vec<Line<'static>>,
    pub(super) option_lines: Vec<Line<'static>>,
    pub(super) status_view: PlanningSimpleReviewStatusView,
}

pub(super) fn collect_simple_review_overlay_sections(
    copy: &PlanningSimpleReviewCopy,
) -> PlanningSimpleReviewOverlaySections {
    let header_summary_sections = collect_simple_review_header_summary_sections();
    let option_status_sections = collect_simple_review_option_status_sections(copy);

    PlanningSimpleReviewOverlaySections {
        header_lines: header_summary_sections.header_lines,
        summary_lines: header_summary_sections.summary_lines,
        option_lines: option_status_sections.option_lines,
        status_view: option_status_sections.status_view,
    }
}
