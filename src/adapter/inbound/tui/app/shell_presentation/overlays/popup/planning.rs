// planning popup surface는 init wizard, existing workspace, manual draft editor를 같은 renderer-facing
// overlay DTO로 낮추는 facade다. 세부 module은 copy extraction, state routing, projection, line assembly로
// 쪼개 두어 popup renderer가 planning domain/runtime shape를 직접 알지 않게 한다.
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

// popup/inline renderers는 이 entry만 호출한다. router가 app state를 보고 mode selection, simple review,
// manual editor handoff, existing workspace variant 중 하나를 고르므로 rendering layer에는 단일 init shape만 보인다.
pub(crate) fn build_planning_init_overlay_view(app: &NativeTuiApp) -> PlanningInitOverlayView {
    build_planning_init_overlay_view_for_app(app)
}

// draft editor는 staged planning files와 cursor projection이 모두 있어야 그릴 수 있다.
// `None`은 renderer가 editor surface 대신 아무 것도 그리지 않아야 하는 app-state mismatch를 뜻한다.
pub(crate) fn build_planning_draft_editor_overlay_view(
    app: &NativeTuiApp,
    // layout layer가 계산한 visible body height를 projection boundary에 넘겨 cursor 주변 slice를 고정한다.
    editor_height: u16,
) -> Option<PlanningDraftEditorOverlayView> {
    build_planning_draft_editor_overlay_view_for_app(app, editor_height)
}
