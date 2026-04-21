use super::super::super::super::super::super::Line;
use super::super::super::copy::PlanningSimpleReviewCopy;

pub(super) fn build_simple_review_option_lines(
    copy: &PlanningSimpleReviewCopy,
) -> Vec<Line<'static>> {
    vec![
        Line::from(format!("staged draft: {}", copy.draft_name)),
        Line::from(format!(
            "reviewed artifacts: {} staged planning files",
            copy.staged_file_count
        )),
        Line::from(
            "promote outcome: generic direction catalog, empty task ledger, and default queue-idle review prompt",
        ),
        Line::from(
            "advanced path: press D to branch into detail-mode authoring instead of promoting the simple scaffold",
        ),
    ]
}
