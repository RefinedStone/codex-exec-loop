use super::super::{ConversationState, FollowupControlEvent, NativeTuiApp, StartupState};
use crate::application::service::planning::PlanningRuntimeSnapshot;

const RESUMED_SESSION_DETAIL_LIMIT: usize = 96;

impl NativeTuiApp {
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

        self.dispatch_followup_controls(FollowupControlEvent::DraftWorkspaceSynced {
            workspace_directory: workspace_directory.to_string(),
        });
        self.refresh_ready_conversation_planning_runtime_snapshot();
    }

    pub(crate) fn current_workspace_directory(&self) -> String {
        match &self.startup_state {
            StartupState::Ready(diagnostics) => diagnostics.workspace_path.clone(),
            _ => std::env::current_dir()
                .map(|path| path.display().to_string())
                .unwrap_or_else(|_| ".".to_string()),
        }
    }

    pub(crate) fn planning_workspace_directory(&self) -> String {
        match &self.conversation_state {
            ConversationState::Ready(conversation) => {
                conversation.planning_workspace_directory().to_string()
            }
            _ => self.current_workspace_directory(),
        }
    }

    pub(crate) fn load_planning_runtime_snapshot(
        &self,
        workspace_directory: &str,
    ) -> PlanningRuntimeSnapshot {
        self.planning
            .runtime
            .load_runtime_snapshot_or_invalid(workspace_directory)
    }

    pub(crate) fn refresh_ready_conversation_planning_runtime_snapshot(&mut self) {
        let workspace_directory = self.planning_workspace_directory();
        self.refresh_ready_conversation_planning_runtime_snapshot_for_workspace(
            &workspace_directory,
        );
    }

    pub(crate) fn refresh_ready_conversation_planning_runtime_snapshot_for_workspace(
        &mut self,
        workspace_directory: &str,
    ) {
        let Some(mut conversation) = self.take_ready_conversation_state() else {
            return;
        };
        conversation.replace_planning_runtime_snapshot(
            self.load_planning_runtime_snapshot(workspace_directory),
        );
        self.conversation_state = ConversationState::ready(conversation);
    }

    pub(crate) fn surface_resumed_session_planning_context(&mut self) {
        let Some(mut conversation) = self.take_ready_conversation_state() else {
            return;
        };
        conversation.set_status_with_warnings(resumed_session_status_text(
            &conversation.planning_runtime_snapshot,
        ));
        self.conversation_state = ConversationState::ready(conversation);
    }
}

fn resumed_session_status_text(snapshot: &PlanningRuntimeSnapshot) -> String {
    let mut status_text = format!(
        "thread loaded / planning status: {}",
        snapshot.preview_status_label()
    );

    if let Some(queue_summary) =
        snapshot.compact_queue_framing_summary(RESUMED_SESSION_DETAIL_LIMIT)
    {
        status_text.push_str(" / queue summary: ");
        status_text.push_str(&queue_summary);
    } else if let Some(detail) = snapshot.preview_detail_with_limit(RESUMED_SESSION_DETAIL_LIMIT) {
        status_text.push_str(" / planning detail: ");
        status_text.push_str(&detail);
    }

    status_text
}
