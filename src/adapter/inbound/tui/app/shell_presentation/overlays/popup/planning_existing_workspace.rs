use crate::application::service::planning::PlanningRuntimeProjection;

use super::super::super::super::NativeTuiApp;
use super::super::PlanningInitOverlayView;
use super::existing_workspace_inputs::build_existing_workspace_copy;
use super::init_copy::build_existing_workspace_overlay_view;

// Router entrypoint for the "planning already exists" branch of the init popup.
pub(super) fn build_existing_workspace_overlay_view_for_app(
    app: &NativeTuiApp,
) -> PlanningInitOverlayView {
    // Use the conversation-aware workspace selector so resumed sessions inspect their own planning directory.
    let workspace_directory = app.planning_workspace_directory();
    build_existing_workspace_overlay_view_for_projection(
        &workspace_directory,
        app.planning_runtime_projection_snapshot(),
    )
}

// Projection-level helper keeps copy assembly testable while the app owns source selection.
fn build_existing_workspace_overlay_view_for_projection(
    workspace_directory: &str,
    runtime_projection: PlanningRuntimeProjection,
) -> PlanningInitOverlayView {
    build_existing_workspace_overlay_view(build_existing_workspace_copy(
        workspace_directory,
        &runtime_projection,
    ))
}

#[cfg(test)]
mod tests {
    use super::build_existing_workspace_overlay_view_for_projection;
    use crate::adapter::inbound::tui::app::test_helpers::sample_planning_runtime_projection;

    #[test]
    // Existing-workspace copy uses the app-selected projection and keeps workspace selection separate.
    fn existing_workspace_view_uses_supplied_core_projection() {
        let runtime_projection = sample_planning_runtime_projection(
            "Planning Context",
            "queue summary from core snapshot",
        );

        let view = build_existing_workspace_overlay_view_for_projection(
            "/tmp/planning-workspace",
            runtime_projection,
        );

        assert!(
            view.option_lines
                .iter()
                .map(|line| line.to_string())
                .collect::<Vec<_>>()
                .join("\n")
                .contains("queue summary from core snapshot")
        );
    }
}
