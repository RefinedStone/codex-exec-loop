use super::super::planning::status_projection::build_resumed_session_status_text;
use super::super::{ConversationState, FollowupControlEvent, NativeTuiApp, StartupState};
use crate::application::service::planning::PlanningRuntimeSnapshot;

/*
Conversation controller owns the workspace boundary between shell startup, editable drafts, and resumed threads.
The shell can learn a new cwd from startup diagnostics while a resumed thread may still belong to a different
workspace, so planning snapshots are refreshed here instead of being recomputed ad hoc in render code.
*/
impl NativeTuiApp {
    // Sync only local draft conversations to the shell workspace; attached sessions keep their own recorded cwd.
    pub(crate) fn sync_draft_shell_workspace(&mut self, workspace_directory: &str) {
        let should_refresh_draft = matches!(
            &self.conversation_state,
            ConversationState::Ready(conversation)
                if !conversation.has_active_thread()
                    && conversation.draft_workspace_directory() != workspace_directory
        );
        if !should_refresh_draft {
            return;
        }

        // Follow-up controls own draft workspace state, so route the change through their reducer before refreshing planning.
        self.dispatch_followup_controls(FollowupControlEvent::DraftWorkspaceSynced {
            workspace_directory: workspace_directory.to_string(),
        });
        self.refresh_ready_conversation_planning_runtime_snapshot();
    }

    // Shell workspace is startup diagnostics first, process cwd second; it is not necessarily the active thread workspace.
    pub(crate) fn current_workspace_directory(&self) -> String {
        match &self.startup_state {
            StartupState::Ready(diagnostics) => diagnostics.workspace_path.clone(),
            _ => std::env::current_dir()
                .map(|path| path.display().to_string())
                // Keep early rendering alive even in unusual test/runtime cwd failures; planning can surface "." as invalid.
                .unwrap_or_else(|_| ".".to_string()),
        }
    }

    // Planning workspace follows the conversation when one exists, otherwise falls back to the shell workspace.
    pub(crate) fn planning_workspace_directory(&self) -> String {
        match &self.conversation_state {
            ConversationState::Ready(conversation) => {
                conversation.planning_workspace_directory().to_string()
            }
            _ => self.current_workspace_directory(),
        }
    }

    // Read planning runtime through the application service and fold IO/parse failures into an invalid snapshot.
    pub(crate) fn load_planning_runtime_snapshot(
        &self,
        workspace_directory: &str,
    ) -> PlanningRuntimeSnapshot {
        self.planning
            .runtime
            .load_runtime_snapshot_or_invalid(workspace_directory)
    }

    // Refresh planning status for whatever workspace the active conversation currently claims.
    pub(crate) fn refresh_ready_conversation_planning_runtime_snapshot(&mut self) {
        // Own the selected path before mutable refresh to avoid borrowing self across the state replacement.
        let workspace_directory = self.planning_workspace_directory();
        self.refresh_ready_conversation_planning_runtime_snapshot_for_workspace(
            &workspace_directory,
        );
    }

    // Replace the Ready conversation's cached planning snapshot for a caller-selected workspace.
    pub(crate) fn refresh_ready_conversation_planning_runtime_snapshot_for_workspace(
        &mut self,
        workspace_directory: &str,
    ) {
        let Some(mut conversation) = self.take_ready_conversation_state() else {
            return;
        };
        // The conversation owns the snapshot cache so render paths do not hit filesystem-backed planning services.
        conversation.replace_planning_runtime_snapshot(
            self.load_planning_runtime_snapshot(workspace_directory),
        );
        self.conversation_state = ConversationState::ready(conversation);
    }

    // After opening a saved thread, surface its planning context so workspace mismatches are visible immediately.
    pub(crate) fn surface_resumed_session_planning_context(&mut self) {
        let Some(mut conversation) = self.take_ready_conversation_state() else {
            return;
        };
        conversation.set_status_with_warnings(build_resumed_session_status_text(
            &conversation.planning_runtime_snapshot,
        ));
        self.conversation_state = ConversationState::ready(conversation);
    }
}
