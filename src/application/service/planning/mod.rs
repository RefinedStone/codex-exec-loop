use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

pub mod admin;
pub(crate) mod authoring;
pub mod control;
pub(crate) mod repair;
pub(crate) mod runtime;
pub(crate) mod shared;
pub(crate) mod worker;

use crate::application::port::outbound::planning_authority_port::{
    PlanningAuthorityDistributorQueueRecord, PlanningAuthorityOfficialRefreshClaimStatus,
    PlanningAuthorityPort, PlanningAuthorityRuntimeProjectionSnapshot,
};
use crate::application::port::outbound::planning_task_repository_port::{
    NoopPlanningTaskRepositoryPort, PlanningTaskRepositoryPort,
};
use crate::application::port::outbound::planning_worker_port::{
    PlanningWorkerPort, PlanningWorkerRequest, PlanningWorkerResponse,
};
use crate::application::port::outbound::planning_workspace_port::PlanningWorkspacePort;
use crate::domain::planning::{
    PlanningAuthorityLocation, PlanningAuthorityShadowStoreInspection,
    PlanningAuthorityShadowStoreSyncState,
};

use self::authoring::bootstrap::PlanningBootstrapService;
use self::authoring::directions::PlanningDirectionsService;
use self::authoring::directions_apply::PlanningDirectionsApplyService;
use self::authoring::init::PlanningInitService;
use self::authoring::proposal_promotion::PlanningProposalPromotionService;
use self::authoring::task_ledger_apply::PlanningTaskLedgerApplyService;
use self::repair::doctor::PlanningDoctorService;
use self::repair::reconciliation::PlanningReconciliationService;
use self::repair::reset::PlanningResetService;
use self::runtime::facade::PlanningRuntimeFacadeService;
use self::runtime::intake::PlanningTaskIntakeService;
use self::runtime::policy::PlanningRuntimePolicyService;
use self::runtime::prompt::PlanningPromptService;
use self::runtime::validation::PlanningValidationService;
use self::worker::orchestration::PlanningWorkerOrchestrationService;
use super::priority_queue_service::PriorityQueueService;
use super::turn_prompt_assembly_service::TurnPromptAssemblyService;

pub use self::PlanningFeature as PlanningServices;
pub use self::admin::{
    PlanningAdminDraftFileUpdate, PlanningAdminDraftKind, PlanningAdminDraftLoadRequest,
    PlanningAdminDraftMutationRequest, PlanningAdminFacadeService, PlanningAdminFileKey,
    PlanningAdminOverview, PlanningAdminResetOutcome, PlanningAdminSessionView,
};
pub use self::authoring::bootstrap::PlanningBootstrapMode;
pub use self::authoring::directions::{
    DirectionsMaintenanceDirectionSummary, DirectionsMaintenanceSummary,
    DirectionsSupportingFileStatus, PlanningDoctorOutcome, QueueIdleReviewContext,
};
pub use self::authoring::directions_apply::PlanningTrackedDirectionsApplyResult;
pub use self::authoring::init::{
    PlanningDraftEditorFile, PlanningDraftEditorSession, PlanningDraftPromoteResult,
    PlanningDraftSaveResult, PlanningInitStageResult, PlanningWorkspaceInitResult,
};
pub use self::authoring::proposal_promotion::{
    PlanningProposalPromotionOutcome, PlanningProposalPromotionRequest,
};
pub use self::authoring::task_ledger_apply::PlanningTrackedTaskLedgerApplyResult;
pub use self::control::{PlanningControlCommand, PlanningControlReply, PlanningControlService};
pub use self::repair::doctor::{PlanningDoctorReport, PlanningDoctorState};
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
pub use self::runtime::intake::{
    LocalPromptTaskDraftGenerator, PlanningTaskDraftGenerator, PlanningTaskIntakeCommitResult,
    PlanningTaskIntakeDraft, PlanningTaskIntakeProposal, PlanningTaskIntakeRequest,
    PlanningTaskIntakeValidationError, PlanningTaskIntakeValidationService,
};
pub use self::runtime::policy::PlanningAutoFollowBlockReason;
pub use self::runtime::prompt::{PlanningRuntimeSnapshot, PlanningRuntimeWorkspaceStatus};
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
        planning_authority_port: Arc<dyn PlanningAuthorityPort>,
        planning_task_repository_port: Arc<dyn PlanningTaskRepositoryPort>,
        planning_worker_port: Arc<dyn PlanningWorkerPort>,
    ) -> Self {
        let validation_service = PlanningValidationService::new();
        let priority_queue_service = PriorityQueueService::new();
        let init_service = PlanningInitService::with_task_repository(
            planning_workspace_port.clone(),
            PlanningBootstrapService::new(),
            validation_service.clone(),
            planning_task_repository_port.clone(),
            priority_queue_service.clone(),
        );
        let reset_service = PlanningResetService::with_task_repository(
            planning_workspace_port.clone(),
            PlanningBootstrapService::new(),
            planning_task_repository_port.clone(),
            validation_service.clone(),
            priority_queue_service.clone(),
        );
        let directions_service = PlanningDirectionsService::new(
            planning_workspace_port.clone(),
            validation_service.clone(),
        );
        let directions_apply_service = PlanningDirectionsApplyService::with_task_repository(
            planning_workspace_port.clone(),
            validation_service.clone(),
            priority_queue_service.clone(),
            planning_task_repository_port.clone(),
        );
        let task_ledger_apply_service = PlanningTaskLedgerApplyService::with_task_repository(
            planning_workspace_port.clone(),
            validation_service.clone(),
            priority_queue_service.clone(),
            planning_task_repository_port.clone(),
        );
        let planning_prompt_service = PlanningPromptService::with_task_repository(
            planning_workspace_port.clone(),
            validation_service.clone(),
            priority_queue_service.clone(),
            planning_task_repository_port.clone(),
        );
        let doctor_service = PlanningDoctorService::new(planning_prompt_service.clone());
        let planning_reconciliation_service = PlanningReconciliationService::with_task_repository(
            planning_workspace_port.clone(),
            validation_service.clone(),
            priority_queue_service.clone(),
            planning_task_repository_port.clone(),
        );
        let runtime_facade = PlanningRuntimeFacadeService::new(
            planning_prompt_service.clone(),
            planning_reconciliation_service.clone(),
            PlanningRuntimePolicyService::new(),
            TurnPromptAssemblyService::new(),
        );
        let task_intake = PlanningTaskIntakeService::new(
            planning_workspace_port.clone(),
            planning_task_repository_port.clone(),
            validation_service.clone(),
            priority_queue_service.clone(),
        );
        let proposal_promotion = PlanningProposalPromotionService::with_task_repository(
            planning_workspace_port,
            planning_prompt_service,
            validation_service,
            priority_queue_service,
            planning_task_repository_port,
        );

        Self {
            workspace: PlanningWorkspaceUseCases::new(
                init_service,
                reset_service,
                doctor_service,
                directions_service.clone(),
                directions_apply_service,
                task_ledger_apply_service,
            ),
            runtime: PlanningRuntimeUseCases::new(runtime_facade.clone(), task_intake),
            worker: PlanningWorkerUseCases::new(
                directions_service,
                PlanningWorkerOrchestrationService::new(
                    planning_worker_port,
                    runtime_facade,
                    planning_authority_port,
                ),
                proposal_promotion,
            ),
        }
    }

    pub fn from_workspace_port(planning_workspace_port: Arc<dyn PlanningWorkspacePort>) -> Self {
        Self::from_ports(
            planning_workspace_port,
            Arc::new(NoopPlanningAuthorityPort::default()),
            Arc::new(NoopPlanningTaskRepositoryPort),
            Arc::new(NoopPlanningWorkerPort),
        )
    }
}

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
    fn new(
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

#[derive(Default)]
struct NoopPlanningAuthorityPort {
    next_refresh_order: AtomicU64,
}

impl PlanningAuthorityPort for NoopPlanningAuthorityPort {
    fn resolve_authority_location(
        &self,
        workspace_dir: &str,
    ) -> anyhow::Result<PlanningAuthorityLocation> {
        Ok(PlanningAuthorityLocation {
            workspace_root: workspace_dir.to_string(),
            canonical_repo_root: workspace_dir.to_string(),
            runtime_dir: String::new(),
            authority_store_path: String::new(),
        })
    }

    fn inspect_shadow_store(
        &self,
        workspace_dir: &str,
    ) -> anyhow::Result<PlanningAuthorityShadowStoreInspection> {
        Ok(PlanningAuthorityShadowStoreInspection {
            location: self.resolve_authority_location(workspace_dir)?,
            sync_state: PlanningAuthorityShadowStoreSyncState::InSync,
            mirrored_document_count: 0,
            parity_issue_count: 0,
            parity_issue_examples: Vec::new(),
        })
    }

    fn reserve_next_official_refresh_order(&self, _workspace_dir: &str) -> anyhow::Result<u64> {
        Ok(self.next_refresh_order.fetch_add(1, Ordering::Relaxed) + 1)
    }

    fn acquire_official_refresh_claim(
        &self,
        _workspace_dir: &str,
        _refresh_order: u64,
        _owner_token: &str,
    ) -> anyhow::Result<PlanningAuthorityOfficialRefreshClaimStatus> {
        Ok(PlanningAuthorityOfficialRefreshClaimStatus::Acquired)
    }

    fn release_official_refresh_claim(
        &self,
        _workspace_dir: &str,
        _refresh_order: u64,
        _owner_token: &str,
    ) -> anyhow::Result<()> {
        Ok(())
    }

    fn try_acquire_distributor_queue_claim(
        &self,
        _workspace_dir: &str,
        _queue_item_id: &str,
        _owner_token: &str,
    ) -> anyhow::Result<bool> {
        Ok(true)
    }

    fn release_distributor_queue_claim(
        &self,
        _workspace_dir: &str,
        _queue_item_id: &str,
        _owner_token: &str,
    ) -> anyhow::Result<()> {
        Ok(())
    }

    fn load_runtime_projections(
        &self,
        _workspace_dir: &str,
    ) -> anyhow::Result<PlanningAuthorityRuntimeProjectionSnapshot> {
        Ok(PlanningAuthorityRuntimeProjectionSnapshot::default())
    }

    fn upsert_runtime_slot_lease(
        &self,
        _workspace_dir: &str,
        _lease: &crate::domain::parallel_mode::ParallelModeSlotLeaseSnapshot,
    ) -> anyhow::Result<()> {
        Ok(())
    }

    fn remove_runtime_slot_lease(
        &self,
        _workspace_dir: &str,
        _slot_id: &str,
    ) -> anyhow::Result<()> {
        Ok(())
    }

    fn upsert_runtime_session_detail(
        &self,
        _workspace_dir: &str,
        _detail: &crate::domain::parallel_mode::ParallelModeAgentSessionDetailSnapshot,
    ) -> anyhow::Result<()> {
        Ok(())
    }

    fn upsert_runtime_distributor_queue_record(
        &self,
        _workspace_dir: &str,
        _record: &PlanningAuthorityDistributorQueueRecord,
    ) -> anyhow::Result<()> {
        Ok(())
    }
}
