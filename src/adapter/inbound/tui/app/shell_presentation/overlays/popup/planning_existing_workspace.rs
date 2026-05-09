use crate::application::service::planning::{PlanningRuntimeSnapshot, PlanningServices};

use super::super::super::super::{ConversationState, NativeTuiApp};
use super::super::PlanningInitOverlayView;
use super::existing_workspace_inputs::build_existing_workspace_copy;
use super::init_copy::build_existing_workspace_overlay_view;

// Router entrypoint for the "planning already exists" branch of the init popup.
pub(super) fn build_existing_workspace_overlay_view_for_app(
    app: &NativeTuiApp,
) -> PlanningInitOverlayView {
    // Use the conversation-aware workspace selector so resumed sessions inspect their own planning directory.
    let workspace_directory = app.planning_workspace_directory();
    build_existing_workspace_overlay_view_for_state(
        &app.conversation_state,
        app.application.planning(),
        &workspace_directory,
    )
}

// State-level seam keeps snapshot source priority testable without constructing a whole NativeTuiApp.
fn build_existing_workspace_overlay_view_for_state(
    conversation_state: &ConversationState,
    planning: &PlanningServices,
    workspace_directory: &str,
) -> PlanningInitOverlayView {
    let snapshot =
        resolve_existing_workspace_snapshot(conversation_state, planning, workspace_directory);
    build_existing_workspace_overlay_view(build_existing_workspace_copy(
        workspace_directory,
        &snapshot,
    ))
}

// Snapshot source policy for existing-workspace inspection.
// Ready conversations win because their cached snapshot may include the latest in-memory post-turn refresh.
fn resolve_existing_workspace_snapshot(
    conversation_state: &ConversationState,
    planning: &PlanningServices,
    workspace_directory: &str,
) -> PlanningRuntimeSnapshot {
    match conversation_state {
        ConversationState::Ready(conversation) => conversation.planning_runtime_snapshot.clone(),
        // Without a conversation cache, fall back to the runtime loader and let failures become invalid snapshots.
        ConversationState::Loading | ConversationState::Failed(_) => planning
            .runtime
            .load_runtime_snapshot_or_invalid(workspace_directory),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::super::super::super::super::{ConversationState, ConversationViewModel};
    use super::resolve_existing_workspace_snapshot;
    use crate::adapter::inbound::tui::app::test_helpers::{
        sample_planning_runtime_snapshot, test_planning_services,
    };
    use crate::adapter::outbound::filesystem::FilesystemPlanningWorkspaceAdapter;

    #[test]
    // Ready state must preserve session-local planning state even when the shell workspace argument differs.
    fn ready_state_prefers_conversation_snapshot() {
        let mut conversation = ConversationViewModel::new_draft("/tmp/app".to_string());
        let snapshot = sample_planning_runtime_snapshot(
            "Planning Context",
            "queue summary from ready conversation",
        );
        conversation.replace_planning_runtime_snapshot(snapshot.clone());
        let planning = test_planning_services(Arc::new(FilesystemPlanningWorkspaceAdapter::new()));

        let resolved = resolve_existing_workspace_snapshot(
            &ConversationState::ready(conversation),
            &planning,
            "/tmp/other-workspace",
        );

        assert_eq!(resolved, snapshot);
    }

    #[test]
    // Loading state has no conversation cache, so the service loader provides the invalid/fallback snapshot.
    fn loading_state_uses_runtime_loader() {
        let planning = test_planning_services(Arc::new(FilesystemPlanningWorkspaceAdapter::new()));
        let workspace_directory = "/tmp/nonexistent-planning-workspace";

        let resolved = resolve_existing_workspace_snapshot(
            &ConversationState::Loading,
            &planning,
            workspace_directory,
        );

        assert_eq!(
            resolved,
            planning
                .runtime
                .load_runtime_snapshot_or_invalid(workspace_directory)
        );
    }
}
