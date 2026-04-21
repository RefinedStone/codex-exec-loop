#[path = "planning_init_copy/existing_workspace.rs"]
mod existing_workspace;
#[path = "planning_init_copy/review.rs"]
mod review;
#[path = "planning_init_copy/selection.rs"]
mod selection;

use super::super::super::super::{PlanningInitDetailSelection, PlanningInitModeSelection};
use super::super::PlanningInitOverlayView;
use super::copy::{PlanningExistingWorkspaceCopy, PlanningSimpleReviewCopy};

pub(super) fn build_existing_workspace_overlay_view(
    copy: PlanningExistingWorkspaceCopy,
) -> PlanningInitOverlayView {
    existing_workspace::build_existing_workspace_overlay_view(copy)
}

pub(super) fn build_mode_selection_overlay_view(
    selected_mode: PlanningInitModeSelection,
) -> PlanningInitOverlayView {
    selection::build_mode_selection_overlay_view(selected_mode)
}

pub(super) fn build_detail_selection_overlay_view(
    selected_detail: PlanningInitDetailSelection,
) -> PlanningInitOverlayView {
    selection::build_detail_selection_overlay_view(selected_detail)
}

pub(super) fn build_simple_review_overlay_view(
    copy: PlanningSimpleReviewCopy,
) -> PlanningInitOverlayView {
    review::build_simple_review_overlay_view(copy)
}

pub(super) fn build_manual_editor_overlay_view() -> PlanningInitOverlayView {
    review::build_manual_editor_overlay_view()
}
