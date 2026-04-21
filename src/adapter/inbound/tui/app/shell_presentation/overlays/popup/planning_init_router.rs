use super::super::super::super::{NativeTuiApp, PlanningInitOverlayStep};
use super::super::PlanningInitOverlayView;
use super::existing_workspace::build_existing_workspace_overlay_view_for_app;
use super::init_copy::{
    build_detail_selection_overlay_view, build_manual_editor_overlay_view,
    build_mode_selection_overlay_view, build_simple_review_overlay_view,
};
use super::inputs::build_simple_review_copy;

pub(super) fn build_planning_init_overlay_view_for_app(
    app: &NativeTuiApp,
) -> PlanningInitOverlayView {
    match app.planning_init_overlay_ui_state.step() {
        PlanningInitOverlayStep::ExistingWorkspace => {
            build_existing_workspace_overlay_view_for_app(app)
        }
        PlanningInitOverlayStep::ModeSelection => {
            build_mode_selection_overlay_view(app.planning_init_overlay_ui_state.selected_mode())
        }
        PlanningInitOverlayStep::DetailSelection => build_detail_selection_overlay_view(
            app.planning_init_overlay_ui_state.selected_detail(),
        ),
        PlanningInitOverlayStep::SimpleReview => {
            build_simple_review_overlay_view(build_simple_review_copy(app))
        }
        PlanningInitOverlayStep::ManualEditor => build_manual_editor_overlay_view(),
    }
}
