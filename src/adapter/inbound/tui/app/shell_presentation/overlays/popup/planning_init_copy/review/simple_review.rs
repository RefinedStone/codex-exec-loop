use super::super::super::super::PlanningInitOverlayView;
use super::super::super::copy::PlanningSimpleReviewCopy;
use super::header::{build_simple_review_header_lines, build_simple_review_summary_lines};
use super::options::build_simple_review_option_lines;
use super::status::{build_simple_review_key_lines, build_simple_review_status_lines};

pub(super) fn build_simple_review_overlay_view(
    copy: PlanningSimpleReviewCopy,
) -> PlanningInitOverlayView {
    PlanningInitOverlayView {
        header_lines: build_simple_review_header_lines(),
        summary_lines: build_simple_review_summary_lines(),
        option_lines: build_simple_review_option_lines(&copy),
        status_lines: build_simple_review_status_lines(&copy),
        key_lines: build_simple_review_key_lines(copy.is_turn_budget_editing),
    }
}
