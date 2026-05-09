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
use super::repair::reconciliation::{PlanningExecutionSnapshot, PlanningReconciliationResult};
use super::repair::reset::{
    PlanningResetService, PlanningResetTarget, PlanningWorkspaceResetResult,
};
use super::runtime::facade::{
    PlanningMainSessionHandoff, PlanningRuntimeAutoFollowDecision,
    PlanningRuntimeAutoFollowPreview, PlanningRuntimeAutoFollowPreviewRequest,
    PlanningRuntimeAutoFollowRequest, PlanningRuntimeFacadeService,
    PlanningRuntimeStatusProjection, PlanningRuntimeStatusProjectionRequest,
    PlanningRuntimeSummaryLineRequest, PlanningSubSessionHandoff,
};
use super::runtime::intake::{
    PlanningTaskIntakeCommitResult, PlanningTaskIntakeProposal, PlanningTaskIntakeRequest,
    PlanningTaskIntakeService,
};
use super::runtime::manual_intake::{
    ManualPromptIntakeOutcome, ManualPromptIntakeRequest, ManualPromptIntakeService,
};
use super::runtime::prompt::PlanningRuntimeSnapshot;
use super::task_tool::{
    PlanningTaskToolRequest, PlanningTaskToolResponse, PlanningTaskToolService,
    planning_task_tool_contract_json,
};
use super::worker::orchestration::{
    PlanningLedgerRepairRequest, PlanningOfficialCompletionRefreshRequest,
    PlanningQueueRefreshRequest, PlanningWorkerOrchestrationService, PlanningWorkerRunOutcome,
};
use crate::application::service::parallel_agent_persona::ParallelAgentPersona;
use crate::domain::planning::PriorityQueueTask;

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
    pub current_runtime_snapshot: &'a PlanningRuntimeSnapshot,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanningPostTurnReconciliationOutcome {
    pub reconciliation_result: PlanningReconciliationResult,
    pub runtime_snapshot: PlanningRuntimeSnapshot,
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
        _snapshot: &PlanningRuntimeSnapshot,
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
        snapshot: &PlanningRuntimeSnapshot,
    ) -> Option<PlanningMainSessionHandoff> {
        // queued-task handoff는 caller가 따로 들고 있는 queue state가 아니라 current runtime snapshot에서 파생한다.
        self.runtime_facade.build_queued_task_handoff(snapshot)
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
    pub fn load_runtime_snapshot_or_invalid(&self, workspace_dir: &str) -> PlanningRuntimeSnapshot {
        self.runtime_facade
            .load_runtime_snapshot_or_invalid(workspace_dir)
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
        let runtime_snapshot =
            if let Some(block_reason) = reconciliation_result.auto_follow_block_reason.clone() {
                PlanningRuntimeSnapshot::invalid(block_reason)
            } else if request.changed_planning_file_paths.is_empty() {
                request.current_runtime_snapshot.clone()
            } else {
                self.load_runtime_snapshot_or_invalid(request.workspace_directory)
            };
        PlanningPostTurnReconciliationOutcome {
            reconciliation_result,
            runtime_snapshot,
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
    pub fn refresh_queue_from_official_completion(
        &self,
        request: PlanningOfficialCompletionRefreshRequest<'_>,
    ) -> anyhow::Result<PlanningWorkerRunOutcome> {
        self.worker_orchestration
            .refresh_queue_from_official_completion(request)
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
    pub fn promote_top_proposal_to_ready_if_needed(
        &self,
        request: PlanningProposalPromotionRequest<'_>,
    ) -> anyhow::Result<PlanningProposalPromotionOutcome> {
        // promotion은 refresh/repair가 queue proposal을 만든 뒤 실행된다. deterministic 단계라 worker model에게 다시 묻지 않는다.
        self.proposal_promotion
            .promote_top_proposal_to_ready_if_needed(request)
    }
}
