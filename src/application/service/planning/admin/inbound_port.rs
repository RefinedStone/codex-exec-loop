use anyhow::Result;

use crate::application::port::inbound::planning_admin_port::{
    PlanningAdminCrudOutcome, PlanningAdminDirectionDeleteRequest,
    PlanningAdminDirectionMutationRequest, PlanningAdminDraftKind, PlanningAdminDraftLoadRequest,
    PlanningAdminDraftMutationRequest, PlanningAdminDraftPromotionOutcome,
    PlanningAdminFileSyncOutcome, PlanningAdminManagementView, PlanningAdminOverview,
    PlanningAdminPort, PlanningAdminResetOutcome, PlanningAdminRuntimeSummary,
    PlanningAdminSessionView, PlanningAdminTaskDeleteRequest, PlanningAdminTaskMutationRequest,
};
use crate::domain::planning::PlanningResetTarget;

use super::PlanningAdminFacadeService;

impl PlanningAdminPort for PlanningAdminFacadeService {
    fn workspace_dir(&self) -> &str {
        PlanningAdminFacadeService::workspace_dir(self)
    }

    fn load_overview(&self) -> Result<PlanningAdminOverview> {
        PlanningAdminFacadeService::load_overview(self)
    }

    fn load_runtime_summary(&self) -> Result<PlanningAdminRuntimeSummary> {
        PlanningAdminFacadeService::load_runtime_summary(self)
    }

    fn load_management_view(&self) -> Result<PlanningAdminManagementView> {
        PlanningAdminFacadeService::load_management_view(self)
    }

    fn upsert_direction(
        &self,
        request: PlanningAdminDirectionMutationRequest,
    ) -> Result<PlanningAdminCrudOutcome> {
        PlanningAdminFacadeService::upsert_direction(self, request)
    }

    fn delete_direction(
        &self,
        request: PlanningAdminDirectionDeleteRequest,
    ) -> Result<PlanningAdminCrudOutcome> {
        PlanningAdminFacadeService::delete_direction(self, request)
    }

    fn upsert_task(
        &self,
        request: PlanningAdminTaskMutationRequest,
    ) -> Result<PlanningAdminCrudOutcome> {
        PlanningAdminFacadeService::upsert_task(self, request)
    }

    fn delete_task(
        &self,
        request: PlanningAdminTaskDeleteRequest,
    ) -> Result<PlanningAdminCrudOutcome> {
        PlanningAdminFacadeService::delete_task(self, request)
    }

    fn create_draft_session(
        &self,
        kind: PlanningAdminDraftKind,
        direction_id: Option<&str>,
    ) -> Result<PlanningAdminSessionView> {
        PlanningAdminFacadeService::create_draft_session(self, kind, direction_id)
    }

    fn load_draft_session(
        &self,
        request: PlanningAdminDraftLoadRequest,
    ) -> Result<PlanningAdminSessionView> {
        PlanningAdminFacadeService::load_draft_session(self, request)
    }

    fn save_draft_session(
        &self,
        request: PlanningAdminDraftMutationRequest,
    ) -> Result<PlanningAdminSessionView> {
        PlanningAdminFacadeService::save_draft(self, request).map(|(_, session)| session)
    }

    fn promote_draft_session(
        &self,
        request: PlanningAdminDraftMutationRequest,
    ) -> Result<PlanningAdminDraftPromotionOutcome> {
        let (result, session) = PlanningAdminFacadeService::promote_draft(self, request)?;
        Ok(PlanningAdminDraftPromotionOutcome {
            promoted_file_count: result.promoted_file_count,
            is_valid: result.validation_report.is_valid(),
            session,
        })
    }

    fn export_active_files_for_edit(&self) -> Result<PlanningAdminFileSyncOutcome> {
        PlanningAdminFacadeService::export_active_files_for_edit(self)
    }

    fn apply_exported_files(&self) -> Result<PlanningAdminFileSyncOutcome> {
        PlanningAdminFacadeService::apply_exported_files(self)
    }

    fn reset_workspace(&self, target: PlanningResetTarget) -> Result<PlanningAdminResetOutcome> {
        PlanningAdminFacadeService::reset_workspace(self, target)
    }
}
