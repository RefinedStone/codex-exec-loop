#[path = "review/manual_editor.rs"]
mod manual_editor;
#[path = "review/status.rs"]
mod status;

use super::super::super::super::super::Line;
use super::super::super::PlanningInitOverlayView;
use super::super::copy::{PlanningSimpleReviewCopy, planning_setup_title_line};
use status::{build_simple_review_key_lines, build_simple_review_status_lines};

pub(super) fn build_simple_review_overlay_view(
    copy: PlanningSimpleReviewCopy,
) -> PlanningInitOverlayView {
    PlanningInitOverlayView {
        header_lines: vec![
            planning_setup_title_line(" / operator inspection"),
            Line::from(
                "Simple mode review: promote the lightest planning baseline before you invest in richer authoring.",
            ),
        ],
        summary_lines: vec![
            Line::from(
                "After promote, planning starts with one generic direction and no active queue task yet.",
            ),
            Line::from(
                "The default queue-idle review prompt is already staged so the first reply can justify follow-up work when needed.",
            ),
            Line::from("No active planning files change until you explicitly promote this review."),
        ],
        option_lines: vec![
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
        ],
        status_lines: build_simple_review_status_lines(&copy),
        key_lines: build_simple_review_key_lines(copy.is_turn_budget_editing),
    }
}

pub(super) fn build_manual_editor_overlay_view() -> PlanningInitOverlayView {
    manual_editor::build_manual_editor_overlay_view()
}
