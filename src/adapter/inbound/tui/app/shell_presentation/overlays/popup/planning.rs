#[path = "planning_copy.rs"]
mod copy;
#[path = "planning_editor_copy.rs"]
mod editor_copy;
#[path = "planning_editor_inputs.rs"]
mod editor_inputs;
#[path = "planning_editor_surface.rs"]
mod editor_surface;
#[path = "planning_existing_workspace.rs"]
mod existing_workspace;
#[path = "planning_existing_workspace_inputs.rs"]
mod existing_workspace_inputs;
#[path = "planning_init_copy.rs"]
mod init_copy;
#[path = "planning_init_router.rs"]
mod init_router;
#[path = "planning_projection.rs"]
mod projection;
#[path = "planning_projection_lines.rs"]
mod projection_lines;
#[path = "planning_runtime.rs"]
mod runtime;
#[path = "planning_session.rs"]
mod session;
#[path = "planning_simple_review_inputs.rs"]
mod simple_review_inputs;

use super::super::super::NativeTuiApp;
use super::{PlanningDraftEditorOverlayView, PlanningInitOverlayView};
use editor_surface::build_planning_draft_editor_overlay_view_for_app;
use init_router::build_planning_init_overlay_view_for_app;

pub(crate) fn build_planning_init_overlay_view(app: &NativeTuiApp) -> PlanningInitOverlayView {
    build_planning_init_overlay_view_for_app(app)
}

pub(crate) fn build_planning_draft_editor_overlay_view(
    app: &NativeTuiApp,
    editor_height: u16,
) -> Option<PlanningDraftEditorOverlayView> {
    build_planning_draft_editor_overlay_view_for_app(app, editor_height)
}
