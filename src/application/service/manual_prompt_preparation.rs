use crate::application::service::planning::{
    ManualPromptIntakeRequest, PlanningRuntimeProjection, PlanningServices,
};
use crate::domain::planning::{
    ManualPlanningBootstrapFailureKind as DomainManualPlanningBootstrapFailureKind,
    ManualPlanningBootstrapReview as DomainManualPlanningBootstrapReview,
    ManualPromptOutcome as DomainManualPromptOutcome,
    ManualPromptRequest as DomainManualPromptRequest,
};

pub type ManualPromptPreparationRequest = DomainManualPromptRequest;
pub type ManualPlanningBootstrapReview = DomainManualPlanningBootstrapReview;
pub type ManualPlanningBootstrapFailureKind = DomainManualPlanningBootstrapFailureKind;
pub type ManualPromptPreparationResult = DomainManualPromptOutcome;

#[derive(Clone)]
pub struct ManualPromptPreparationService {
    planning: PlanningServices,
}

impl ManualPromptPreparationService {
    pub fn new(planning: PlanningServices) -> Self {
        Self { planning }
    }

    pub fn prepare(
        &self,
        request: ManualPromptPreparationRequest,
    ) -> ManualPromptPreparationResult {
        let transcript_text = request.raw_prompt.trim().to_string();
        let mut runtime_projection = self
            .planning
            .runtime
            .load_runtime_projection_or_invalid(&request.workspace_directory);
        if transcript_text.is_empty() {
            return ManualPromptPreparationResult::Rejected {
                transcript_text,
                runtime_projection: Box::new(runtime_projection),
                reason: "manual prompt is empty".to_string(),
            };
        }

        if !runtime_projection.workspace_present() {
            runtime_projection = match self.ensure_manual_planning_workspace(
                &request.workspace_directory,
                &transcript_text,
                runtime_projection,
            ) {
                ManualWorkspacePreparation::Ready(runtime_projection) => *runtime_projection,
                ManualWorkspacePreparation::Blocked(result) => return result,
            };
        }

        let intake =
            self.planning
                .runtime
                .prepare_manual_prompt_intake(ManualPromptIntakeRequest {
                    workspace_directory: request.workspace_directory,
                    raw_prompt: transcript_text.clone(),
                    legacy_source_turn_id: None,
                    parent_thread_id: request.parent_thread_id,
                    parent_turn_id: request.parent_turn_id,
                });
        ManualPromptPreparationResult::PromptReady {
            transcript_text,
            runtime_projection: Box::new(runtime_projection),
            intake: Box::new(intake),
        }
    }

    fn ensure_manual_planning_workspace(
        &self,
        workspace_directory: &str,
        transcript_text: &str,
        initial_projection: PlanningRuntimeProjection,
    ) -> ManualWorkspacePreparation {
        let stage_result = match self
            .planning
            .workspace
            .stage_simple_mode_draft(workspace_directory)
        {
            Ok(stage_result) => stage_result,
            Err(error) => {
                return ManualWorkspacePreparation::Blocked(
                    ManualPromptPreparationResult::BootstrapFailed {
                        transcript_text: transcript_text.to_string(),
                        runtime_projection: Box::new(initial_projection),
                        kind: ManualPlanningBootstrapFailureKind::Stage,
                        reason: error.to_string(),
                    },
                );
            }
        };
        let promote_result = match self
            .planning
            .workspace
            .promote_staged_draft(workspace_directory, &stage_result.draft_name)
        {
            Ok(promote_result) => promote_result,
            Err(error) => {
                return ManualWorkspacePreparation::Blocked(
                    ManualPromptPreparationResult::BootstrapFailed {
                        transcript_text: transcript_text.to_string(),
                        runtime_projection: Box::new(initial_projection),
                        kind: ManualPlanningBootstrapFailureKind::Promote,
                        reason: error.to_string(),
                    },
                );
            }
        };
        let runtime_projection = self
            .planning
            .runtime
            .load_runtime_projection_or_invalid(workspace_directory);
        if promote_result.promoted_file_count > 0 {
            return ManualWorkspacePreparation::Ready(Box::new(runtime_projection));
        }

        ManualWorkspacePreparation::Blocked(
            ManualPromptPreparationResult::BootstrapReviewRequired {
                transcript_text: transcript_text.to_string(),
                runtime_projection: Box::new(runtime_projection),
                review: ManualPlanningBootstrapReview {
                    draft_name: stage_result.draft_name,
                    staged_file_count: stage_result.staged_file_count,
                    validation_report: stage_result.validation_report,
                },
            },
        )
    }
}

enum ManualWorkspacePreparation {
    Ready(Box<PlanningRuntimeProjection>),
    Blocked(ManualPromptPreparationResult),
}
