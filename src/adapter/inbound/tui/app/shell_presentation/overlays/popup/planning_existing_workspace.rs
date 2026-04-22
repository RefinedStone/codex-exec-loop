use crate::application::service::planning::{PlanningRuntimeSnapshot, PlanningServices};

use super::super::super::super::{ConversationState, NativeTuiApp};
use super::super::PlanningInitOverlayView;
use super::existing_workspace_inputs::build_existing_workspace_copy;
use super::init_copy::build_existing_workspace_overlay_view;

pub(super) fn build_existing_workspace_overlay_view_for_app(
    app: &NativeTuiApp,
) -> PlanningInitOverlayView {
    let workspace_directory = app.planning_workspace_directory();
    build_existing_workspace_overlay_view_for_state(
        &app.conversation_state,
        &app.planning,
        &workspace_directory,
    )
}

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

fn resolve_existing_workspace_snapshot(
    conversation_state: &ConversationState,
    planning: &PlanningServices,
    workspace_directory: &str,
) -> PlanningRuntimeSnapshot {
    match conversation_state {
        ConversationState::Ready(conversation) => conversation.planning_runtime_snapshot.clone(),
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
    use crate::adapter::inbound::tui::app::test_helpers::sample_planning_runtime_snapshot;
    use crate::adapter::outbound::filesystem::FilesystemPlanningWorkspaceAdapter;
    use crate::application::service::planning::PlanningServices;

    #[test]
    fn ready_state_prefers_conversation_snapshot() {
        let mut conversation = ConversationViewModel::new_draft("/tmp/app".to_string());
        let snapshot = sample_planning_runtime_snapshot(
            "Planning Context",
            "queue summary from ready conversation",
        );
        conversation.replace_planning_runtime_snapshot(snapshot.clone());
        let planning = PlanningServices::from_workspace_port(Arc::new(
            FilesystemPlanningWorkspaceAdapter::new(),
        ));

        let resolved = resolve_existing_workspace_snapshot(
            &ConversationState::ready(conversation),
            &planning,
            "/tmp/other-workspace",
        );

        assert_eq!(resolved, snapshot);
    }

    #[test]
    fn loading_state_uses_runtime_loader() {
        let planning = PlanningServices::from_workspace_port(Arc::new(
            FilesystemPlanningWorkspaceAdapter::new(),
        ));
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
