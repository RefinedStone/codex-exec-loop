use std::time::Instant;

use super::{ConversationState, NativeTuiApp, ShellChromeEvent};
use crate::core::app::{
    ActiveTurnPhase, ActiveTurnSnapshot, ApprovalAuthorityPhase, ApprovalAuthoritySnapshot,
    ConversationRuntimeSnapshot, CorePromptOrigin, TurnSubmissionCorrelation,
};

impl NativeTuiApp {
    pub(super) fn project_gateway_conversation_activity(
        &mut self,
        mut snapshot: ConversationRuntimeSnapshot,
    ) {
        let ConversationState::Ready(conversation) =
            &mut self.conversation.lifecycle.conversation_state
        else {
            return;
        };
        let current = self
            .conversation
            .thread_activity
            .iter()
            .find(|item| item.thread_id == conversation.thread_id);
        snapshot.active_turn = current
            .filter(|activity| activity.running)
            .and_then(|activity| activity.active_turn_id.as_ref())
            .map(|turn_id| {
                let prior = conversation
                    .runtime_snapshot()
                    .active_turn
                    .as_ref()
                    .filter(|active| active.turn_id.as_ref() == Some(turn_id));
                ActiveTurnSnapshot {
                    correlation: TurnSubmissionCorrelation::new(0),
                    phase: ActiveTurnPhase::Running,
                    workspace_directory: conversation.cwd.clone(),
                    turn_id: Some(turn_id.clone()),
                    prompt_origin: CorePromptOrigin::Manual,
                    started_at: prior.map_or_else(Instant::now, |active| active.started_at),
                }
            });
        snapshot.approval = current
            .and_then(|activity| activity.approval.clone())
            .map(|request| ApprovalAuthoritySnapshot {
                request,
                decision: None,
                phase: ApprovalAuthorityPhase::Pending,
            });
        let had_approval = conversation.pending_approval_request().is_some();
        let has_approval = snapshot.approval.is_some();
        conversation.apply_runtime_snapshot(snapshot);
        match (had_approval, has_approval) {
            (false, true) => self.dispatch_shell_chrome(ShellChromeEvent::ApprovalOverlayShown),
            (true, false) => self.dispatch_shell_chrome(ShellChromeEvent::ApprovalOverlayClosed),
            (true, true) | (false, false) => {}
        }
    }
}
