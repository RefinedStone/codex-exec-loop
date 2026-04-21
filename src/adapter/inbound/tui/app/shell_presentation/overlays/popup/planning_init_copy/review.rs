#[path = "review/header.rs"]
mod header;
#[path = "review/manual_editor.rs"]
mod manual_editor;
#[path = "review/options.rs"]
mod options;
#[path = "review/simple_review.rs"]
mod simple_review;
#[path = "review/status.rs"]
mod status;

use super::super::super::PlanningInitOverlayView;
use super::super::copy::PlanningSimpleReviewCopy;

pub(super) fn build_simple_review_overlay_view(
    copy: PlanningSimpleReviewCopy,
) -> PlanningInitOverlayView {
    simple_review::build_simple_review_overlay_view(copy)
}

pub(super) fn build_manual_editor_overlay_view() -> PlanningInitOverlayView {
    manual_editor::build_manual_editor_overlay_view()
}
