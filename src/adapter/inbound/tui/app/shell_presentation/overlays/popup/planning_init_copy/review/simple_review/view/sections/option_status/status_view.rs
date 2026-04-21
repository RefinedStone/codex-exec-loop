use super::super::super::super::super::status;
use super::{PlanningSimpleReviewCopy, PlanningSimpleReviewStatusView};

pub(super) fn collect_simple_review_status_view(
    copy: &PlanningSimpleReviewCopy,
) -> PlanningSimpleReviewStatusView {
    status::build_simple_review_status_view(copy)
}
