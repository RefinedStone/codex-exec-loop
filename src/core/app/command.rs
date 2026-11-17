use super::{
    ConversationPreferencePersistenceRequest, GithubReviewPollingSetupRequest,
    ManualPromptPreparationIntent, PlanningEditorMutationRequest, PlanningEditorSessionIdentity,
    PlanningEditorStageTarget, PlanningWorkspaceResetIntent, PostTurnEvaluationCorrelation,
    PostTurnRouteResolution, QueueMutationIntent, SessionCatalogLoadIntent, ThreadPromptSubmission,
    TurnSubmissionRequest,
};
use crate::domain::conversation::{
    ConversationApprovalDecision, ConversationApprovalRequestIdentity, ConversationTurnSteerRequest,
};
use crate::domain::planning::PostTurnRequest;
use crate::domain::recent_sessions::SessionRenameRequest;

impl AppCommand {
    /*
     * 이 명령의 연산 id 를 넣는다 (plan 18 K4-4). 런타임이 명령을 기록한 뒤 부른다. core 는 그 명령의 끝을 기록할 때
     * 이 id 를 쓴다. thread 지정 프롬프트는 준비와 턴을 거치므로 id 를 제출 값에 싣는다. 효과 하나로 끝나는 명령
     * 다섯은 `OfOperation` 으로 감싼다. 받아들이면 끝나는 명령과 읽기 명령은 그대로 둔다.
     */
    pub fn with_operation(self, op_id: i64) -> Self {
        match self {
            Self::SubmitThreadPrompt(mut submission) => {
                submission.op_id = Some(op_id);
                Self::SubmitThreadPrompt(submission)
            }
            Self::RenameSession(_)
            | Self::ResetPlanningWorkspace(_)
            | Self::RequestStopAllSessions
            | Self::SteerTurn(_)
            | Self::SubmitApprovalDecision { .. } => Self::OfOperation {
                op_id,
                command: Box::new(self),
            },
            other => other,
        }
    }

    /// 이 명령이 가리키는 thread 이다. 끝 기록의 `thread_id` 이다. thread 를 가리키지 않는 명령은 비어 있다.
    pub fn operation_thread_id(&self) -> &str {
        match self {
            Self::RenameSession(request) => &request.thread_id,
            Self::SteerTurn(request) => &request.thread_id,
            _ => "",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AppCommand {
    /*
     * 연산 id 를 가진 명령이다 (plan 18 K4-4b). 커널이 기록한 클라이언트 명령만 이 모양이다. core 는 안의 명령을
     * 그대로 처리하고, 그 명령이 낸 효과의 완료를 그 연산의 끝으로 적는다. `with_operation` 이 만든다.
     */
    OfOperation {
        op_id: i64,
        command: Box<AppCommand>,
    },
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
    // thread 를 지정한 프롬프트이다 (plan 18 K3-3b). core 가 준비와 제출을 이어서 한다.
    SubmitThreadPrompt(Box<ThreadPromptSubmission>),
    // 호스트 전용이다 (plan 18 K3-3b). 병렬 제어부의 병렬 모드 값이 바뀌면 호스트가 알린다. core 는 thread 지정
    // 프롬프트를 받을 때와 준비가 끝나 제출할 때 이 값을 본다.
    SetParallelModeEnabled(bool),
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
    // 호스트 전용이다 (plan 19 Q2-3). 원격 준비가 큐에 남긴 프롬프트 하나를 커널이 쉴 때 다시 낸다.
    ResubmitQueuedThreadPrompt,
    // 호스트 전용이다 (plan 19 Q3-1). 커널을 조립할 때 사건 기록에서 읽은 끝나지 않은 남김을 되돌린다.
    RestoreQueuedThreadPrompts(Vec<crate::domain::kernel_event::QueuedThreadPrompt>),
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
