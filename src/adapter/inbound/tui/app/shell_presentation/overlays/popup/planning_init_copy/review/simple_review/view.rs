use super::super::super::super::super::PlanningInitOverlayView;
use super::super::super::super::copy::PlanningSimpleReviewCopy;
use super::super::{header, options, status};

pub(super) fn build_simple_review_overlay_view(
    copy: PlanningSimpleReviewCopy,
) -> PlanningInitOverlayView {
    let status_view = status::build_simple_review_status_view(&copy);

    PlanningInitOverlayView {
        header_lines: header::build_simple_review_header_lines(),
        summary_lines: header::build_simple_review_summary_lines(),
        option_lines: options::build_simple_review_option_lines(&copy),
        status_lines: status_view.status_lines,
        key_lines: status_view.key_lines,
    }
}
