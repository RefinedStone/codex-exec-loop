#[path = "sections/composition.rs"]
pub(super) mod composition;
#[path = "sections/header_summary.rs"]
mod header_summary;
#[path = "sections/option_status.rs"]
mod option_status;

pub(super) use super::super::super::super::super::copy::PlanningSimpleReviewCopy;
pub(super) use super::super::super::status::PlanningSimpleReviewStatusView;
use composition::{PlanningSimpleReviewOverlaySections, compose_simple_review_overlay_sections};
use header_summary::collect_simple_review_header_summary_sections;
use option_status::collect_simple_review_option_status_sections;

pub(super) fn collect_simple_review_overlay_sections(
    copy: &PlanningSimpleReviewCopy,
) -> PlanningSimpleReviewOverlaySections {
    let header_summary_sections = collect_simple_review_header_summary_sections();
    let option_status_sections = collect_simple_review_option_status_sections(copy);

    compose_simple_review_overlay_sections(header_summary_sections, option_status_sections)
}
