use super::{
    ConversationPreferencePersistenceRequest, GithubReviewPollingSetupRequest,
    ManualPromptPreparationIntent, PlanningEditorMutationRequest, PlanningEditorSessionIdentity,
    PlanningEditorStageTarget, PlanningWorkspaceResetIntent, PostTurnEvaluationCorrelation,
    PostTurnRouteResolution, QueueMutationIntent, SessionCatalogLoadIntent, TurnSubmissionRequest,
};
use crate::domain::conversation::{
    ConversationApprovalDecision, ConversationApprovalRequestIdentity, ConversationTurnSteerRequest,
};
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
    LoadSessionCatalog(SessionCatalogLoadIntent),
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
    StagePlanningEditor {
        workspace_directory: String,
        target: PlanningEditorStageTarget,
    },
    MutatePlanningEditor {
        workspace_directory: String,
        request: Box<PlanningEditorMutationRequest>,
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
    SubmitTurn(Box<TurnSubmissionRequest>),
    RequestStopAllSessions,
    SteerTurn(ConversationTurnSteerRequest),
    SubmitApprovalDecision {
        request_identity: ConversationApprovalRequestIdentity,
        decision: ConversationApprovalDecision,
    },
    PersistConversationPreferences(Box<ConversationPreferencePersistenceRequest>),
    SetAutoFollowMaxTurns {
        value: usize,
    },
    PausePostTurnContinuation,
    SetParallelPostTurnRearm {
        rearmed: bool,
    },
    ResolvePostTurnContinuation {
        correlation: PostTurnEvaluationCorrelation,
        resolution: PostTurnRouteResolution,
    },
    SetupGithubReviewPolling(GithubReviewPollingSetupRequest),
    // 호스트 전용이다 (plan 18 K1-4). core 가 기다리는 완료 턴의 평가를 시작한다. 병렬 값은 병렬 제어부를 가진
    // 호스트만 안다.
    EvaluateCompletedTurn {
        parallel_mode_enabled: bool,
        parallel_automation_epoch_id: Option<u64>,
    },
    // 클라이언트가 고른 턴 옵션(모델, 추론 수준)이다. 상태 값이다. core 는 자동 후속 턴에 이 값을 쓴다.
    SetConversationTurnOptions(crate::domain::conversation::ConversationTurnOptions),
    // 호스트 전용이다 (plan 18 K1-5). 평가 경로가 AutoSubmit 이면 호스트가 자동 후속 턴을 core 로 제출한다.
    SubmitAutoFollowTurn(Box<AutoFollowTurnSubmission>),
    EvaluatePostTurn(Box<PostTurnRequest>),
}

/// 자동 후속 턴 제출이다 (plan 18 K1-5). 평가가 만든 프롬프트와, 호스트가 아는 병렬 사용 여부를 담는다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AutoFollowTurnSubmission {
    pub source: super::PostTurnEvaluationCorrelation,
    pub completed_turn_id: String,
    pub prompt: String,
    pub transcript_text: String,
    pub mode_label: String,
    pub parallel_mode_enabled: bool,
}
