use super::authoring::directions::PlanningDirectionsService;
use super::authoring::directions::{DirectionsMaintenanceSummary, QueueIdleReviewContext};
use super::authoring::directions_apply::{
    PlanningDirectionsApplyService, PlanningTrackedDirectionsApplyResult,
};
use super::authoring::init::{
    PlanningDraftEditorFile, PlanningDraftEditorSession, PlanningDraftPromoteResult,
    PlanningDraftSaveResult, PlanningInitService, PlanningInitStageResult,
    PlanningWorkspaceInitResult,
};
use super::authoring::proposal_promotion::{
    PlanningProposalPromotionOutcome, PlanningProposalPromotionRequest,
    PlanningProposalPromotionService,
};
use super::authoring::task_ledger_apply::{
    PlanningTaskLedgerApplyService, PlanningTrackedTaskLedgerApplyResult,
};
use super::repair::doctor::{PlanningDoctorReport, PlanningDoctorService};
use super::repair::reconciliation::{PlanningExecutionSnapshot, PlanningReconciliationResult};
use super::repair::reset::{
    PlanningResetService, PlanningResetTarget, PlanningWorkspaceResetResult,
};
use super::runtime::facade::{
    PlanningMainSessionHandoff, PlanningRuntimeAutoFollowDecision,
    PlanningRuntimeAutoFollowRequest, PlanningRuntimeFacadeService, PlanningRuntimePreviewRequest,
    PlanningRuntimeRenderedPreview, PlanningRuntimeStatusProjection,
    PlanningRuntimeStatusProjectionRequest, PlanningRuntimeSummaryLineRequest,
};
use super::runtime::intake::{
    PlanningTaskIntakeCommitResult, PlanningTaskIntakeProposal, PlanningTaskIntakeRequest,
    PlanningTaskIntakeService,
};
use super::runtime::prompt::PlanningRuntimeSnapshot;
use super::worker::orchestration::{
    PlanningLedgerRepairRequest, PlanningOfficialCompletionRefreshRequest,
    PlanningQueueRefreshRequest, PlanningWorkerOrchestrationService, PlanningWorkerRunOutcome,
};

#[derive(Clone)]
pub struct PlanningWorkspaceUseCases {
    init_service: PlanningInitService,
    reset_service: PlanningResetService,
    doctor_service: PlanningDoctorService,
    directions_service: PlanningDirectionsService,
    directions_apply_service: PlanningDirectionsApplyService,
    task_ledger_apply_service: PlanningTaskLedgerApplyService,
}

impl PlanningWorkspaceUseCases {
    pub(super) fn new(
        init_service: PlanningInitService,
        reset_service: PlanningResetService,
        doctor_service: PlanningDoctorService,
        directions_service: PlanningDirectionsService,
        directions_apply_service: PlanningDirectionsApplyService,
        task_ledger_apply_service: PlanningTaskLedgerApplyService,
    ) -> Self {
        Self {
            init_service,
            reset_service,
            doctor_service,
            directions_service,
            directions_apply_service,
            task_ledger_apply_service,
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

    pub fn apply_tracked_directions(
        &self,
        workspace_dir: &str,
    ) -> anyhow::Result<PlanningTrackedDirectionsApplyResult> {
        self.directions_apply_service
            .apply_tracked_directions(workspace_dir)
    }

    pub fn apply_tracked_task_ledger(
        &self,
        workspace_dir: &str,
    ) -> anyhow::Result<PlanningTrackedTaskLedgerApplyResult> {
        self.task_ledger_apply_service
            .apply_tracked_task_ledger(workspace_dir)
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
        self.directions_service.load_summary(workspace_dir)
    }

    pub fn load_queue_idle_review_context(
        &self,
        workspace_dir: &str,
    ) -> anyhow::Result<QueueIdleReviewContext> {
        self.directions_service
            .load_queue_idle_review_context(workspace_dir)
    }

    pub fn stage_editor_session(
        &self,
        workspace_dir: &str,
    ) -> anyhow::Result<PlanningDraftEditorSession> {
        self.directions_service.stage_editor_session(workspace_dir)
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
    runtime_facade: PlanningRuntimeFacadeService,
    task_intake: PlanningTaskIntakeService,
}

impl PlanningRuntimeUseCases {
    pub(crate) fn new(
        runtime_facade: PlanningRuntimeFacadeService,
        task_intake: PlanningTaskIntakeService,
    ) -> Self {
        Self {
            runtime_facade,
            task_intake,
        }
    }

    pub fn build_manual_prompt(
        &self,
        operator_prompt: &str,
        snapshot: &PlanningRuntimeSnapshot,
    ) -> Option<String> {
        self.runtime_facade
            .build_manual_prompt(operator_prompt, snapshot)
    }

    pub fn build_builtin_next_task_handoff(
        &self,
        snapshot: &PlanningRuntimeSnapshot,
    ) -> Option<PlanningMainSessionHandoff> {
        self.runtime_facade
            .build_builtin_next_task_handoff(snapshot)
    }

    pub fn decide_auto_followup(
        &self,
        request: PlanningRuntimeAutoFollowRequest<'_>,
    ) -> PlanningRuntimeAutoFollowDecision {
        self.runtime_facade.decide_auto_followup(request)
    }

    pub fn build_auto_follow_preview(
        &self,
        request: PlanningRuntimePreviewRequest<'_>,
    ) -> PlanningRuntimeRenderedPreview {
        self.runtime_facade.build_auto_follow_preview(request)
    }

    pub fn build_summary_line(
        &self,
        request: PlanningRuntimeSummaryLineRequest<'_>,
    ) -> Option<String> {
        self.runtime_facade.build_summary_line(request)
    }

    pub fn build_followup_status_projection(
        &self,
        request: PlanningRuntimeStatusProjectionRequest<'_>,
    ) -> PlanningRuntimeStatusProjection {
        self.runtime_facade
            .build_followup_status_projection(request)
    }

    pub fn load_runtime_snapshot_or_invalid(&self, workspace_dir: &str) -> PlanningRuntimeSnapshot {
        self.runtime_facade
            .load_runtime_snapshot_or_invalid(workspace_dir)
    }

    pub fn prepare_task_intake(
        &self,
        request: PlanningTaskIntakeRequest,
    ) -> anyhow::Result<PlanningTaskIntakeProposal> {
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

    pub fn reconcile_after_turn(
        &self,
        workspace_dir: &str,
        turn_id: &str,
        changed_planning_file_paths: &[String],
        execution_snapshot: &PlanningExecutionSnapshot,
    ) -> anyhow::Result<PlanningReconciliationResult> {
        self.runtime_facade.reconcile_after_turn(
            workspace_dir,
            turn_id,
            changed_planning_file_paths,
            execution_snapshot,
        )
    }
}

#[derive(Clone)]
pub struct PlanningWorkerUseCases {
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
        self.worker_orchestration.refresh_queue_from_reply(request)
    }

    pub fn refresh_queue_from_official_completion(
        &self,
        request: PlanningOfficialCompletionRefreshRequest<'_>,
    ) -> anyhow::Result<PlanningWorkerRunOutcome> {
        self.worker_orchestration
            .refresh_queue_from_official_completion(request)
    }

    pub fn render_repair_task_ledger_prompt(
        &self,
        request: &PlanningLedgerRepairRequest<'_>,
    ) -> String {
        self.worker_orchestration
            .render_repair_task_ledger_prompt(request)
    }

    pub fn repair_task_ledger(
        &self,
        request: PlanningLedgerRepairRequest<'_>,
    ) -> anyhow::Result<PlanningWorkerRunOutcome> {
        self.worker_orchestration.repair_task_ledger(request)
    }

    pub fn promote_top_proposal_to_ready_if_needed(
        &self,
        request: PlanningProposalPromotionRequest<'_>,
    ) -> anyhow::Result<PlanningProposalPromotionOutcome> {
        self.proposal_promotion
            .promote_top_proposal_to_ready_if_needed(request)
    }
}
