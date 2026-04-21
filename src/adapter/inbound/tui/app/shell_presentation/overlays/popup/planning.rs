#[path = "planning_copy.rs"]
mod copy;
#[path = "planning_editor_surface.rs"]
mod editor_surface;
#[path = "planning_existing_workspace.rs"]
mod existing_workspace;
#[path = "planning_init_router.rs"]
mod init_router;
#[path = "planning_inputs.rs"]
mod inputs;
#[path = "planning_projection.rs"]
mod projection;
#[path = "planning_runtime.rs"]
mod runtime;
#[path = "planning_session.rs"]
mod session;

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
