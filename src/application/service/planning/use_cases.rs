use super::authoring::directions::PlanningDirectionsService;
use super::authoring::directions::{DirectionsMaintenanceSummary, QueueIdleReviewContext};
use super::authoring::init::{
    PlanningDraftEditorFile, PlanningDraftEditorSession, PlanningDraftPromoteResult,
    PlanningDraftSaveResult, PlanningInitService, PlanningInitStageResult,
    PlanningWorkspaceInitResult,
};
use super::authoring::proposal_promotion::{
    PlanningProposalPromotionOutcome, PlanningProposalPromotionRequest,
    PlanningProposalPromotionService,
};
use super::repair::doctor::{PlanningDoctorReport, PlanningDoctorService};
use super::repair::reconciliation::{
    PlanningExecutionSnapshot, PlanningReconciliationResult, PlanningRepairRequest,
    PlanningRepairRetryReason,
};
use super::repair::reset::{
    PlanningResetService, PlanningResetTarget, PlanningWorkspaceResetResult,
};
use super::runtime::facade::{
    PlanningMainSessionHandoff, PlanningRuntimeAutoFollowDecision,
    PlanningRuntimeAutoFollowPreview, PlanningRuntimeAutoFollowPreviewRequest,
    PlanningRuntimeAutoFollowRequest, PlanningRuntimeFacadeService,
    PlanningRuntimeQueuedAutoFollowPrompt, PlanningRuntimeStatusProjection,
    PlanningRuntimeStatusProjectionRequest, PlanningRuntimeSummaryLineRequest,
    PlanningSubSessionHandoff, PlanningTaskHandoff,
};
use super::runtime::intake::{
    PlanningTaskIntakeCommitResult, PlanningTaskIntakeProposal, PlanningTaskIntakeRequest,
    PlanningTaskIntakeService,
};
use super::runtime::manual_intake::{
    ManualPromptIntakeOutcome, ManualPromptIntakeRequest, ManualPromptIntakeService,
};
use super::runtime::policy::PlanningAutoFollowBlockReason;
use super::runtime::prompt::{PlanningRuntimeProjection, PlanningRuntimeWorkspaceStatus};
use super::task_tool::{
    PlanningTaskToolRequest, PlanningTaskToolResponse, PlanningTaskToolService,
    planning_task_tool_contract_json,
};
use super::worker::orchestration::{
    PlanningLedgerRepairRequest, PlanningOfficialCompletionRefreshRequest,
    PlanningQueueRefreshMode, PlanningQueueRefreshRequest, PlanningWorkerOrchestrationService,
    PlanningWorkerRunOutcome,
};
use crate::application::service::parallel_agent_persona::ParallelAgentPersona;
use crate::domain::planning::{
    PlanningOfficialCompletionRefreshContract, PriorityQueueTask, QueueIdlePolicy,
};

pub const PLANNING_WORKER_REFRESH_FAILURE_BLOCK_REASON: &str = "planning worker refresh failed; auto-follow stays paused until the next accepted planning worker refresh";
pub const OFFICIAL_COMPLETION_REFRESH_FAILURE_BLOCK_REASON: &str =
    "official completion refresh failed; the leased slot stays reserved until planning is repaired";
pub const DEFAULT_POST_TURN_REPAIR_ATTEMPT_LIMIT: usize = 2;

/*
 * 이 파일은 planning의 public application facade다.
 * 의도적으로 business logic을 거의 담지 않는다. 각 use-case group은 inbound adapter에 stable API를 제공하고,
 * 실제 behavior의 ownership은 authoring/runtime/repair/task-tool/worker service에 남긴다.
 */
#[derive(Clone)]
pub struct PlanningWorkspaceUseCases {
    // workspace use case는 operator가 관리하는 artifact를 다룬다. initialization, draft editing, doctor/reset,
    // direction maintenance가 모두 active planning workspace와 authority seed를 공유하기 때문이다.
    init_service: PlanningInitService,
    reset_service: PlanningResetService,
    doctor_service: PlanningDoctorService,
    directions_service: PlanningDirectionsService,
}
impl PlanningWorkspaceUseCases {
    pub(super) fn new(
        init_service: PlanningInitService,
        reset_service: PlanningResetService,
        doctor_service: PlanningDoctorService,
        directions_service: PlanningDirectionsService,
    ) -> Self {
        Self {
            init_service,
            reset_service,
            doctor_service,
            directions_service,
        }
    }
    pub fn has_planning_workspace(&self, workspace_dir: &str) -> anyhow::Result<bool> {
        self.init_service.has_planning_workspace(workspace_dir)
    }
    pub fn has_planning_candidate_workspace(&self, workspace_dir: &str) -> anyhow::Result<bool> {
        self.init_service
            .has_planning_candidate_workspace(workspace_dir)
    }
    pub fn initialize_simple_workspace(
        &self,
        workspace_dir: &str,
    ) -> anyhow::Result<PlanningWorkspaceInitResult> {
        // simple initialization은 baseline planning workspace를 즉시 만든다. 더 풍부한 editing path는 아래에서
        // draft를 stage한 뒤 promotion하는 흐름을 사용한다.
        self.init_service.initialize_simple_workspace(workspace_dir)
    }
    pub fn reset_workspace(
        &self,
        workspace_dir: &str,
        target: PlanningResetTarget,
    ) -> anyhow::Result<PlanningWorkspaceResetResult> {
        self.reset_service.reset_workspace(workspace_dir, target)
    }
    pub fn inspect_workspace(&self, workspace_dir: &str) -> PlanningDoctorReport {
        self.doctor_service.inspect_workspace(workspace_dir)
    }
    pub fn stage_simple_mode_draft(
        &self,
        workspace_dir: &str,
    ) -> anyhow::Result<PlanningInitStageResult> {
        self.init_service.stage_simple_mode_draft(workspace_dir)
    }
    pub fn stage_manual_editor_session(
        &self,
        workspace_dir: &str,
    ) -> anyhow::Result<PlanningDraftEditorSession> {
        // manual editor session은 나중의 save/promote가 검증하고 publish하기 전까지 draft file을 active authority에서 격리한다.
        self.init_service.stage_manual_editor_session(workspace_dir)
    }
    pub fn load_manual_editor_session(
        &self,
        workspace_dir: &str,
        draft_name: &str,
    ) -> anyhow::Result<PlanningDraftEditorSession> {
        self.init_service
            .load_manual_editor_session(workspace_dir, draft_name)
    }
    pub fn save_draft_editor_files(
        &self,
        workspace_dir: &str,
        draft_name: &str,
        files: &[PlanningDraftEditorFile],
    ) -> anyhow::Result<PlanningDraftSaveResult> {
        self.init_service
            .save_draft_editor_files(workspace_dir, draft_name, files)
    }
    pub fn promote_draft_editor_files(
        &self,
        workspace_dir: &str,
        draft_name: &str,
        files: &[PlanningDraftEditorFile],
    ) -> anyhow::Result<PlanningDraftPromoteResult> {
        self.init_service
            .promote_draft_editor_files(workspace_dir, draft_name, files)
    }
    pub fn promote_staged_draft(
        &self,
        workspace_dir: &str,
        draft_name: &str,
    ) -> anyhow::Result<PlanningDraftPromoteResult> {
        self.init_service
            .promote_staged_draft(workspace_dir, draft_name)
    }
    pub fn load_summary(
        &self,
        workspace_dir: &str,
    ) -> anyhow::Result<DirectionsMaintenanceSummary> {
        // direction maintenance는 구현이 PlanningDirectionsService에 있어도 workspace use case에 묶는다.
        // operator는 planning strategy와 workspace file을 하나의 관리 흐름으로 편집하기 때문이다.
        self.directions_service.load_summary(workspace_dir)
    }
    pub fn load_queue_idle_review_context(
        &self,
        workspace_dir: &str,
    ) -> anyhow::Result<QueueIdleReviewContext> {
        self.directions_service
            .load_queue_idle_review_context(workspace_dir)
    }
    pub fn stage_detail_doc_editor_session(
        &self,
        workspace_dir: &str,
        direction_id: &str,
    ) -> anyhow::Result<PlanningDraftEditorSession> {
        self.directions_service
            .stage_detail_doc_editor_session(workspace_dir, direction_id)
    }
    pub fn stage_queue_idle_prompt_editor_session(
        &self,
        workspace_dir: &str,
    ) -> anyhow::Result<PlanningDraftEditorSession> {
        self.directions_service
            .stage_queue_idle_prompt_editor_session(workspace_dir)
    }
}
#[derive(Clone)]
pub struct PlanningRuntimeUseCases {
    // runtime use case는 session 실행 중 호출된다. prompt/handoff rendering은 runtime facade에 남기고,
    // proposed task intake는 mutation-backed intake service로 위임한다.
    runtime_facade: PlanningRuntimeFacadeService,
    task_intake: PlanningTaskIntakeService,
    manual_intake: ManualPromptIntakeService,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanningTurnExecutionSnapshotCaptureRequest {
    pub workspace_directory: String,
}
impl PlanningTurnExecutionSnapshotCaptureRequest {
    pub fn new(workspace_directory: impl Into<String>) -> Self {
        Self {
            workspace_directory: workspace_directory.into(),
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanningTurnExecutionSnapshotCapture {
    pub workspace_directory: String,
    pub state: PlanningTurnExecutionSnapshotCaptureState,
}
impl PlanningTurnExecutionSnapshotCapture {
    pub fn ready(
        workspace_directory: impl Into<String>,
        snapshot: PlanningExecutionSnapshot,
    ) -> Self {
        Self {
            workspace_directory: workspace_directory.into(),
            state: PlanningTurnExecutionSnapshotCaptureState::Ready(snapshot),
        }
    }

    pub fn capture_failed(workspace_directory: impl Into<String>, message: String) -> Self {
        Self {
            workspace_directory: workspace_directory.into(),
            state: PlanningTurnExecutionSnapshotCaptureState::CaptureFailed(message),
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlanningTurnExecutionSnapshotCaptureState {
    Ready(PlanningExecutionSnapshot),
    CaptureFailed(String),
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanningPostTurnReconciliationRequest<'a> {
    pub workspace_directory: &'a str,
    pub completed_turn_id: &'a str,
    pub changed_planning_file_paths: &'a [String],
    pub execution_snapshot_capture: Option<&'a PlanningTurnExecutionSnapshotCapture>,
    pub current_runtime_projection: &'a PlanningRuntimeProjection,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanningPostTurnReconciliationOutcome {
    pub reconciliation_result: PlanningReconciliationResult,
    pub runtime_projection: PlanningRuntimeProjection,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanningPostTurnWorkerPanelStartRequest<'a> {
    pub continuation_paused: bool,
    pub changed_planning_file_paths: &'a [String],
    pub current_runtime_projection: &'a PlanningRuntimeProjection,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlanningPostTurnWorkerPanelStartState {
    PreserveCurrent,
    RepairRunning,
    RefreshRunning,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanningPostTurnAutoFollowRequest<'a> {
    pub continuation_paused: bool,
    pub can_queue_next: bool,
    pub latest_agent_message: Option<&'a str>,
    pub stop_keyword: &'a str,
    pub stop_keyword_matched: bool,
    pub no_file_changes_stop_matched: bool,
    pub runtime_projection: &'a PlanningRuntimeProjection,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlanningPostTurnAutoFollowDecision {
    QueuePrompt(PlanningRuntimeQueuedAutoFollowPrompt),
    Skip(PlanningPostTurnAutoFollowSkipReason),
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlanningPostTurnAutoFollowSkipReason {
    PostTurnContinuationPaused,
    PlanningQueueDrained,
    PlanningQueueIdlePolicyStop,
    LimitReached,
    NoAgentReply,
    StopKeywordMatched,
    NoFileChanges,
    PlanningBlocked,
    PlanningQueueHeadRequired,
    PlanningRepeatedQueueHead,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanningPostTurnQueueRefreshPreparationRequest<'a> {
    pub workspace_directory: &'a str,
    pub parent_thread_id: Option<&'a str>,
    pub completed_turn_id: &'a str,
    pub latest_user_message: Option<&'a str>,
    pub latest_main_reply: Option<&'a str>,
    pub previous_handoff_task: Option<&'a PlanningTaskHandoff>,
    pub current_runtime_projection: &'a PlanningRuntimeProjection,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlanningPostTurnQueueRefreshPreparation {
    Skipped(Box<PlanningPostTurnQueueRefreshSkipped>),
    Ready(Box<PlanningPreparedQueueRefresh>),
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanningPostTurnQueueRefreshSkipped {
    pub reason: PlanningPostTurnQueueRefreshSkipReason,
    pub runtime_projection: PlanningRuntimeProjection,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlanningPostTurnQueueRefreshSkipReason {
    PlanningRuntimeNotReady,
    LatestMainReplyEmpty,
    QueueIdleReviewContextUnavailable,
    QueueIdlePolicyStop,
    QueueIdlePromptMissing,
}
impl PlanningPostTurnQueueRefreshSkipReason {
    pub fn log_label(self) -> &'static str {
        match self {
            Self::PlanningRuntimeNotReady => "planning_runtime_not_ready",
            Self::LatestMainReplyEmpty => "latest_main_reply_empty",
            Self::QueueIdleReviewContextUnavailable => "queue_idle_review_context_unavailable",
            Self::QueueIdlePolicyStop => "queue_idle_policy_stop",
            Self::QueueIdlePromptMissing => "queue_idle_prompt_missing",
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanningPreparedQueueRefresh {
    workspace_directory: String,
    parent_thread_id: Option<String>,
    completed_turn_id: String,
    latest_user_message: Option<String>,
    latest_main_reply: String,
    previous_handoff_task: Option<PlanningTaskHandoff>,
    mode: PlanningPreparedQueueRefreshMode,
    worker_prompt: String,
}
impl PlanningPreparedQueueRefresh {
    fn new(
        request: &PlanningPostTurnQueueRefreshPreparationRequest<'_>,
        latest_main_reply: &str,
        mode: PlanningPreparedQueueRefreshMode,
        worker_prompt: String,
    ) -> Self {
        Self {
            workspace_directory: request.workspace_directory.to_string(),
            parent_thread_id: request.parent_thread_id.map(str::to_string),
            completed_turn_id: request.completed_turn_id.to_string(),
            latest_user_message: request.latest_user_message.map(str::to_string),
            latest_main_reply: latest_main_reply.to_string(),
            previous_handoff_task: request.previous_handoff_task.cloned(),
            mode,
            worker_prompt,
        }
    }

    pub fn worker_prompt(&self) -> &str {
        &self.worker_prompt
    }

    pub fn mode_label(&self) -> &'static str {
        self.mode.log_label()
    }

    pub fn panel_operation_label(&self) -> &'static str {
        self.mode.panel_operation_label()
    }

    pub fn latest_main_reply_char_count(&self) -> usize {
        self.latest_main_reply.chars().count()
    }

    pub fn has_latest_user_message(&self) -> bool {
        self.latest_user_message.is_some()
    }

    pub fn has_previous_handoff(&self) -> bool {
        self.previous_handoff_task.is_some()
    }

    pub fn is_queue_idle_derivation(&self) -> bool {
        matches!(
            self.mode,
            PlanningPreparedQueueRefreshMode::DeriveQueueHeadWhenQueueIdle { .. }
        )
    }

    fn as_refresh_request(&self) -> PlanningQueueRefreshRequest<'_> {
        PlanningQueueRefreshRequest {
            workspace_directory: &self.workspace_directory,
            parent_thread_id: self.parent_thread_id.as_deref(),
            completed_turn_id: &self.completed_turn_id,
            latest_user_message: self.latest_user_message.as_deref(),
            latest_main_reply: &self.latest_main_reply,
            previous_handoff_task: self.previous_handoff_task.as_ref(),
            mode: self.mode.as_refresh_mode(),
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
enum PlanningPreparedQueueRefreshMode {
    FromLatestMainReply,
    DeriveQueueHeadWhenQueueIdle { queue_idle_prompt_markdown: String },
}
impl PlanningPreparedQueueRefreshMode {
    fn as_refresh_mode(&self) -> PlanningQueueRefreshMode<'_> {
        match self {
            Self::FromLatestMainReply => PlanningQueueRefreshMode::FromLatestMainReply,
            Self::DeriveQueueHeadWhenQueueIdle {
                queue_idle_prompt_markdown,
            } => PlanningQueueRefreshMode::DeriveQueueHeadWhenQueueIdle {
                queue_idle_prompt_markdown,
            },
        }
    }

    fn log_label(&self) -> &'static str {
        match self {
            Self::FromLatestMainReply => "from_latest_main_reply",
            Self::DeriveQueueHeadWhenQueueIdle { .. } => "derive_queue_head_when_queue_idle",
        }
    }

    fn panel_operation_label(&self) -> &'static str {
        match self {
            Self::FromLatestMainReply => "refresh",
            Self::DeriveQueueHeadWhenQueueIdle { .. } => "queue-idle-derive",
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanningPostTurnRepairRequest<'a> {
    pub workspace_directory: &'a str,
    pub parent_thread_id: Option<&'a str>,
    pub completed_turn_id: &'a str,
    pub repair_request: &'a PlanningRepairRequest,
    pub previous_handoff_task: Option<&'a PlanningTaskHandoff>,
    pub max_attempts: usize,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanningPostTurnRepairOutcome {
    pub runtime_projection: PlanningRuntimeProjection,
    pub resolved: bool,
    pub attempts: Vec<PlanningPostTurnRepairAttempt>,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanningPostTurnRepairAttempt {
    pub attempt_number: usize,
    pub max_attempts: usize,
    pub retry_reason: Option<PlanningRepairRetryReason>,
    pub started_runtime_projection: PlanningRuntimeProjection,
    pub worker_prompt: String,
    pub result: PlanningPostTurnRepairAttemptResult,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlanningPostTurnRepairAttemptResult {
    WorkerFailed {
        detail: String,
        error: String,
    },
    WorkerSucceeded {
        outcome: Box<PlanningWorkerRunOutcome>,
        next_repair_request: Option<PlanningRepairRequest>,
        next_retry_reason: Option<PlanningRepairRetryReason>,
        resolved: bool,
        exhausted: bool,
    },
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanningPostTurnQueueRefreshFinalizationRequest<'a> {
    pub workspace_directory: &'a str,
    pub previous_handoff_task: Option<&'a PlanningTaskHandoff>,
    pub previous_runtime_projection: &'a PlanningRuntimeProjection,
    pub refreshed_runtime_projection: &'a PlanningRuntimeProjection,
    pub queue_idle_derivation: bool,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanningPostTurnQueueRefreshFinalizationOutcome {
    pub runtime_projection: PlanningRuntimeProjection,
    pub events: Vec<PlanningPostTurnQueueRefreshFinalizationEvent>,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlanningPostTurnQueueRefreshFinalizationEvent {
    ProposalPromotionCompleted {
        outcome: PlanningProposalPromotionOutcome,
    },
    ProposalPromotionFailed {
        detail: String,
        runtime_projection: PlanningRuntimeProjection,
    },
    QueueIdleDerivationEmpty {
        detail: String,
    },
    RepeatedQueueHead {
        detail: String,
        runtime_projection: PlanningRuntimeProjection,
    },
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanningPostTurnOfficialCompletionPreparationRequest<'a> {
    pub planning_workspace_directory: &'a str,
    pub turn_workspace_directory: &'a str,
    pub parent_thread_id: Option<&'a str>,
    pub latest_user_message: Option<&'a str>,
    pub latest_main_reply: Option<&'a str>,
    pub previous_handoff_task: Option<&'a PlanningTaskHandoff>,
    pub current_runtime_projection: &'a PlanningRuntimeProjection,
    pub contract: &'a PlanningOfficialCompletionRefreshContract,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlanningPostTurnOfficialCompletionPreparation {
    Blocked(Box<PlanningPostTurnOfficialCompletionBlocked>),
    Ready(Box<PlanningPreparedOfficialCompletionRefresh>),
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanningPostTurnOfficialCompletionBlocked {
    pub planning_workspace_projection: PlanningRuntimeProjection,
    pub failure_detail: String,
    pub failure_projection: PlanningRuntimeProjection,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanningPreparedOfficialCompletionRefresh {
    workspace_directory: String,
    parent_thread_id: Option<String>,
    latest_user_message: Option<String>,
    latest_main_reply: String,
    previous_handoff_task: Option<PlanningTaskHandoff>,
    contract: PlanningOfficialCompletionRefreshContract,
    planning_workspace_projection: PlanningRuntimeProjection,
    worker_prompt: String,
}
impl PlanningPreparedOfficialCompletionRefresh {
    fn new(
        request: &PlanningPostTurnOfficialCompletionPreparationRequest<'_>,
        planning_workspace_projection: PlanningRuntimeProjection,
        latest_main_reply: &str,
        worker_prompt: String,
    ) -> Self {
        Self {
            workspace_directory: request.planning_workspace_directory.to_string(),
            parent_thread_id: request.parent_thread_id.map(str::to_string),
            latest_user_message: request.latest_user_message.map(str::to_string),
            latest_main_reply: latest_main_reply.to_string(),
            previous_handoff_task: request.previous_handoff_task.cloned(),
            contract: request.contract.clone(),
            planning_workspace_projection,
            worker_prompt,
        }
    }

    pub fn planning_workspace_projection(&self) -> &PlanningRuntimeProjection {
        &self.planning_workspace_projection
    }

    pub fn worker_prompt(&self) -> &str {
        &self.worker_prompt
    }

    pub fn latest_main_reply_char_count(&self) -> usize {
        self.latest_main_reply.chars().count()
    }

    pub fn has_latest_user_message(&self) -> bool {
        self.latest_user_message.is_some()
    }

    pub fn has_previous_handoff(&self) -> bool {
        self.previous_handoff_task.is_some()
    }

    pub fn refresh_order(&self) -> u64 {
        self.contract.refresh_order
    }

    fn as_refresh_request(&self) -> PlanningOfficialCompletionRefreshRequest<'_> {
        PlanningOfficialCompletionRefreshRequest {
            workspace_directory: &self.workspace_directory,
            parent_thread_id: self.parent_thread_id.as_deref(),
            latest_user_message: self.latest_user_message.as_deref(),
            latest_main_reply: &self.latest_main_reply,
            previous_handoff_task: self.previous_handoff_task.as_ref(),
            contract: &self.contract,
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanningPostTurnOfficialCompletionFinalizationRequest<'a> {
    pub planning_workspace_directory: &'a str,
    pub previous_handoff_task: Option<&'a PlanningTaskHandoff>,
    pub previous_runtime_projection: &'a PlanningRuntimeProjection,
    pub refreshed_runtime_projection: &'a PlanningRuntimeProjection,
    pub worker_summary: Option<&'a str>,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanningPostTurnOfficialCompletionFinalizationOutcome {
    pub runtime_projection: PlanningRuntimeProjection,
    pub repeated_queue_head_detail: Option<String>,
    pub blocked_failure_detail: Option<String>,
    pub authority_refresh_outcome: Option<String>,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanningPostTurnOfficialCompletionRepairBlockRequest<'a> {
    pub runtime_projection: &'a PlanningRuntimeProjection,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanningPostTurnOfficialCompletionRepairBlockOutcome {
    pub runtime_projection: PlanningRuntimeProjection,
    pub failure_detail: &'static str,
}
impl PlanningRuntimeUseCases {
    pub(crate) fn new(
        runtime_facade: PlanningRuntimeFacadeService,
        task_intake: PlanningTaskIntakeService,
        manual_intake: ManualPromptIntakeService,
    ) -> Self {
        Self {
            runtime_facade,
            task_intake,
            manual_intake,
        }
    }
    pub fn build_manual_prompt(
        &self,
        operator_prompt: &str,
        _projection: &PlanningRuntimeProjection,
    ) -> Option<String> {
        self.runtime_facade.build_manual_prompt(operator_prompt)
    }
    pub fn prepare_manual_prompt_intake(
        &self,
        request: ManualPromptIntakeRequest,
    ) -> ManualPromptIntakeOutcome {
        self.manual_intake.prepare_manual_turn(request)
    }
    pub fn build_queued_task_handoff(
        &self,
        projection: &PlanningRuntimeProjection,
    ) -> Option<PlanningMainSessionHandoff> {
        // queued-task handoff는 caller가 따로 들고 있는 queue state가 아니라 current runtime projection에서 파생한다.
        self.runtime_facade.build_queued_task_handoff(projection)
    }
    pub fn build_main_session_task_handoff(
        &self,
        task: &PriorityQueueTask,
    ) -> PlanningMainSessionHandoff {
        self.runtime_facade.build_main_session_task_handoff(task)
    }
    pub fn build_sub_session_task_handoff(
        &self,
        task: &PriorityQueueTask,
    ) -> PlanningSubSessionHandoff {
        self.runtime_facade.build_sub_session_task_handoff(task)
    }
    pub fn build_sub_session_task_handoff_with_persona(
        &self,
        task: &PriorityQueueTask,
        persona: ParallelAgentPersona,
    ) -> PlanningSubSessionHandoff {
        self.runtime_facade
            .build_sub_session_task_handoff_with_persona(task, persona)
    }
    pub fn decide_auto_follow(
        &self,
        request: PlanningRuntimeAutoFollowRequest<'_>,
    ) -> PlanningRuntimeAutoFollowDecision {
        self.runtime_facade.decide_auto_follow(request)
    }
    pub fn build_auto_follow_preview(
        &self,
        request: PlanningRuntimeAutoFollowPreviewRequest<'_>,
    ) -> PlanningRuntimeAutoFollowPreview {
        self.runtime_facade.build_auto_follow_preview(request)
    }
    pub fn build_summary_line(
        &self,
        request: PlanningRuntimeSummaryLineRequest<'_>,
    ) -> Option<String> {
        self.runtime_facade.build_summary_line(request)
    }
    pub fn build_auto_follow_status_projection(
        &self,
        request: PlanningRuntimeStatusProjectionRequest<'_>,
    ) -> PlanningRuntimeStatusProjection {
        self.runtime_facade
            .build_auto_follow_status_projection(request)
    }
    pub fn load_runtime_projection_or_invalid(
        &self,
        workspace_dir: &str,
    ) -> PlanningRuntimeProjection {
        self.runtime_facade
            .load_runtime_projection_or_invalid(workspace_dir)
    }
    pub fn prepare_task_intake(
        &self,
        request: PlanningTaskIntakeRequest,
    ) -> anyhow::Result<PlanningTaskIntakeProposal> {
        // intake는 two-step flow다. prepare가 preview/proposal을 만들고, inbound UI는 commit 전에 이를 inspect할 수 있다.
        self.task_intake.prepare_task_intake(request)
    }
    pub fn commit_task_intake(
        &self,
        proposal: &PlanningTaskIntakeProposal,
    ) -> anyhow::Result<PlanningTaskIntakeCommitResult> {
        self.task_intake.commit_task_intake(proposal)
    }
    pub fn load_execution_snapshot(
        &self,
        workspace_dir: &str,
    ) -> anyhow::Result<PlanningExecutionSnapshot> {
        self.runtime_facade.load_execution_snapshot(workspace_dir)
    }
    pub fn capture_turn_execution_snapshot(
        &self,
        request: PlanningTurnExecutionSnapshotCaptureRequest,
    ) -> PlanningTurnExecutionSnapshotCapture {
        match self.load_execution_snapshot(&request.workspace_directory) {
            Ok(snapshot) => {
                PlanningTurnExecutionSnapshotCapture::ready(request.workspace_directory, snapshot)
            }
            Err(error) => PlanningTurnExecutionSnapshotCapture::capture_failed(
                request.workspace_directory,
                format!(
                    "planning reconciliation could not capture the execution snapshot before the turn started: {error}"
                ),
            ),
        }
    }
    pub fn reconcile_after_turn(
        &self,
        workspace_dir: &str,
        turn_id: &str,
        changed_planning_file_paths: &[String],
        execution_snapshot: &PlanningExecutionSnapshot,
    ) -> anyhow::Result<PlanningReconciliationResult> {
        // reconciliation은 turn 전에 capture한 execution snapshot을 받고, 완료 뒤 바뀐 planning file과 비교한다.
        self.runtime_facade.reconcile_after_turn(
            workspace_dir,
            turn_id,
            changed_planning_file_paths,
            execution_snapshot,
        )
    }
    pub fn reconcile_post_turn(
        &self,
        request: PlanningPostTurnReconciliationRequest<'_>,
    ) -> PlanningPostTurnReconciliationOutcome {
        let reconciliation_result = self.reconcile_post_turn_result(&request);
        let runtime_projection =
            if let Some(block_reason) = reconciliation_result.auto_follow_block_reason.clone() {
                PlanningRuntimeProjection::invalid(block_reason)
            } else if request.changed_planning_file_paths.is_empty() {
                request.current_runtime_projection.clone()
            } else {
                self.load_runtime_projection_or_invalid(request.workspace_directory)
            };
        PlanningPostTurnReconciliationOutcome {
            reconciliation_result,
            runtime_projection,
        }
    }

    fn reconcile_post_turn_result(
        &self,
        request: &PlanningPostTurnReconciliationRequest<'_>,
    ) -> PlanningReconciliationResult {
        let requires_execution_snapshot = request
            .changed_planning_file_paths
            .iter()
            .any(|path| PlanningExecutionSnapshot::captures_path(path));
        if !requires_execution_snapshot {
            return PlanningReconciliationResult::default();
        }
        let Some(capture) = request.execution_snapshot_capture else {
            return blocked_reconciliation_result(
                "planning reconciliation could not restore protected planning files because the execution snapshot was unavailable"
                    .to_string(),
            );
        };
        if capture.workspace_directory != request.workspace_directory {
            return blocked_reconciliation_result(format!(
                "planning reconciliation ignored a stale execution snapshot captured for {} while the completed turn resolved in {}",
                capture.workspace_directory, request.workspace_directory
            ));
        }
        let execution_snapshot = match &capture.state {
            PlanningTurnExecutionSnapshotCaptureState::Ready(snapshot) => snapshot,
            PlanningTurnExecutionSnapshotCaptureState::CaptureFailed(error_message) => {
                return blocked_reconciliation_result(error_message.clone());
            }
        };
        match self.reconcile_after_turn(
            request.workspace_directory,
            request.completed_turn_id,
            request.changed_planning_file_paths,
            execution_snapshot,
        ) {
            Ok(result) => result,
            Err(error) => PlanningReconciliationResult {
                notices: vec![format!("planning reconciliation failed: {error}")],
                auto_follow_block_reason: Some(
                    "planning reconciliation failed; auto-follow stays paused until the planning workspace is repaired"
                        .to_string(),
                ),
                ..PlanningReconciliationResult::default()
            },
        }
    }

    pub fn post_turn_worker_panel_start_state(
        &self,
        request: PlanningPostTurnWorkerPanelStartRequest<'_>,
    ) -> PlanningPostTurnWorkerPanelStartState {
        if request.continuation_paused {
            return PlanningPostTurnWorkerPanelStartState::PreserveCurrent;
        }
        if request
            .changed_planning_file_paths
            .iter()
            .any(|path| PlanningExecutionSnapshot::captures_path(path))
        {
            return PlanningPostTurnWorkerPanelStartState::RepairRunning;
        }
        if request.current_runtime_projection.workspace_status()
            == PlanningRuntimeWorkspaceStatus::ReadyNoTask
            && request.current_runtime_projection.queue_idle_policy() == QueueIdlePolicy::Stop
        {
            return PlanningPostTurnWorkerPanelStartState::PreserveCurrent;
        }
        PlanningPostTurnWorkerPanelStartState::RefreshRunning
    }

    pub fn decide_post_turn_auto_follow(
        &self,
        request: PlanningPostTurnAutoFollowRequest<'_>,
    ) -> PlanningPostTurnAutoFollowDecision {
        if request.continuation_paused {
            return PlanningPostTurnAutoFollowDecision::Skip(
                PlanningPostTurnAutoFollowSkipReason::PostTurnContinuationPaused,
            );
        }
        if request.runtime_projection.queue_is_drained() {
            return PlanningPostTurnAutoFollowDecision::Skip(
                PlanningPostTurnAutoFollowSkipReason::PlanningQueueDrained,
            );
        }
        if request.runtime_projection.workspace_status()
            == PlanningRuntimeWorkspaceStatus::ReadyNoTask
            && request.runtime_projection.queue_idle_policy() == QueueIdlePolicy::Stop
        {
            return PlanningPostTurnAutoFollowDecision::Skip(
                PlanningPostTurnAutoFollowSkipReason::PlanningQueueIdlePolicyStop,
            );
        }
        if !request.can_queue_next {
            return PlanningPostTurnAutoFollowDecision::Skip(
                PlanningPostTurnAutoFollowSkipReason::LimitReached,
            );
        }
        let Some(last_message) = request
            .latest_agent_message
            .map(str::trim)
            .filter(|message| !message.is_empty())
        else {
            return PlanningPostTurnAutoFollowDecision::Skip(
                PlanningPostTurnAutoFollowSkipReason::NoAgentReply,
            );
        };
        if request.stop_keyword_matched {
            return PlanningPostTurnAutoFollowDecision::Skip(
                PlanningPostTurnAutoFollowSkipReason::StopKeywordMatched,
            );
        }
        if request.no_file_changes_stop_matched {
            return PlanningPostTurnAutoFollowDecision::Skip(
                PlanningPostTurnAutoFollowSkipReason::NoFileChanges,
            );
        }
        match self.decide_auto_follow(PlanningRuntimeAutoFollowRequest {
            stop_keyword: request.stop_keyword,
            last_message,
            projection: request.runtime_projection,
        }) {
            PlanningRuntimeAutoFollowDecision::QueuePrompt(prompt) => {
                PlanningPostTurnAutoFollowDecision::QueuePrompt(prompt)
            }
            PlanningRuntimeAutoFollowDecision::Blocked(block_reason) => {
                PlanningPostTurnAutoFollowDecision::Skip(match block_reason {
                    PlanningAutoFollowBlockReason::InvalidWorkspace => {
                        PlanningPostTurnAutoFollowSkipReason::PlanningBlocked
                    }
                    PlanningAutoFollowBlockReason::ActionableQueueRequired => {
                        PlanningPostTurnAutoFollowSkipReason::PlanningQueueHeadRequired
                    }
                    PlanningAutoFollowBlockReason::RepeatedQueueHead => {
                        PlanningPostTurnAutoFollowSkipReason::PlanningRepeatedQueueHead
                    }
                })
            }
        }
    }
}

fn blocked_reconciliation_result(message: String) -> PlanningReconciliationResult {
    PlanningReconciliationResult {
        notices: vec![message.clone()],
        auto_follow_block_reason: Some(message),
        ..PlanningReconciliationResult::default()
    }
}
#[derive(Clone)]
pub struct PlanningTaskToolUseCases {
    // 이 얇은 wrapper는 worker-facing planning task tool을 다른 runtime planning action과 같은 use-case 묶음으로 노출한다.
    task_tool: PlanningTaskToolService,
}
impl PlanningTaskToolUseCases {
    pub(crate) fn new(task_tool: PlanningTaskToolService) -> Self {
        Self { task_tool }
    }
    pub fn contract_json(&self) -> &'static str {
        planning_task_tool_contract_json()
    }
    pub fn run(
        &self,
        workspace_dir: &str,
        request: PlanningTaskToolRequest,
    ) -> anyhow::Result<PlanningTaskToolResponse> {
        self.task_tool.handle_request(workspace_dir, request)
    }
}
#[derive(Clone)]
pub struct PlanningWorkerUseCases {
    // worker use case는 model-mediated queue refresh와 repair loop를 소유한다.
    // proposal promotion은 queue state가 알려진 뒤에는 deterministic하므로 별도 service로 분리한다.
    directions_service: PlanningDirectionsService,
    worker_orchestration: PlanningWorkerOrchestrationService,
    proposal_promotion: PlanningProposalPromotionService,
}
impl PlanningWorkerUseCases {
    pub(super) fn new(
        directions_service: PlanningDirectionsService,
        worker_orchestration: PlanningWorkerOrchestrationService,
        proposal_promotion: PlanningProposalPromotionService,
    ) -> Self {
        Self {
            directions_service,
            worker_orchestration,
            proposal_promotion,
        }
    }
    pub fn load_queue_idle_review_context(
        &self,
        workspace_dir: &str,
    ) -> anyhow::Result<QueueIdleReviewContext> {
        self.directions_service
            .load_queue_idle_review_context(workspace_dir)
    }
    pub fn prepare_post_turn_queue_refresh(
        &self,
        request: PlanningPostTurnQueueRefreshPreparationRequest<'_>,
    ) -> PlanningPostTurnQueueRefreshPreparation {
        let runtime_projection = request.current_runtime_projection;
        let skipped = |reason| {
            PlanningPostTurnQueueRefreshPreparation::Skipped(Box::new(
                PlanningPostTurnQueueRefreshSkipped {
                    reason,
                    runtime_projection: runtime_projection.clone(),
                },
            ))
        };
        if !matches!(
            runtime_projection.workspace_status(),
            PlanningRuntimeWorkspaceStatus::ReadyNoTask
                | PlanningRuntimeWorkspaceStatus::ReadyWithTask
        ) {
            return skipped(PlanningPostTurnQueueRefreshSkipReason::PlanningRuntimeNotReady);
        }
        let Some(latest_main_reply) = request
            .latest_main_reply
            .map(str::trim)
            .filter(|message| !message.is_empty())
        else {
            return skipped(PlanningPostTurnQueueRefreshSkipReason::LatestMainReplyEmpty);
        };
        let mode = match runtime_projection.workspace_status() {
            PlanningRuntimeWorkspaceStatus::ReadyWithTask => {
                PlanningPreparedQueueRefreshMode::FromLatestMainReply
            }
            PlanningRuntimeWorkspaceStatus::ReadyNoTask => {
                let review_context = match self
                    .load_queue_idle_review_context(request.workspace_directory)
                {
                    Ok(context) => context,
                    Err(_) => {
                        let reason =
                            PlanningPostTurnQueueRefreshSkipReason::QueueIdleReviewContextUnavailable;
                        return skipped(reason);
                    }
                };
                match review_context.policy {
                    QueueIdlePolicy::Stop => {
                        return skipped(
                            PlanningPostTurnQueueRefreshSkipReason::QueueIdlePolicyStop,
                        );
                    }
                    QueueIdlePolicy::ReviewAndEnqueue => {
                        let Some(prompt_markdown) = review_context.prompt_markdown else {
                            return skipped(
                                PlanningPostTurnQueueRefreshSkipReason::QueueIdlePromptMissing,
                            );
                        };
                        PlanningPreparedQueueRefreshMode::DeriveQueueHeadWhenQueueIdle {
                            queue_idle_prompt_markdown: prompt_markdown,
                        }
                    }
                }
            }
            PlanningRuntimeWorkspaceStatus::Uninitialized
            | PlanningRuntimeWorkspaceStatus::Invalid => {
                unreachable!("non-ready planning states return before queue refresh mode is built")
            }
        };
        let worker_prompt =
            self.worker_orchestration
                .render_refresh_queue_prompt(&PlanningQueueRefreshRequest {
                    workspace_directory: request.workspace_directory,
                    parent_thread_id: request.parent_thread_id,
                    completed_turn_id: request.completed_turn_id,
                    latest_user_message: request.latest_user_message,
                    latest_main_reply,
                    previous_handoff_task: request.previous_handoff_task,
                    mode: mode.as_refresh_mode(),
                });
        PlanningPostTurnQueueRefreshPreparation::Ready(Box::new(PlanningPreparedQueueRefresh::new(
            &request,
            latest_main_reply,
            mode,
            worker_prompt,
        )))
    }
    pub fn render_refresh_queue_prompt(&self, request: &PlanningQueueRefreshRequest<'_>) -> String {
        self.worker_orchestration
            .render_refresh_queue_prompt(request)
    }
    pub fn render_official_completion_refresh_prompt(
        &self,
        request: &PlanningOfficialCompletionRefreshRequest<'_>,
    ) -> String {
        self.worker_orchestration
            .render_official_completion_refresh_prompt(request)
    }
    pub fn refresh_queue_from_reply(
        &self,
        request: PlanningQueueRefreshRequest<'_>,
    ) -> anyhow::Result<PlanningWorkerRunOutcome> {
        // model reply는 orchestration을 통해 들어온다. extraction, validation, repair prompt, mutation commit이 한 경로에 남게 한다.
        self.worker_orchestration.refresh_queue_from_reply(request)
    }
    pub fn refresh_prepared_queue_from_reply(
        &self,
        prepared: &PlanningPreparedQueueRefresh,
    ) -> anyhow::Result<PlanningWorkerRunOutcome> {
        self.worker_orchestration
            .refresh_queue_from_reply(prepared.as_refresh_request())
    }
    pub fn finalize_post_turn_queue_refresh(
        &self,
        request: PlanningPostTurnQueueRefreshFinalizationRequest<'_>,
    ) -> PlanningPostTurnQueueRefreshFinalizationOutcome {
        let mut runtime_projection = request.refreshed_runtime_projection.clone();
        let mut events = Vec::new();
        if !runtime_projection.has_actionable_queue_head()
            && runtime_projection.has_proposal_candidates()
        {
            match self.promote_top_proposal_to_ready_if_needed(PlanningProposalPromotionRequest {
                workspace_directory: request.workspace_directory,
            }) {
                Ok(outcome) => {
                    runtime_projection = outcome.runtime_projection.clone();
                    events.push(
                        PlanningPostTurnQueueRefreshFinalizationEvent::ProposalPromotionCompleted {
                            outcome,
                        },
                    );
                }
                Err(error) => {
                    let detail = format!("host proposal promotion failed: {error}");
                    let invalid_projection = PlanningRuntimeProjection::invalid(
                        PLANNING_WORKER_REFRESH_FAILURE_BLOCK_REASON,
                    );
                    events.push(
                        PlanningPostTurnQueueRefreshFinalizationEvent::ProposalPromotionFailed {
                            detail,
                            runtime_projection: invalid_projection.clone(),
                        },
                    );
                    return PlanningPostTurnQueueRefreshFinalizationOutcome {
                        runtime_projection: invalid_projection,
                        events,
                    };
                }
            }
        }
        if !runtime_projection.has_actionable_queue_head()
            && !runtime_projection.has_proposal_candidates()
            && request.queue_idle_derivation
        {
            events.push(
                PlanningPostTurnQueueRefreshFinalizationEvent::QueueIdleDerivationEmpty {
                    detail:
                        "planning worker derived no justified follow-up task from the latest request and reply"
                            .to_string(),
                },
            );
        }
        if let Some(detail) = repeated_queue_head_detail(
            request.previous_handoff_task,
            request.previous_runtime_projection,
            &runtime_projection,
        ) {
            runtime_projection = runtime_projection.with_auto_follow_pause_reason(detail.clone());
            events.push(
                PlanningPostTurnQueueRefreshFinalizationEvent::RepeatedQueueHead {
                    detail,
                    runtime_projection: runtime_projection.clone(),
                },
            );
        }

        PlanningPostTurnQueueRefreshFinalizationOutcome {
            runtime_projection,
            events,
        }
    }
    pub fn refresh_queue_from_official_completion(
        &self,
        request: PlanningOfficialCompletionRefreshRequest<'_>,
    ) -> anyhow::Result<PlanningWorkerRunOutcome> {
        self.worker_orchestration
            .refresh_queue_from_official_completion(request)
    }
    pub fn prepare_post_turn_official_completion_refresh(
        &self,
        request: PlanningPostTurnOfficialCompletionPreparationRequest<'_>,
    ) -> PlanningPostTurnOfficialCompletionPreparation {
        let planning_workspace_projection =
            if request.planning_workspace_directory == request.turn_workspace_directory {
                request.current_runtime_projection.clone()
            } else {
                self.worker_orchestration
                    .load_runtime_projection_or_invalid(request.planning_workspace_directory)
            };
        if matches!(
            planning_workspace_projection.workspace_status(),
            PlanningRuntimeWorkspaceStatus::Invalid | PlanningRuntimeWorkspaceStatus::Uninitialized
        ) {
            let failure_detail = planning_workspace_projection
                .preview_detail()
                .unwrap_or(
                    "official completion refresh is blocked because the planning workspace is unavailable",
                )
                .to_string();
            let failure_projection = official_completion_failure_projection(
                &planning_workspace_projection,
                &failure_detail,
            );
            return PlanningPostTurnOfficialCompletionPreparation::Blocked(Box::new(
                PlanningPostTurnOfficialCompletionBlocked {
                    planning_workspace_projection,
                    failure_detail,
                    failure_projection,
                },
            ));
        }
        let latest_main_reply = request
            .latest_main_reply
            .map(str::trim)
            .filter(|message| !message.is_empty())
            .unwrap_or(request.contract.completion.final_response_summary.as_str());
        let worker_prompt = self
            .worker_orchestration
            .render_official_completion_refresh_prompt(&PlanningOfficialCompletionRefreshRequest {
                workspace_directory: request.planning_workspace_directory,
                parent_thread_id: request.parent_thread_id,
                latest_user_message: request.latest_user_message,
                latest_main_reply,
                previous_handoff_task: request.previous_handoff_task,
                contract: request.contract,
            });

        PlanningPostTurnOfficialCompletionPreparation::Ready(Box::new(
            PlanningPreparedOfficialCompletionRefresh::new(
                &request,
                planning_workspace_projection,
                latest_main_reply,
                worker_prompt,
            ),
        ))
    }
    pub fn refresh_prepared_official_completion(
        &self,
        prepared: &PlanningPreparedOfficialCompletionRefresh,
    ) -> anyhow::Result<PlanningWorkerRunOutcome> {
        self.worker_orchestration
            .refresh_queue_from_official_completion(prepared.as_refresh_request())
    }
    pub fn finalize_post_turn_official_completion_refresh(
        &self,
        request: PlanningPostTurnOfficialCompletionFinalizationRequest<'_>,
    ) -> PlanningPostTurnOfficialCompletionFinalizationOutcome {
        let mut runtime_projection = request.refreshed_runtime_projection.clone();
        let repeated_queue_head_detail = repeated_queue_head_detail(
            request.previous_handoff_task,
            request.previous_runtime_projection,
            &runtime_projection,
        );
        if let Some(detail) = repeated_queue_head_detail.as_ref() {
            runtime_projection = runtime_projection.with_auto_follow_pause_reason(detail.clone());
        }
        if runtime_projection.blocks_auto_follow() {
            let failure_detail = runtime_projection
                .preview_detail()
                .unwrap_or(OFFICIAL_COMPLETION_REFRESH_FAILURE_BLOCK_REASON)
                .to_string();
            let failure_projection =
                official_completion_failure_projection(&runtime_projection, &failure_detail);
            return PlanningPostTurnOfficialCompletionFinalizationOutcome {
                runtime_projection: failure_projection,
                repeated_queue_head_detail,
                blocked_failure_detail: Some(failure_detail),
                authority_refresh_outcome: None,
            };
        }
        let authority_refresh_outcome = request
            .worker_summary
            .map(|summary| format!("official ledger refresh succeeded: {summary}"))
            .unwrap_or_else(|| "official ledger refresh succeeded".to_string());
        PlanningPostTurnOfficialCompletionFinalizationOutcome {
            runtime_projection,
            repeated_queue_head_detail,
            blocked_failure_detail: None,
            authority_refresh_outcome: Some(authority_refresh_outcome),
        }
    }
    pub fn block_unresolved_post_turn_official_completion_repair(
        &self,
        request: PlanningPostTurnOfficialCompletionRepairBlockRequest<'_>,
    ) -> PlanningPostTurnOfficialCompletionRepairBlockOutcome {
        PlanningPostTurnOfficialCompletionRepairBlockOutcome {
            runtime_projection: official_completion_failure_projection(
                request.runtime_projection,
                OFFICIAL_COMPLETION_REFRESH_FAILURE_BLOCK_REASON,
            ),
            failure_detail: OFFICIAL_COMPLETION_REFRESH_FAILURE_BLOCK_REASON,
        }
    }
    pub fn render_repair_task_authority_prompt(
        &self,
        request: &PlanningLedgerRepairRequest<'_>,
    ) -> String {
        self.worker_orchestration
            .render_repair_task_authority_prompt(request)
    }
    pub fn repair_task_authority(
        &self,
        request: PlanningLedgerRepairRequest<'_>,
    ) -> anyhow::Result<PlanningWorkerRunOutcome> {
        self.worker_orchestration.repair_task_authority(request)
    }
    pub fn repair_post_turn_task_authority(
        &self,
        request: PlanningPostTurnRepairRequest<'_>,
    ) -> PlanningPostTurnRepairOutcome {
        let max_attempts = request.max_attempts.max(1);
        let mut runtime_projection = self
            .worker_orchestration
            .load_runtime_projection_or_invalid(request.workspace_directory);
        let mut next_request = request.repair_request.clone();
        let mut next_retry_reason = None;
        let mut attempts = Vec::new();

        for attempt_number in 1..=max_attempts {
            let attempt_retry_reason = next_retry_reason;
            let started_runtime_projection = runtime_projection.clone();
            let worker_request = PlanningLedgerRepairRequest {
                workspace_directory: request.workspace_directory,
                parent_thread_id: request.parent_thread_id,
                completed_turn_id: request.completed_turn_id,
                repair_request: &next_request,
                previous_handoff_task: request.previous_handoff_task,
                attempt_number,
                max_attempts,
                retry_reason: attempt_retry_reason,
            };
            let worker_prompt = self
                .worker_orchestration
                .render_repair_task_authority_prompt(&worker_request);
            let worker_outcome = self
                .worker_orchestration
                .repair_task_authority(worker_request);
            let result = match worker_outcome {
                Ok(outcome) => {
                    runtime_projection = outcome.runtime_projection.clone();
                    let next_repair_request = outcome.repair_request.clone();
                    let resolved = next_repair_request.is_none();
                    let exhausted = !resolved && attempt_number == max_attempts;
                    let next_reason = if resolved || exhausted {
                        None
                    } else if outcome.task_authority_changed {
                        Some(PlanningRepairRetryReason::TaskAuthorityStillInvalid)
                    } else {
                        Some(PlanningRepairRetryReason::TaskAuthorityUnchanged)
                    };
                    PlanningPostTurnRepairAttemptResult::WorkerSucceeded {
                        outcome: Box::new(outcome),
                        next_repair_request,
                        next_retry_reason: next_reason,
                        resolved,
                        exhausted,
                    }
                }
                Err(error) => {
                    let detail = format!(
                        "planning worker repair attempt {attempt_number}/{max_attempts} failed: {error}"
                    );
                    PlanningPostTurnRepairAttemptResult::WorkerFailed {
                        detail,
                        error: error.to_string(),
                    }
                }
            };
            let should_return = matches!(
                &result,
                PlanningPostTurnRepairAttemptResult::WorkerFailed { .. }
                    | PlanningPostTurnRepairAttemptResult::WorkerSucceeded { resolved: true, .. }
                    | PlanningPostTurnRepairAttemptResult::WorkerSucceeded {
                        exhausted: true,
                        ..
                    }
            );
            if let PlanningPostTurnRepairAttemptResult::WorkerSucceeded {
                next_repair_request: Some(repair_request),
                next_retry_reason: retry_reason_for_next_attempt,
                resolved: false,
                exhausted: false,
                ..
            } = &result
            {
                next_request = repair_request.clone();
                next_retry_reason = *retry_reason_for_next_attempt;
            }
            attempts.push(PlanningPostTurnRepairAttempt {
                attempt_number,
                max_attempts,
                retry_reason: attempt_retry_reason,
                started_runtime_projection,
                worker_prompt,
                result,
            });
            if should_return {
                let resolved = matches!(
                    attempts.last().map(|attempt| &attempt.result),
                    Some(PlanningPostTurnRepairAttemptResult::WorkerSucceeded {
                        resolved: true,
                        ..
                    })
                );
                return PlanningPostTurnRepairOutcome {
                    runtime_projection,
                    resolved,
                    attempts,
                };
            }
        }

        PlanningPostTurnRepairOutcome {
            runtime_projection,
            resolved: false,
            attempts,
        }
    }
    pub fn promote_top_proposal_to_ready_if_needed(
        &self,
        request: PlanningProposalPromotionRequest<'_>,
    ) -> anyhow::Result<PlanningProposalPromotionOutcome> {
        // promotion은 refresh/repair가 queue proposal을 만든 뒤 실행된다. deterministic 단계라 worker model에게 다시 묻지 않는다.
        self.proposal_promotion
            .promote_top_proposal_to_ready_if_needed(request)
    }
}

fn official_completion_failure_projection(
    current_projection: &PlanningRuntimeProjection,
    failure_detail: &str,
) -> PlanningRuntimeProjection {
    let detail = if failure_detail.trim().is_empty() {
        OFFICIAL_COMPLETION_REFRESH_FAILURE_BLOCK_REASON
    } else {
        failure_detail
    };
    current_projection.with_auto_follow_pause_reason(detail.to_string())
}

fn repeated_queue_head_detail(
    previous_handoff: Option<&PlanningTaskHandoff>,
    previous_projection: &PlanningRuntimeProjection,
    projection: &PlanningRuntimeProjection,
) -> Option<String> {
    let previous_handoff = previous_handoff?;
    let queue_head = projection.queue_head()?;
    if queue_head.task_id.trim() != previous_handoff.task_id.trim() {
        return None;
    }

    let unchanged = queue_head.task_title.trim() == previous_handoff.task_title.trim()
        && queue_head.direction_id.trim() == previous_handoff.direction_id.trim()
        && queue_head.combined_priority == previous_handoff.combined_priority
        && queue_head.updated_at.trim() == previous_handoff.updated_at.trim()
        && queue_head.status.label() == previous_handoff.status_label;
    if !unchanged {
        return None;
    }

    let queue_head_task_unchanged = match (
        previous_projection.queue_head_task_signature(),
        projection.queue_head_task_signature(),
    ) {
        (Some(previous), Some(current)) => previous == current,
        (None, None) => true,
        _ => false,
    };
    if !queue_head_task_unchanged {
        return None;
    }

    Some(format!(
        "planning worker refresh kept the previously handed-off task unchanged as the queue head; unrelated ledger edits do not count as queue advancement: {}",
        previous_handoff.task_title
    ))
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use anyhow::{Result, anyhow};

    use super::*;
    use crate::application::port::outbound::planning_authority_port::NoopPlanningAuthorityPort;
    use crate::application::port::outbound::planning_task_repository_port::NoopPlanningTaskRepositoryPort;
    use crate::application::port::outbound::planning_worker_port::NoopPlanningWorkerPort;
    use crate::application::port::outbound::planning_workspace_port::{
        PlanningDraftFileRecord, PlanningDraftLoadRecord, PlanningDraftStageRecord,
        PlanningWorkspaceLoadRecord, PlanningWorkspacePort,
    };
    use crate::application::service::planning::PlanningServices;
    use crate::application::service::planning::shared::contract::RESULT_OUTPUT_FILE_PATH;
    use crate::domain::planning::{PriorityQueueTask, TaskStatus};

    #[derive(Default)]
    struct ScriptedPlanningWorkspacePort {
        record: Mutex<PlanningWorkspaceLoadRecord>,
        load_error: Mutex<Option<String>>,
        commits: Mutex<Vec<PlanningWorkspaceLoadRecord>>,
    }

    impl ScriptedPlanningWorkspacePort {
        fn with_result_output(result_output_markdown: &str) -> Self {
            Self {
                record: Mutex::new(PlanningWorkspaceLoadRecord {
                    result_output_markdown: Some(result_output_markdown.to_string()),
                }),
                load_error: Mutex::new(None),
                commits: Mutex::new(Vec::new()),
            }
        }

        fn failing_load(message: &str) -> Self {
            Self {
                record: Mutex::new(PlanningWorkspaceLoadRecord::default()),
                load_error: Mutex::new(Some(message.to_string())),
                commits: Mutex::new(Vec::new()),
            }
        }

        fn commits(&self) -> Vec<PlanningWorkspaceLoadRecord> {
            self.commits
                .lock()
                .expect("workspace commit log should not be poisoned")
                .clone()
        }
    }

    impl PlanningWorkspacePort for ScriptedPlanningWorkspacePort {
        fn stage_planning_draft_files(
            &self,
            _workspace_dir: &str,
            _draft_name: &str,
            _files: &[PlanningDraftFileRecord],
        ) -> Result<PlanningDraftStageRecord> {
            Err(anyhow!(
                "stage_planning_draft_files should not be called by use-case tests"
            ))
        }

        fn load_planning_draft_files(
            &self,
            _workspace_dir: &str,
            _draft_name: &str,
        ) -> Result<PlanningDraftLoadRecord> {
            Err(anyhow!(
                "load_planning_draft_files should not be called by use-case tests"
            ))
        }

        fn replace_planning_draft_file(
            &self,
            _workspace_dir: &str,
            _draft_name: &str,
            _active_path: &str,
            _body: &str,
        ) -> Result<String> {
            Err(anyhow!(
                "replace_planning_draft_file should not be called by use-case tests"
            ))
        }

        fn load_planning_workspace_files(
            &self,
            _workspace_dir: &str,
        ) -> Result<PlanningWorkspaceLoadRecord> {
            if let Some(message) = self
                .load_error
                .lock()
                .expect("load error slot should not be poisoned")
                .clone()
            {
                return Err(anyhow!(message));
            }
            Ok(self
                .record
                .lock()
                .expect("workspace record should not be poisoned")
                .clone())
        }

        fn load_planning_workspace_candidate_files(
            &self,
            _workspace_dir: &str,
        ) -> Result<PlanningWorkspaceLoadRecord> {
            Err(anyhow!(
                "load_planning_workspace_candidate_files should not be called by use-case tests"
            ))
        }

        fn commit_planning_workspace_files(
            &self,
            _workspace_dir: &str,
            record: &PlanningWorkspaceLoadRecord,
        ) -> Result<()> {
            *self
                .record
                .lock()
                .expect("workspace record should not be poisoned") = record.clone();
            self.commits
                .lock()
                .expect("workspace commit log should not be poisoned")
                .push(record.clone());
            Ok(())
        }

        fn load_optional_planning_file(
            &self,
            _workspace_dir: &str,
            _relative_path: &str,
        ) -> Result<Option<String>> {
            Ok(None)
        }

        fn load_optional_planning_candidate_file(
            &self,
            _workspace_dir: &str,
            _relative_path: &str,
        ) -> Result<Option<String>> {
            Err(anyhow!(
                "load_optional_planning_candidate_file should not be called by use-case tests"
            ))
        }

        fn replace_planning_workspace_file(
            &self,
            _workspace_dir: &str,
            _relative_path: &str,
            _body: Option<&str>,
        ) -> Result<()> {
            Err(anyhow!(
                "replace_planning_workspace_file should not be called by use-case tests"
            ))
        }

        fn remove_planning_workspace_entry(
            &self,
            _workspace_dir: &str,
            _relative_path: &str,
        ) -> Result<()> {
            Err(anyhow!(
                "remove_planning_workspace_entry should not be called by use-case tests"
            ))
        }

        fn archive_rejected_planning_file(
            &self,
            _workspace_dir: &str,
            _archive_name: &str,
            _active_path: &str,
            _body: &str,
        ) -> Result<String> {
            Err(anyhow!(
                "archive_rejected_planning_file should not be called by use-case tests"
            ))
        }
    }

    #[test]
    fn capture_turn_execution_snapshot_maps_success_and_failure_to_stable_outcomes() {
        let planning =
            planning_services(Arc::new(ScriptedPlanningWorkspacePort::with_result_output(
                "# Result Output\n- Keep completion copy.",
            )));

        let capture = planning.runtime.capture_turn_execution_snapshot(
            PlanningTurnExecutionSnapshotCaptureRequest::new("/tmp/workspace"),
        );

        assert_eq!(capture.workspace_directory, "/tmp/workspace");
        assert_eq!(
            capture.state,
            PlanningTurnExecutionSnapshotCaptureState::Ready(PlanningExecutionSnapshot {
                result_output_markdown: Some(
                    "# Result Output\n- Keep completion copy.".to_string()
                )
            })
        );

        let planning = planning_services(Arc::new(ScriptedPlanningWorkspacePort::failing_load(
            "workspace unavailable",
        )));
        let capture = planning.runtime.capture_turn_execution_snapshot(
            PlanningTurnExecutionSnapshotCaptureRequest::new("/tmp/broken"),
        );

        assert_eq!(capture.workspace_directory, "/tmp/broken");
        assert!(matches!(
            capture.state,
            PlanningTurnExecutionSnapshotCaptureState::CaptureFailed(ref message)
                if message.contains("workspace unavailable")
        ));
    }

    #[test]
    fn reconcile_post_turn_blocks_without_matching_ready_execution_snapshot() {
        let planning =
            planning_services(Arc::new(ScriptedPlanningWorkspacePort::with_result_output(
                "# Result Output\n- Keep completion copy.",
            )));
        let current = PlanningRuntimeProjection::ready(
            "prompt".to_string(),
            "queue summary".to_string(),
            Some(sample_queue_head()),
        );
        let changed_paths = vec![RESULT_OUTPUT_FILE_PATH.to_string()];

        let missing_capture =
            planning
                .runtime
                .reconcile_post_turn(PlanningPostTurnReconciliationRequest {
                    workspace_directory: "/tmp/workspace",
                    completed_turn_id: "turn-1",
                    changed_planning_file_paths: &changed_paths,
                    execution_snapshot_capture: None,
                    current_runtime_projection: &current,
                });
        assert_eq!(
            missing_capture
                .reconciliation_result
                .auto_follow_block_reason
                .as_deref(),
            Some(
                "planning reconciliation could not restore protected planning files because the execution snapshot was unavailable"
            )
        );

        let stale_capture = PlanningTurnExecutionSnapshotCapture::ready(
            "/tmp/other",
            PlanningExecutionSnapshot {
                result_output_markdown: Some("old".to_string()),
            },
        );
        let stale = planning
            .runtime
            .reconcile_post_turn(PlanningPostTurnReconciliationRequest {
                workspace_directory: "/tmp/workspace",
                completed_turn_id: "turn-1",
                changed_planning_file_paths: &changed_paths,
                execution_snapshot_capture: Some(&stale_capture),
                current_runtime_projection: &current,
            });
        assert!(
            stale
                .reconciliation_result
                .auto_follow_block_reason
                .as_deref()
                .is_some_and(|message| message.contains("stale execution snapshot"))
        );

        let failed_capture = PlanningTurnExecutionSnapshotCapture::capture_failed(
            "/tmp/workspace",
            "capture failed before turn".to_string(),
        );
        let failed = planning
            .runtime
            .reconcile_post_turn(PlanningPostTurnReconciliationRequest {
                workspace_directory: "/tmp/workspace",
                completed_turn_id: "turn-1",
                changed_planning_file_paths: &changed_paths,
                execution_snapshot_capture: Some(&failed_capture),
                current_runtime_projection: &current,
            });
        assert_eq!(
            failed
                .reconciliation_result
                .auto_follow_block_reason
                .as_deref(),
            Some("capture failed before turn")
        );
    }

    #[test]
    fn reconcile_post_turn_restores_protected_files_and_preserves_current_projection_without_changes()
     {
        let workspace_port = Arc::new(ScriptedPlanningWorkspacePort::with_result_output(
            "# Result Output\n- Worker-edited copy.",
        ));
        let planning = planning_services(workspace_port.clone());
        let current = PlanningRuntimeProjection::ready(
            "prompt".to_string(),
            "queue summary".to_string(),
            Some(sample_queue_head()),
        );
        let unchanged =
            planning
                .runtime
                .reconcile_post_turn(PlanningPostTurnReconciliationRequest {
                    workspace_directory: "/tmp/workspace",
                    completed_turn_id: "turn-1",
                    changed_planning_file_paths: &[],
                    execution_snapshot_capture: None,
                    current_runtime_projection: &current,
                });
        assert_eq!(unchanged.runtime_projection, current);
        assert!(unchanged.reconciliation_result.notices.is_empty());

        let capture = PlanningTurnExecutionSnapshotCapture::ready(
            "/tmp/workspace",
            PlanningExecutionSnapshot {
                result_output_markdown: Some("# Result Output\n- Pre-turn copy.".to_string()),
            },
        );
        let changed_paths = vec![RESULT_OUTPUT_FILE_PATH.to_string()];
        let restored =
            planning
                .runtime
                .reconcile_post_turn(PlanningPostTurnReconciliationRequest {
                    workspace_directory: "/tmp/workspace",
                    completed_turn_id: "turn-1",
                    changed_planning_file_paths: &changed_paths,
                    execution_snapshot_capture: Some(&capture),
                    current_runtime_projection: &current,
                });

        assert!(restored.reconciliation_result.notices.iter().any(|notice| {
            notice == "planning reconciliation restored protected planning files"
        }));
        assert_eq!(workspace_port.commits().len(), 1);
        assert_eq!(
            workspace_port.commits()[0]
                .result_output_markdown
                .as_deref(),
            Some("# Result Output\n- Pre-turn copy.")
        );
    }

    #[test]
    fn post_turn_worker_panel_state_prioritizes_pause_repair_and_stop_policy() {
        let planning =
            planning_services(Arc::new(ScriptedPlanningWorkspacePort::with_result_output(
                "# Result Output\n- Keep completion copy.",
            )));
        let ready_with_task = PlanningRuntimeProjection::ready(
            "prompt".to_string(),
            "queue summary".to_string(),
            Some(sample_queue_head()),
        );
        let changed_paths = vec![RESULT_OUTPUT_FILE_PATH.to_string()];

        assert_eq!(
            planning.runtime.post_turn_worker_panel_start_state(
                PlanningPostTurnWorkerPanelStartRequest {
                    continuation_paused: true,
                    changed_planning_file_paths: &changed_paths,
                    current_runtime_projection: &ready_with_task,
                },
            ),
            PlanningPostTurnWorkerPanelStartState::PreserveCurrent
        );
        assert_eq!(
            planning.runtime.post_turn_worker_panel_start_state(
                PlanningPostTurnWorkerPanelStartRequest {
                    continuation_paused: false,
                    changed_planning_file_paths: &changed_paths,
                    current_runtime_projection: &ready_with_task,
                },
            ),
            PlanningPostTurnWorkerPanelStartState::RepairRunning
        );
        assert_eq!(
            planning.runtime.post_turn_worker_panel_start_state(
                PlanningPostTurnWorkerPanelStartRequest {
                    continuation_paused: false,
                    changed_planning_file_paths: &[],
                    current_runtime_projection: &PlanningRuntimeProjection::ready(
                        "prompt".to_string(),
                        "queue empty".to_string(),
                        None,
                    ),
                },
            ),
            PlanningPostTurnWorkerPanelStartState::PreserveCurrent
        );
        assert_eq!(
            planning.runtime.post_turn_worker_panel_start_state(
                PlanningPostTurnWorkerPanelStartRequest {
                    continuation_paused: false,
                    changed_planning_file_paths: &[],
                    current_runtime_projection: &ready_with_task,
                },
            ),
            PlanningPostTurnWorkerPanelStartState::RefreshRunning
        );
    }

    #[test]
    fn decide_post_turn_auto_follow_short_circuits_skips_and_builds_queue_prompt() {
        let planning =
            planning_services(Arc::new(ScriptedPlanningWorkspacePort::with_result_output(
                "# Result Output\n- Keep completion copy.",
            )));
        let ready_with_task = PlanningRuntimeProjection::ready(
            "prompt".to_string(),
            "queue summary".to_string(),
            Some(sample_queue_head()),
        );

        assert_eq!(
            decide_auto_follow_skip(&planning, &ready_with_task, |request| {
                request.continuation_paused = true;
            }),
            PlanningPostTurnAutoFollowSkipReason::PostTurnContinuationPaused
        );
        assert_eq!(
            decide_auto_follow_skip(
                &planning,
                &PlanningRuntimeProjection::ready(
                    "prompt".to_string(),
                    "queue empty".to_string(),
                    None,
                ),
                |_| {}
            ),
            PlanningPostTurnAutoFollowSkipReason::PlanningQueueDrained
        );
        assert_eq!(
            decide_auto_follow_skip(&planning, &ready_with_task, |request| {
                request.can_queue_next = false;
            }),
            PlanningPostTurnAutoFollowSkipReason::LimitReached
        );
        assert_eq!(
            decide_auto_follow_skip(&planning, &ready_with_task, |request| {
                request.latest_agent_message = Some("   ");
            }),
            PlanningPostTurnAutoFollowSkipReason::NoAgentReply
        );
        assert_eq!(
            decide_auto_follow_skip(&planning, &ready_with_task, |request| {
                request.stop_keyword_matched = true;
            }),
            PlanningPostTurnAutoFollowSkipReason::StopKeywordMatched
        );
        assert_eq!(
            decide_auto_follow_skip(&planning, &ready_with_task, |request| {
                request.no_file_changes_stop_matched = true;
            }),
            PlanningPostTurnAutoFollowSkipReason::NoFileChanges
        );

        let decision =
            planning
                .runtime
                .decide_post_turn_auto_follow(PlanningPostTurnAutoFollowRequest {
                    continuation_paused: false,
                    can_queue_next: true,
                    latest_agent_message: Some("completed"),
                    stop_keyword: "stop",
                    stop_keyword_matched: false,
                    no_file_changes_stop_matched: false,
                    runtime_projection: &ready_with_task,
                });

        let PlanningPostTurnAutoFollowDecision::QueuePrompt(prompt) = decision else {
            panic!("ready queue head should build an auto-follow prompt");
        };
        assert_eq!(
            prompt
                .handoff_task
                .as_ref()
                .map(|task| task.task_id.as_str()),
            Some("task-1")
        );
        assert!(prompt.prompt.contains("Queue head"));
    }

    #[test]
    fn prepare_post_turn_queue_refresh_skips_invalid_or_empty_reply_and_builds_ready_refresh() {
        let planning =
            planning_services(Arc::new(ScriptedPlanningWorkspacePort::with_result_output(
                "# Result Output\n- Keep completion copy.",
            )));
        let ready_with_task = PlanningRuntimeProjection::ready(
            "prompt".to_string(),
            "queue summary".to_string(),
            Some(sample_queue_head()),
        );

        let invalid = planning.worker.prepare_post_turn_queue_refresh(
            PlanningPostTurnQueueRefreshPreparationRequest {
                workspace_directory: "/tmp/workspace",
                parent_thread_id: Some("thread-1"),
                completed_turn_id: "turn-1",
                latest_user_message: Some("user"),
                latest_main_reply: Some("reply"),
                previous_handoff_task: None,
                current_runtime_projection: &PlanningRuntimeProjection::invalid("broken"),
            },
        );
        let PlanningPostTurnQueueRefreshPreparation::Skipped(skipped) = invalid else {
            panic!("invalid projection should skip queue refresh");
        };
        assert_eq!(
            skipped.reason,
            PlanningPostTurnQueueRefreshSkipReason::PlanningRuntimeNotReady
        );

        let empty = planning.worker.prepare_post_turn_queue_refresh(
            PlanningPostTurnQueueRefreshPreparationRequest {
                workspace_directory: "/tmp/workspace",
                parent_thread_id: Some("thread-1"),
                completed_turn_id: "turn-1",
                latest_user_message: Some("user"),
                latest_main_reply: Some("   "),
                previous_handoff_task: None,
                current_runtime_projection: &ready_with_task,
            },
        );
        let PlanningPostTurnQueueRefreshPreparation::Skipped(skipped) = empty else {
            panic!("blank reply should skip queue refresh");
        };
        assert_eq!(
            skipped.reason,
            PlanningPostTurnQueueRefreshSkipReason::LatestMainReplyEmpty
        );
        assert_eq!(skipped.reason.log_label(), "latest_main_reply_empty");

        let ready = planning.worker.prepare_post_turn_queue_refresh(
            PlanningPostTurnQueueRefreshPreparationRequest {
                workspace_directory: "/tmp/workspace",
                parent_thread_id: Some("thread-1"),
                completed_turn_id: "turn-1",
                latest_user_message: Some("user"),
                latest_main_reply: Some("  refreshed queue  "),
                previous_handoff_task: Some(&sample_handoff()),
                current_runtime_projection: &ready_with_task,
            },
        );
        let PlanningPostTurnQueueRefreshPreparation::Ready(prepared) = ready else {
            panic!("ready queue head should prepare worker refresh");
        };
        assert_eq!(prepared.mode_label(), "from_latest_main_reply");
        assert_eq!(prepared.panel_operation_label(), "refresh");
        assert_eq!(
            prepared.latest_main_reply_char_count(),
            "refreshed queue".chars().count()
        );
        assert!(prepared.has_latest_user_message());
        assert!(prepared.has_previous_handoff());
        assert!(!prepared.is_queue_idle_derivation());
        assert!(prepared.worker_prompt().contains("refreshed queue"));
    }

    fn sample_queue_head() -> PriorityQueueTask {
        PriorityQueueTask {
            rank: 1,
            task_id: "task-1".to_string(),
            direction_id: "direction-1".to_string(),
            direction_title: "Direction".to_string(),
            task_title: "Queue head".to_string(),
            status: TaskStatus::Ready,
            combined_priority: 80,
            updated_at: "2026-04-23T00:00:00Z".to_string(),
            rank_reasons: vec!["ready".to_string()],
        }
    }

    fn sample_handoff() -> PlanningTaskHandoff {
        PlanningTaskHandoff {
            task_id: "task-1".to_string(),
            task_title: "Queue head".to_string(),
            direction_id: "direction-1".to_string(),
            combined_priority: 80,
            updated_at: "2026-04-23T00:00:00Z".to_string(),
            status_label: "ready".to_string(),
        }
    }

    fn projection_with_signature(signature: Option<u64>) -> PlanningRuntimeProjection {
        PlanningRuntimeProjection::ready(
            "prompt".to_string(),
            "summary".to_string(),
            Some(sample_queue_head()),
        )
        .with_test_signatures(None, signature)
    }

    fn planning_services(workspace_port: Arc<dyn PlanningWorkspacePort>) -> PlanningServices {
        PlanningServices::from_ports(
            workspace_port,
            Arc::new(NoopPlanningAuthorityPort::default()),
            Arc::new(NoopPlanningTaskRepositoryPort),
            Arc::new(NoopPlanningWorkerPort),
        )
    }

    fn decide_auto_follow_skip(
        planning: &PlanningServices,
        projection: &PlanningRuntimeProjection,
        mutate: impl FnOnce(&mut PlanningPostTurnAutoFollowRequest<'_>),
    ) -> PlanningPostTurnAutoFollowSkipReason {
        let mut request = PlanningPostTurnAutoFollowRequest {
            continuation_paused: false,
            can_queue_next: true,
            latest_agent_message: Some("completed"),
            stop_keyword: "stop",
            stop_keyword_matched: false,
            no_file_changes_stop_matched: false,
            runtime_projection: projection,
        };
        mutate(&mut request);
        match planning.runtime.decide_post_turn_auto_follow(request) {
            PlanningPostTurnAutoFollowDecision::Skip(reason) => reason,
            PlanningPostTurnAutoFollowDecision::QueuePrompt(_) => {
                panic!("expected auto-follow decision to skip")
            }
        }
    }

    #[test]
    fn post_turn_repeated_queue_head_treats_missing_and_present_signatures_as_changed() {
        let detail = repeated_queue_head_detail(
            Some(&sample_handoff()),
            &projection_with_signature(None),
            &projection_with_signature(Some(7)),
        );

        assert!(detail.is_none());
    }

    #[test]
    fn post_turn_repeated_queue_head_accepts_both_missing_signatures_as_unchanged() {
        let detail = repeated_queue_head_detail(
            Some(&sample_handoff()),
            &projection_with_signature(None),
            &projection_with_signature(None),
        );

        assert!(detail.is_some());
    }
}
