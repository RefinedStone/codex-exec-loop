use super::{
    ApprovalDecisionCorrelation, ApprovalReviewPersistenceCorrelation, ConversationLoadCorrelation,
    ConversationPreferencePersistenceCorrelation, DirectionsMaintenanceLoadCorrelation,
    GithubReviewPollCorrelation, GithubReviewPollingSetupCorrelation,
    GithubReviewPollingSetupRequest, ParallelPeekLoadCorrelation, PlanningEditorMutationRequest,
    PlanningRuntimeRefreshCorrelation, PlanningWorkspaceOperationCorrelation,
    PostTurnEvaluationCorrelation, QueueAuthorityLoadCorrelation, QueueMutationCorrelation,
    ReviewCenterLoadCorrelation, SessionCatalogLoadCorrelation, SessionRenameCorrelation,
    StartupCheckCorrelation, StopRequestAttempt, StopRequestCorrelation, TurnSteerCorrelation,
    TurnSubmissionCorrelation, TurnSubmissionRequest,
};
use crate::domain::conversation::ConversationTurnSteerRequest;
use crate::domain::github_review::GithubPullRequestPollState;
use crate::domain::planning::{ManualPromptRequest, PostTurnRequest};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CoreEffect {
    RunStartupChecks {
        correlation: StartupCheckCorrelation,
    },
    LoadSessionCatalog {
        correlation: SessionCatalogLoadCorrelation,
    },
    RenameSession {
        correlation: SessionRenameCorrelation,
    },
    LoadConversation {
        correlation: ConversationLoadCorrelation,
        fallback_workspace_directory: String,
    },
    LoadParallelPeekConversation {
        correlation: ParallelPeekLoadCorrelation,
    },
    LoadReviewCenter {
        correlation: ReviewCenterLoadCorrelation,
    },
    LoadQueueAuthority {
        correlation: QueueAuthorityLoadCorrelation,
    },
    LoadDirectionsMaintenance {
        correlation: DirectionsMaintenanceLoadCorrelation,
    },
    LoadPlanningRuntime {
        correlation: PlanningRuntimeRefreshCorrelation,
    },
    ResetPlanningWorkspace {
        correlation: PlanningWorkspaceOperationCorrelation,
    },
    StageSimplePlanningDraft {
        correlation: PlanningWorkspaceOperationCorrelation,
    },
    StagePlanningEditor {
        correlation: PlanningWorkspaceOperationCorrelation,
    },
    MutatePlanningEditor {
        correlation: PlanningWorkspaceOperationCorrelation,
        request: Box<PlanningEditorMutationRequest>,
    },
    LoadSimplePlanningEditor {
        correlation: PlanningWorkspaceOperationCorrelation,
    },
    PromoteSimplePlanningDraft {
        correlation: PlanningWorkspaceOperationCorrelation,
    },
    ExecuteQueueMutation {
        correlation: QueueMutationCorrelation,
    },
    PrepareManualPrompt(Box<ManualPromptRequest>),
    CancelManualPromptPreparation {
        correlation: crate::domain::planning::ManualPromptCorrelation,
    },
    SubmitTurn {
        correlation: TurnSubmissionCorrelation,
        request: Box<TurnSubmissionRequest>,
    },
    RequestStopAllSessions {
        correlation: StopRequestCorrelation,
        attempt: StopRequestAttempt,
    },
    InvalidateStopRequest {
        correlation: StopRequestCorrelation,
    },
    SteerTurn {
        correlation: TurnSteerCorrelation,
        request: ConversationTurnSteerRequest,
    },
    SubmitApprovalDecision {
        correlation: ApprovalDecisionCorrelation,
    },
    PersistApprovalReview {
        correlation: ApprovalReviewPersistenceCorrelation,
    },
    PersistConversationPreferences {
        correlation: ConversationPreferencePersistenceCorrelation,
    },
    SetupGithubReviewPolling {
        correlation: GithubReviewPollingSetupCorrelation,
        request: GithubReviewPollingSetupRequest,
    },
    PollGithubReview {
        setup_correlation: GithubReviewPollingSetupCorrelation,
        correlation: GithubReviewPollCorrelation,
        previous_state: Option<GithubPullRequestPollState>,
    },
    EvaluatePostTurn {
        correlation: PostTurnEvaluationCorrelation,
        request: Box<PostTurnRequest>,
    },
}
