#[path = "view/assembly.rs"]
mod assembly;

use super::super::super::super::super::PlanningInitOverlayView;
use super::super::super::super::copy::PlanningSimpleReviewCopy;
use super::super::{header, options, status};
use assembly::{PlanningSimpleReviewOverlaySections, assemble_simple_review_overlay_view};

fn collect_simple_review_overlay_sections(
    copy: &PlanningSimpleReviewCopy,
) -> PlanningSimpleReviewOverlaySections {
    PlanningSimpleReviewOverlaySections {
        header_lines: header::build_simple_review_header_lines(),
        summary_lines: header::build_simple_review_summary_lines(),
        option_lines: options::build_simple_review_option_lines(copy),
        status_view: status::build_simple_review_status_view(copy),
    }
}

pub(super) fn build_simple_review_overlay_view(
    copy: PlanningSimpleReviewCopy,
) -> PlanningInitOverlayView {
    assemble_simple_review_overlay_view(collect_simple_review_overlay_sections(&copy))
}
