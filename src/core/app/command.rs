use super::{ManualPromptPreparationIntent, TurnSubmissionRequest};
use crate::domain::conversation::ConversationTurnSteerRequest;
use crate::domain::planning::PostTurnRequest;
use crate::domain::recent_sessions::SessionRenameRequest;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AppCommand {
    /*
     * Noop gives the skeleton a real command path without changing product
     * behavior. Feature slices replace this with domain-specific commands such
     * as startup/session/conversation orchestration.
     */
    Noop,
    RunStartupChecks,
    LoadSessionCatalog {
        limit: usize,
        workspace_directory: String,
    },
    RenameSession(SessionRenameRequest),
    LoadConversation {
        thread_id: String,
        fallback_workspace_directory: String,
    },
    InvalidateConversationLoad,
    LoadParallelPeekConversation {
        thread_id: String,
    },
    LoadReviewCenter {
        workspace_directory: String,
        active_thread_id: Option<String>,
    },
    PrepareManualPrompt(Box<ManualPromptPreparationIntent>),
    CancelManualPromptPreparation,
    SubmitTurn(TurnSubmissionRequest),
    SteerTurn(ConversationTurnSteerRequest),
    EvaluatePostTurn(Box<PostTurnRequest>),
}
