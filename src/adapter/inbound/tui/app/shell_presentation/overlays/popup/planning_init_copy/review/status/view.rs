use super::{PlanningSimpleReviewCopy, PlanningSimpleReviewStatusView, key_lines, lines};

pub(super) fn build_simple_review_status_view(
    copy: &PlanningSimpleReviewCopy,
) -> PlanningSimpleReviewStatusView {
    PlanningSimpleReviewStatusView {
        status_lines: lines::build_simple_review_status_lines(copy),
        key_lines: key_lines::build_simple_review_key_lines(copy.is_turn_budget_editing),
    }
}
