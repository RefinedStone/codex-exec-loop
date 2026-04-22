use super::super::super::super::super::copy::PlanningSimpleReviewCopy;
use super::super::super::{header, options, status::{self, PlanningSimpleReviewStatusView}};
use crate::adapter::inbound::tui::app::Line;

pub(super) struct PlanningSimpleReviewOverlaySections {
    pub(super) header_lines: Vec<Line<'static>>,
    pub(super) summary_lines: Vec<Line<'static>>,
    pub(super) option_lines: Vec<Line<'static>>,
    pub(super) status_view: PlanningSimpleReviewStatusView,
}

pub(super) fn collect_simple_review_overlay_sections(
    copy: &PlanningSimpleReviewCopy,
) -> PlanningSimpleReviewOverlaySections {
    PlanningSimpleReviewOverlaySections {
        header_lines: header::build_simple_review_header_lines(),
        summary_lines: header::build_simple_review_summary_lines(),
        option_lines: options::build_simple_review_option_lines(copy),
        status_view: status::build_simple_review_status_view(copy),
    }
}
