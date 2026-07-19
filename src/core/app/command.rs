use super::{
    GithubReviewPollingSetupRequest, ManualPromptPreparationIntent, PlanningEditorSessionIdentity,
    PlanningWorkspaceResetIntent, QueueMutationIntent, TurnSubmissionRequest,
};
use crate::domain::conversation::{ConversationApprovalDecision, ConversationTurnSteerRequest};
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
    RunStartupChecks {
        workspace_directory: String,
    },
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
    LoadQueueAuthority {
        workspace_directory: String,
        active_thread_id: Option<String>,
    },
    LoadDirectionsMaintenance {
        workspace_directory: String,
    },
    RefreshPlanningRuntime {
        workspace_directory: String,
    },
    ResetPlanningWorkspace(PlanningWorkspaceResetIntent),
    StageSimplePlanningDraft {
        workspace_directory: String,
    },
    LoadSimplePlanningEditor {
        workspace_directory: String,
        draft_name: String,
        source_session: PlanningEditorSessionIdentity,
    },
    PromoteSimplePlanningDraft {
        workspace_directory: String,
        draft_name: String,
        source_session: PlanningEditorSessionIdentity,
    },
    SubmitQueueMutation(Box<QueueMutationIntent>),
    PrepareManualPrompt(Box<ManualPromptPreparationIntent>),
    CancelManualPromptPreparation,
    SubmitTurn(TurnSubmissionRequest),
    RequestStopAllSessions,
    SteerTurn(ConversationTurnSteerRequest),
    SubmitApprovalDecision {
        approval_id: String,
        decision: ConversationApprovalDecision,
    },
    SetupGithubReviewPolling(GithubReviewPollingSetupRequest),
    PollGithubReview,
    EvaluatePostTurn(Box<PostTurnRequest>),
}
