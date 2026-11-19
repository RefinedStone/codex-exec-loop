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
    // 전체 정지 뒤에 시작한 턴 하나만 멈춘다 (#2304). 완료는 `AfterTurnStarted` 정지 시도의 완료이다.
    InterruptTurn {
        correlation: StopRequestCorrelation,
        thread_id: String,
        turn_id: String,
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
