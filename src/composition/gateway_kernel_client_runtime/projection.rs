use std::time::Instant;

use super::GatewayKernelClientRuntime;
use crate::adapter::inbound::kernel_protocol::{SessionsState, SessionsStatus, ThreadActivity};
use crate::core::app::{
    ActiveTurnPhase, ActiveTurnSnapshot, ApprovalAuthorityPhase, ApprovalAuthoritySnapshot,
    ConversationSnapshot, CorePromptOrigin, SessionCatalogReadySnapshot, SessionCatalogSnapshot,
    ThreadActivitySummary, TurnSubmissionCorrelation,
};
use crate::domain::conversation::{ConversationApprovalRequest, ConversationApprovalRequestKind};
use crate::domain::recent_sessions::{RecentSessions, SessionCatalog, SessionCatalogTier};
use crate::domain::session_summary::SessionSummary;

impl GatewayKernelClientRuntime {
    pub(super) fn refresh_conversation_activity(&mut self) {
        let authority = &mut self.snapshot.conversation_runtime;
        let current = match &self.snapshot.conversation {
            ConversationSnapshot::Ready(ready) => self
                .thread_activity
                .iter()
                .find(|activity| activity.thread_id == ready.thread_id)
                .map(|activity| (ready, activity)),
            _ => None,
        };
        authority.active_turn = current
            .filter(|(_, activity)| activity.running)
            .and_then(|(ready, activity)| {
                activity
                    .active_turn_id
                    .as_ref()
                    .map(|turn_id| (ready, turn_id))
            })
            .map(|(ready, turn_id)| {
                let prior = authority.active_turn.as_ref().filter(|active| {
                    active.turn_id.as_ref() == Some(turn_id)
                        && active.workspace_directory == ready.workspace_directory
                });
                ActiveTurnSnapshot {
                    correlation: TurnSubmissionCorrelation::new(0),
                    phase: ActiveTurnPhase::Running,
                    workspace_directory: ready.workspace_directory.clone(),
                    turn_id: Some(turn_id.clone()),
                    prompt_origin: CorePromptOrigin::Manual,
                    started_at: prior.map_or_else(Instant::now, |active| active.started_at),
                }
            });
        authority.approval = current
            .and_then(|(_, activity)| activity.approval.clone())
            .map(|request| ApprovalAuthoritySnapshot {
                request,
                decision: None,
                phase: ApprovalAuthorityPhase::Pending,
            });
    }
}

pub(super) fn session_catalog(state: SessionsState) -> SessionCatalogSnapshot {
    match state.status {
        SessionsStatus::Idle => SessionCatalogSnapshot::Idle,
        SessionsStatus::Loading => SessionCatalogSnapshot::Loading,
        SessionsStatus::Failed => SessionCatalogSnapshot::Failed {
            message: state
                .message
                .unwrap_or_else(|| "the kernel could not read the sessions".to_string()),
        },
        SessionsStatus::Unavailable => SessionCatalogSnapshot::Failed {
            message: "the kernel has no session list for this workspace".to_string(),
        },
        SessionsStatus::Ready => {
            let items = state
                .sessions
                .into_iter()
                .map(|entry| SessionSummary {
                    id: entry.thread_id,
                    name: Some(entry.title),
                    preview: String::new(),
                    cwd: entry.workspace,
                    source: "akra-kernel".to_string(),
                    model_provider: String::new(),
                    updated_at_epoch: entry.updated_at_epoch,
                    status_type: String::new(),
                    path: String::new(),
                    git_branch: None,
                })
                .collect();
            SessionCatalogSnapshot::Ready(SessionCatalogReadySnapshot::from_catalog(
                SessionCatalog::ready(
                    SessionCatalogTier::ProviderBackedCatalog,
                    RecentSessions {
                        items,
                        warnings: Vec::new(),
                        next_cursor: None,
                    },
                ),
            ))
        }
    }
}

pub(super) fn thread_activity(activity: ThreadActivity) -> ThreadActivitySummary {
    ThreadActivitySummary {
        thread_id: activity.thread_id,
        running: activity.running,
        active_turn_id: activity.active_turn_id,
        // The protocol leaves out the method and kind. The session list shows only that an approval waits.
        approval: activity
            .approval
            .map(|request| ConversationApprovalRequest {
                approval_id: request.approval_id,
                server_request_id: request.server_request_id,
                method: String::new(),
                kind: ConversationApprovalRequestKind::CommandExecution,
                summary: request.summary,
                details: request.details,
            }),
        auto_follow_active: activity.auto_follow_active,
    }
}
