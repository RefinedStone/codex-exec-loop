#[path = "header_summary/header_lines.rs"]
mod header_lines;
#[path = "header_summary/summary_lines.rs"]
mod summary_lines;

use crate::adapter::inbound::tui::app::Line;
use header_lines::collect_simple_review_header_lines;
use summary_lines::collect_simple_review_summary_lines;

pub(super) struct PlanningSimpleReviewHeaderSummarySections {
    pub(super) header_lines: Vec<Line<'static>>,
    pub(super) summary_lines: Vec<Line<'static>>,
}

pub(super) fn collect_simple_review_header_summary_sections()
-> PlanningSimpleReviewHeaderSummarySections {
    PlanningSimpleReviewHeaderSummarySections {
        header_lines: collect_simple_review_header_lines(),
        summary_lines: collect_simple_review_summary_lines(),
    }
}
