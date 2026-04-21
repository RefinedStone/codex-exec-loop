#[path = "review/header.rs"]
mod header;
#[path = "review/manual_editor.rs"]
mod manual_editor;
#[path = "review/options.rs"]
mod options;
#[path = "review/status.rs"]
mod status;

use super::super::super::PlanningInitOverlayView;
use super::super::copy::PlanningSimpleReviewCopy;
use header::{build_simple_review_header_lines, build_simple_review_summary_lines};
use options::build_simple_review_option_lines;
use status::{build_simple_review_key_lines, build_simple_review_status_lines};

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

pub(super) fn build_manual_editor_overlay_view() -> PlanningInitOverlayView {
    manual_editor::build_manual_editor_overlay_view()
}
