use std::sync::Arc;

use anyhow::{Result, anyhow};
use chrono::{SecondsFormat, Utc};

use crate::application::port::outbound::planning_task_repository_port::{
    PlanningTaskAuthorityCommit, PlanningTaskAuthorityCommitResult, PlanningTaskRepositoryPort,
};
use crate::application::port::outbound::planning_workspace_port::{
    PlanningWorkspaceLoadRecord, PlanningWorkspacePort,
};
use crate::application::service::planning::runtime::prompt::{
    PlanningPromptService, PlanningRuntimeSnapshot,
};
use crate::application::service::planning::runtime::validation::PlanningValidationService;
use crate::domain::planning::PriorityQueueService;
use crate::domain::planning::{
    DirectionCatalogDocument, PLANNING_FORMAT_VERSION, PlanningWorkspaceFiles, TaskActor,
    TaskAuthorityDocument, TaskStatus,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanningProposalPromotionRequest<'a> {
    pub workspace_directory: &'a str,
    pub root_turn_id: &'a str,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanningProposalPromotionOutcome {
    pub runtime_snapshot: PlanningRuntimeSnapshot,
    pub notices: Vec<String>,
    pub promoted_task_title: Option<String>,
    pub promoted: bool,
}

#[derive(Clone)]
pub struct PlanningProposalPromotionService {
    planning_workspace_port: Arc<dyn PlanningWorkspacePort>,
    planning_prompt_service: PlanningPromptService,
    planning_validation_service: PlanningValidationService,
    priority_queue_service: PriorityQueueService,
    planning_task_repository_port: Arc<dyn PlanningTaskRepositoryPort>,
}

impl PlanningProposalPromotionService {
    #[cfg(test)]
    #[allow(dead_code)]
    pub fn new(
        planning_workspace_port: Arc<dyn PlanningWorkspacePort>,
        planning_prompt_service: PlanningPromptService,
        planning_validation_service: PlanningValidationService,
        priority_queue_service: PriorityQueueService,
    ) -> Self {
        Self::with_task_repository(
            planning_workspace_port,
            planning_prompt_service,
            planning_validation_service,
            priority_queue_service,
            Arc::new(crate::application::port::outbound::planning_task_repository_port::NoopPlanningTaskRepositoryPort),
        )
    }

    pub fn with_task_repository(
        planning_workspace_port: Arc<dyn PlanningWorkspacePort>,
        planning_prompt_service: PlanningPromptService,
        planning_validation_service: PlanningValidationService,
        priority_queue_service: PriorityQueueService,
        planning_task_repository_port: Arc<dyn PlanningTaskRepositoryPort>,
    ) -> Self {
        Self {
            planning_workspace_port,
            planning_prompt_service,
            planning_validation_service,
            priority_queue_service,
            planning_task_repository_port,
        }
    }

    pub fn promote_top_proposal_to_ready_if_needed(
        &self,
        request: PlanningProposalPromotionRequest<'_>,
    ) -> Result<PlanningProposalPromotionOutcome> {
        let workspace_record = self
            .planning_workspace_port
            .load_planning_workspace_files(request.workspace_directory)?;
        let (directions, mut task_authority, observed_planning_revision) =
            self.load_valid_workspace_documents(request.workspace_directory, &workspace_record)?;
        let queue_projection = self
            .priority_queue_service
            .build_projection(&directions, &task_authority)?;
        if queue_projection.next_task.is_some() || queue_projection.proposed_tasks.is_empty() {
            return Ok(PlanningProposalPromotionOutcome {
                runtime_snapshot: self
                    .planning_prompt_service
                    .load_runtime_snapshot(request.workspace_directory)?,
                notices: Vec::new(),
                promoted_task_title: None,
                promoted: false,
            });
        }

        let top_proposal = queue_projection
            .proposed_tasks
            .into_iter()
            .next()
            .ok_or_else(|| anyhow!("proposal promotion requested without a promotable proposal"))?;
        let promoted_task = task_authority
            .tasks
            .iter_mut()
            .find(|task| task.id.trim() == top_proposal.task_id.trim())
            .ok_or_else(|| {
                anyhow!(
                    "top promotable proposal {} was not found in task authority",
                    top_proposal.task_id
                )
            })?;

        promoted_task.status = TaskStatus::Ready;
        promoted_task.last_updated_by = TaskActor::System;
        promoted_task.updated_at = Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true);

        let next_queue_projection = self
            .priority_queue_service
            .build_projection(&directions, &task_authority)?;
        match self
            .planning_task_repository_port
            .commit_task_authority_snapshot(
                request.workspace_directory,
                PlanningTaskAuthorityCommit {
                    observed_planning_revision: Some(observed_planning_revision),
                    task_authority: &task_authority,
                    queue_projection: &next_queue_projection,
                },
            )? {
            PlanningTaskAuthorityCommitResult::Committed { .. } => {}
            PlanningTaskAuthorityCommitResult::Conflict {
                observed_planning_revision,
                current_planning_revision,
            } => {
                anyhow::bail!(
                    "planning db changed while promoting proposal (observed revision {observed_planning_revision}, current revision {current_planning_revision}); reload and retry"
                );
            }
        }

        let runtime_snapshot = self
            .planning_prompt_service
            .load_runtime_snapshot(request.workspace_directory)?;

        let promoted_task_title = top_proposal.task_title.trim().to_string();
        let mut notices = Vec::new();
        notices.push(format!(
            "host promoted top follow-up proposal into the executable queue: {}",
            promoted_task_title
        ));

        Ok(PlanningProposalPromotionOutcome {
            runtime_snapshot,
            notices,
            promoted_task_title: Some(promoted_task_title),
            promoted: true,
        })
    }

    fn load_valid_workspace_documents(
        &self,
        workspace_dir: &str,
        workspace_record: &PlanningWorkspaceLoadRecord,
    ) -> Result<(DirectionCatalogDocument, TaskAuthorityDocument, i64)> {
        let snapshot = self
            .planning_task_repository_port
            .load_task_authority_snapshot(workspace_dir)?
            .ok_or_else(|| anyhow!("planning task authority is unavailable"))?;
        let directions_snapshot = self
            .planning_task_repository_port
            .load_direction_authority_snapshot(workspace_dir)?
            .ok_or_else(|| anyhow!("planning direction authority is unavailable"))?;
        let task_authority_json = serde_json::to_string(&snapshot.task_authority)?;
        let validation_result =
            self.planning_validation_service
                .validate_workspace_files(workspace_record_to_files(
                    workspace_record,
                    &directions_snapshot.directions,
                    &task_authority_json,
                )?);
        if !validation_result.is_valid() {
            let first_error = validation_result
                .report
                .errors()
                .first()
                .map(|issue| issue.message.clone())
                .unwrap_or_else(|| "planning validation failed".to_string());
            return Err(anyhow!(
                "cannot promote proposal from an invalid planning workspace: {first_error}"
            ));
        }

        let directions = validation_result
            .directions
            .ok_or_else(|| anyhow!("validated planning workspace did not include directions"))?;
        let task_authority = validation_result.task_authority.ok_or_else(|| {
            anyhow!("validated planning workspace did not include task-authority")
        })?;
        if task_authority.version != PLANNING_FORMAT_VERSION {
            return Err(anyhow!(
                "unsupported task-authority version {}; expected {}",
                task_authority.version,
                PLANNING_FORMAT_VERSION
            ));
        }

        Ok((directions, task_authority, snapshot.planning_revision))
    }
}

fn workspace_record_to_files<'a>(
    workspace_record: &'a PlanningWorkspaceLoadRecord,
    directions: &'a DirectionCatalogDocument,
    task_authority_json: &'a str,
) -> Result<PlanningWorkspaceFiles<'a>> {
    Ok(PlanningWorkspaceFiles {
        directions,
        task_authority_json,
        result_output_markdown: workspace_record
            .result_output_markdown
            .as_deref()
            .ok_or_else(|| anyhow!("planning workspace is missing result-output.md"))?,
    })
}
