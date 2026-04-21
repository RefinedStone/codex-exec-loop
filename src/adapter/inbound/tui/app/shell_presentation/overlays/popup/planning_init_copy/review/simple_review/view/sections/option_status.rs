#[path = "option_status/option_lines.rs"]
mod option_lines;
#[path = "option_status/status_view.rs"]
mod status_view;

pub(super) use super::{PlanningSimpleReviewCopy, PlanningSimpleReviewStatusView};
use crate::adapter::inbound::tui::app::Line;
use option_lines::collect_simple_review_option_lines;
use status_view::collect_simple_review_status_view;

pub(super) struct PlanningSimpleReviewOptionStatusSections {
    pub(super) option_lines: Vec<Line<'static>>,
    pub(super) status_view: PlanningSimpleReviewStatusView,
}

pub(super) fn collect_simple_review_option_status_sections(
    copy: &PlanningSimpleReviewCopy,
) -> PlanningSimpleReviewOptionStatusSections {
    PlanningSimpleReviewOptionStatusSections {
        option_lines: collect_simple_review_option_lines(copy),
        status_view: collect_simple_review_status_view(copy),
    }
}
