use super::super::super::super::PlanningInitOverlayView;
use super::super::super::copy::PlanningSimpleReviewCopy;
use super::header::{build_simple_review_header_lines, build_simple_review_summary_lines};
use super::options::build_simple_review_option_lines;
use super::status::build_simple_review_status_view;

pub(super) fn build_simple_review_overlay_view(
    copy: PlanningSimpleReviewCopy,
) -> PlanningInitOverlayView {
    let status_view = build_simple_review_status_view(&copy);

    PlanningInitOverlayView {
        header_lines: build_simple_review_header_lines(),
        summary_lines: build_simple_review_summary_lines(),
        option_lines: build_simple_review_option_lines(&copy),
        status_lines: status_view.status_lines,
        key_lines: status_view.key_lines,
    }
}
