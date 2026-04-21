use super::super::super::super::header;
use crate::adapter::inbound::tui::app::Line;

pub(super) struct PlanningSimpleReviewHeaderSummarySections {
    pub(super) header_lines: Vec<Line<'static>>,
    pub(super) summary_lines: Vec<Line<'static>>,
}

pub(super) fn collect_simple_review_header_summary_sections()
-> PlanningSimpleReviewHeaderSummarySections {
    PlanningSimpleReviewHeaderSummarySections {
        header_lines: header::build_simple_review_header_lines(),
        summary_lines: header::build_simple_review_summary_lines(),
    }
}
