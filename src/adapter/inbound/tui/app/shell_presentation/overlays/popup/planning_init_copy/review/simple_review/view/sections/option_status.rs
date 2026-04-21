use super::super::super::super::{options, status};
use super::{PlanningSimpleReviewCopy, PlanningSimpleReviewStatusView};
use crate::adapter::inbound::tui::app::Line;

pub(super) struct PlanningSimpleReviewOptionStatusSections {
    pub(super) option_lines: Vec<Line<'static>>,
    pub(super) status_view: PlanningSimpleReviewStatusView,
}

pub(super) fn collect_simple_review_option_status_sections(
    copy: &PlanningSimpleReviewCopy,
) -> PlanningSimpleReviewOptionStatusSections {
    PlanningSimpleReviewOptionStatusSections {
        option_lines: options::build_simple_review_option_lines(copy),
        status_view: status::build_simple_review_status_view(copy),
    }
}
