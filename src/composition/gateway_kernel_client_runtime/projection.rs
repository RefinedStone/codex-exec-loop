use crate::adapter::inbound::kernel_protocol::{SessionsState, SessionsStatus, ThreadActivity};
use crate::core::app::{
    SessionCatalogReadySnapshot, SessionCatalogSnapshot, ThreadActivitySummary,
};
use crate::domain::conversation::{ConversationApprovalRequest, ConversationApprovalRequestKind};
use crate::domain::recent_sessions::{RecentSessions, SessionCatalog, SessionCatalogTier};
use crate::domain::session_summary::SessionSummary;

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
