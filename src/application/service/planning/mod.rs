use std::sync::Arc;

pub mod admin;
pub(crate) mod authoring;
pub(crate) mod repair;
pub(crate) mod runtime;
pub(crate) mod shared;
pub(crate) mod worker;

use crate::application::port::outbound::planning_worker_port::{
    PlanningWorkerPort, PlanningWorkerRequest, PlanningWorkerResponse,
};
use crate::application::port::outbound::planning_workspace_port::PlanningWorkspacePort;

use super::priority_queue_service::PriorityQueueService;
use super::turn_prompt_assembly_service::TurnPromptAssemblyService;
use self::authoring::bootstrap::PlanningBootstrapService;
use self::authoring::directions::PlanningDirectionsService;
use self::authoring::init::PlanningInitService;
use self::authoring::proposal_promotion::PlanningProposalPromotionService;
use self::repair::doctor::PlanningDoctorService;
use self::repair::reconciliation::PlanningReconciliationService;
use self::repair::reset::PlanningResetService;
use self::runtime::facade::PlanningRuntimeFacadeService;
use self::runtime::policy::PlanningRuntimePolicyService;
use self::runtime::prompt::PlanningPromptService;
use self::runtime::validation::PlanningValidationService;
use self::worker::orchestration::PlanningWorkerOrchestrationService;

pub use self::PlanningFeature as PlanningServices;
pub use self::admin::{
    PlanningAdminDraftFileUpdate, PlanningAdminDraftKind, PlanningAdminDraftLoadRequest,
    PlanningAdminDraftMutationRequest, PlanningAdminFacadeService, PlanningAdminFileKey,
    PlanningAdminOverview, PlanningAdminResetOutcome, PlanningAdminSessionView,
    PlanningAdminTogglePlanOutcome,
};
pub use self::authoring::bootstrap::PlanningBootstrapMode;
pub use self::authoring::directions::{
    DirectionsMaintenanceDirectionSummary, DirectionsMaintenanceSummary,
    DirectionsSupportingFileStatus, PlanningDoctorOutcome, QueueIdleReviewContext,
};
pub use self::repair::doctor::{PlanningDoctorReport, PlanningDoctorState};
pub use self::authoring::init::{
    PlanningDraftEditorFile, PlanningDraftEditorSession, PlanningDraftPromoteResult,
    PlanningDraftSaveResult, PlanningInitStageResult, PlanningWorkspaceInitResult,
};
pub use self::runtime::prompt::{PlanningRuntimeSnapshot, PlanningRuntimeWorkspaceStatus};
pub use self::authoring::proposal_promotion::{
    PlanningProposalPromotionOutcome, PlanningProposalPromotionRequest,
};
pub use self::repair::reconciliation::{
    PlanningExecutionSnapshot, PlanningProtectedFileRestoration, PlanningQueueSnapshotAction,
    PlanningReconciliationResult, PlanningRepairRequest, PlanningRepairRetryReason,
};
pub use self::repair::reset::{PlanningResetTarget, PlanningWorkspaceResetResult};
pub use self::runtime::facade::{
    PlanningMainSessionHandoff, PlanningRuntimeAutoFollowDecision,
    PlanningRuntimeAutoFollowRequest, PlanningRuntimePreviewRequest,
    PlanningRuntimeQueuedAutoFollowPrompt, PlanningRuntimeRenderedPreview,
    PlanningRuntimeRepairAttempt, PlanningRuntimeStatusProjection,
    PlanningRuntimeStatusProjectionRequest, PlanningRuntimeSummaryLineRequest,
    PlanningRuntimeSummaryRequest, PlanningTaskHandoff,
};
pub use self::runtime::policy::PlanningAutoFollowBlockReason;
pub use self::shared::auto_follow_copy::BUILTIN_NEXT_TASK_TRANSCRIPT_TEXT;
pub use self::worker::orchestration::{
    PlanningLedgerRepairRequest, PlanningOfficialCompletionRefreshRequest,
    PlanningQueueRefreshMode, PlanningQueueRefreshRequest, PlanningWorkerRunOutcome,
};
pub use crate::domain::planning::{
    PlanningOfficialCompletionRefreshContract, PlanningOfficialCompletionRefreshPayload,
};

#[derive(Clone)]
pub struct PlanningFeature {
    pub workspace: PlanningWorkspaceUseCases,
    pub runtime: PlanningRuntimeUseCases,
    pub worker: PlanningWorkerUseCases,
}

impl PlanningFeature {
    pub fn from_ports(
        planning_workspace_port: Arc<dyn PlanningWorkspacePort>,
        planning_worker_port: Arc<dyn PlanningWorkerPort>,
    ) -> Self {
        let validation_service = PlanningValidationService::new();
        let priority_queue_service = PriorityQueueService::new();
        let init_service = PlanningInitService::new(
            planning_workspace_port.clone(),
            PlanningBootstrapService::new(),
            validation_service.clone(),
        );
        let reset_service = PlanningResetService::new(
            planning_workspace_port.clone(),
            PlanningBootstrapService::new(),
        );
        let directions_service = PlanningDirectionsService::new(
            planning_workspace_port.clone(),
            validation_service.clone(),
        );
        let planning_prompt_service = PlanningPromptService::new(
            planning_workspace_port.clone(),
            validation_service.clone(),
            priority_queue_service.clone(),
        );
        let doctor_service = PlanningDoctorService::new(planning_prompt_service.clone());
        let planning_reconciliation_service = PlanningReconciliationService::new(
            planning_workspace_port.clone(),
            validation_service.clone(),
            priority_queue_service.clone(),
        );
        let runtime_facade = PlanningRuntimeFacadeService::new(
            planning_prompt_service.clone(),
            planning_reconciliation_service.clone(),
            PlanningRuntimePolicyService::new(),
            TurnPromptAssemblyService::new(),
        );
        let proposal_promotion = PlanningProposalPromotionService::new(
            planning_workspace_port,
            planning_prompt_service,
            validation_service,
            priority_queue_service,
        );

        Self {
            workspace: PlanningWorkspaceUseCases::new(
                init_service,
                reset_service,
                doctor_service,
                directions_service.clone(),
            ),
            runtime: PlanningRuntimeUseCases::new(runtime_facade.clone()),
            worker: PlanningWorkerUseCases::new(
                directions_service,
                PlanningWorkerOrchestrationService::new(planning_worker_port, runtime_facade),
                proposal_promotion,
            ),
        }
    }

    pub fn from_workspace_port(planning_workspace_port: Arc<dyn PlanningWorkspacePort>) -> Self {
        Self::from_ports(planning_workspace_port, Arc::new(NoopPlanningWorkerPort))
    }
}

#[derive(Clone)]
pub struct PlanningWorkspaceUseCases {
    init_service: PlanningInitService,
    reset_service: PlanningResetService,
    doctor_service: PlanningDoctorService,
    directions_service: PlanningDirectionsService,
}

impl PlanningWorkspaceUseCases {
    fn new(
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

    pub fn set_plan_enabled(&self, workspace_dir: &str, enabled: bool) -> anyhow::Result<()> {
        self.init_service.set_plan_enabled(workspace_dir, enabled)
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
}

impl PlanningRuntimeUseCases {
    pub(crate) fn new(runtime_facade: PlanningRuntimeFacadeService) -> Self {
        Self { runtime_facade }
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
    fn new(
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

struct NoopPlanningWorkerPort;

impl PlanningWorkerPort for NoopPlanningWorkerPort {
    fn run_planning_session(
        &self,
        request: PlanningWorkerRequest,
    ) -> anyhow::Result<PlanningWorkerResponse> {
        Ok(PlanningWorkerResponse {
            operation: request.operation,
            final_agent_message: Some("planner worker disabled".to_string()),
            changed_planning_file_paths: Vec::new(),
        })
    }
}
