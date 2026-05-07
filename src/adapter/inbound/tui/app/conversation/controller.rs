use super::super::planning::status_projection::build_resumed_session_status_text;
use super::super::{AutoFollowControlEvent, ConversationState, NativeTuiApp, StartupState};
use crate::application::service::planning::PlanningRuntimeSnapshot;

/*
conversation controller는 shell startup, editable draft, resumed thread 사이의 workspace boundary를 소유한다.
startup diagnostics가 shell cwd를 새로 알려 줄 수 있지만 resumed thread는 여전히 다른 workspace에 속할 수 있다.
그래서 planning runtime snapshot refresh를 render code의 ad hoc 계산으로 흩뜨리지 않고 이 controller 경계에 모아 둔다.
*/
impl NativeTuiApp {
    // local draft conversation만 shell workspace로 동기화한다. attached session은 기록된 cwd를 그대로 보존한다.
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

        // follow-up control도 draft workspace state를 들고 있으므로 planning refresh 전에 reducer를 통해 같은 cwd로 맞춘다.
        self.dispatch_auto_follow_controls(AutoFollowControlEvent::DraftWorkspaceSynced {
            workspace_directory: workspace_directory.to_string(),
        });
        self.refresh_ready_conversation_planning_runtime_snapshot();
    }

    // shell workspace는 startup diagnostics를 우선하고 없으면 process cwd를 쓴다. active thread workspace와 항상 같지는 않다.
    pub(crate) fn current_workspace_directory(&self) -> String {
        match &self.startup_state {
            StartupState::Ready(diagnostics) => diagnostics.workspace_path.clone(),
            _ => std::env::current_dir()
                .map(|path| path.display().to_string())
                // 특이한 test/runtime cwd failure에서도 early rendering은 살려 둔다. planning 쪽이 "." invalid 상태를 표시할 수 있다.
                .unwrap_or_else(|_| ".".to_string()),
        }
    }

    // planning workspace는 conversation이 있으면 그 thread/draft 기준을 따르고, 없으면 shell workspace로 fallback한다.
    pub(crate) fn planning_workspace_directory(&self) -> String {
        match &self.conversation_state {
            ConversationState::Ready(conversation) => {
                conversation.planning_workspace_directory().to_string()
            }
            _ => self.current_workspace_directory(),
        }
    }

    // planning runtime은 application service로 읽고, IO/parse failure는 invalid snapshot으로 접어 presentation에 전달한다.
    pub(crate) fn load_planning_runtime_snapshot(
        &self,
        workspace_directory: &str,
    ) -> PlanningRuntimeSnapshot {
        self.planning
            .runtime
            .load_runtime_snapshot_or_invalid(workspace_directory)
    }

    // active conversation이 현재 주장하는 workspace에 맞춰 planning status cache를 갱신한다.
    pub(crate) fn refresh_ready_conversation_planning_runtime_snapshot(&mut self) {
        // state replacement 중 self를 계속 빌리지 않도록 선택된 path를 먼저 소유한다.
        let workspace_directory = self.planning_workspace_directory();
        self.refresh_ready_conversation_planning_runtime_snapshot_for_workspace(
            &workspace_directory,
        );
    }

    // caller가 고른 workspace에 대해 Ready conversation이 가진 cached planning runtime snapshot을 교체한다.
    pub(crate) fn refresh_ready_conversation_planning_runtime_snapshot_for_workspace(
        &mut self,
        workspace_directory: &str,
    ) {
        let Some(mut conversation) = self.take_ready_conversation_state() else {
            return;
        };
        // conversation이 snapshot cache를 소유해야 render path가 filesystem-backed planning service를 직접 호출하지 않는다.
        conversation.replace_planning_runtime_snapshot(
            self.load_planning_runtime_snapshot(workspace_directory),
        );
        self.conversation_state = ConversationState::ready(conversation);
    }

    // saved thread를 연 직후 planning context를 status에 올려 workspace mismatch가 즉시 보이게 한다.
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
