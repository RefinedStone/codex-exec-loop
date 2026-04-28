use std::sync::Arc;

use anyhow::{Result, anyhow};

use crate::application::port::outbound::planning_task_repository_port::{
    PlanningDirectionAuthorityCommit, PlanningTaskAuthorityCommit, PlanningTaskRepositoryPort,
};
use crate::application::port::outbound::planning_workspace_port::{
    PlanningWorkspaceLoadRecord, PlanningWorkspacePort,
};
use crate::application::service::planning::authoring::bootstrap::{
    PlanningBootstrapArtifacts, PlanningBootstrapMode, PlanningBootstrapService,
};
use crate::application::service::planning::runtime::validation::PlanningValidationService;
use crate::application::service::planning::shared::contract::{
    DEFAULT_QUEUE_IDLE_PROMPT_FILE_PATH, PLANNING_DIRECTION_DOCS_DIRECTORY,
    PLANNING_PROMPTS_DIRECTORY, RESULT_OUTPUT_FILE_PATH,
};
use crate::domain::planning::PriorityQueueService;
use crate::domain::planning::{DirectionCatalogDocument, TaskAuthorityDocument, TaskStatus};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlanningResetTarget {
    Queue,
    Directions,
    All,
}

impl PlanningResetTarget {
    pub fn label(self) -> &'static str {
        match self {
            Self::Queue => "queue",
            Self::Directions => "directions",
            Self::All => "all",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanningWorkspaceResetResult {
    pub target: PlanningResetTarget,
    pub rewritten_paths: Vec<String>,
    pub removed_paths: Vec<String>,
}

#[derive(Clone)]
pub struct PlanningResetService {
    planning_workspace_port: Arc<dyn PlanningWorkspacePort>,
    planning_bootstrap_service: PlanningBootstrapService,
    planning_task_repository_port: Arc<dyn PlanningTaskRepositoryPort>,
    planning_validation_service: PlanningValidationService,
    priority_queue_service: PriorityQueueService,
}

impl PlanningResetService {
    #[cfg(test)]
    #[allow(dead_code)]
    pub fn new(
        planning_workspace_port: Arc<dyn PlanningWorkspacePort>,
        planning_bootstrap_service: PlanningBootstrapService,
    ) -> Self {
        Self::with_task_repository(
            planning_workspace_port,
            planning_bootstrap_service,
            Arc::new(crate::application::port::outbound::planning_task_repository_port::NoopPlanningTaskRepositoryPort),
            PlanningValidationService::new(),
            PriorityQueueService::new(),
        )
    }

    pub fn with_task_repository(
        planning_workspace_port: Arc<dyn PlanningWorkspacePort>,
        planning_bootstrap_service: PlanningBootstrapService,
        planning_task_repository_port: Arc<dyn PlanningTaskRepositoryPort>,
        planning_validation_service: PlanningValidationService,
        priority_queue_service: PriorityQueueService,
    ) -> Self {
        Self {
            planning_workspace_port,
            planning_bootstrap_service,
            planning_task_repository_port,
            planning_validation_service,
            priority_queue_service,
        }
    }

    pub fn reset_workspace(
        &self,
        workspace_dir: &str,
        target: PlanningResetTarget,
    ) -> Result<PlanningWorkspaceResetResult> {
        let workspace = self.load_existing_workspace(workspace_dir)?;
        let bootstrap = self
            .planning_bootstrap_service
            .build_artifacts_for_mode(PlanningBootstrapMode::Simple);

        match target {
            PlanningResetTarget::Queue => self.reset_queue(workspace_dir, &workspace, &bootstrap),
            PlanningResetTarget::Directions => {
                self.ensure_directions_reset_is_safe(workspace_dir)?;
                self.reset_directions(workspace_dir, &workspace, &bootstrap)
            }
            PlanningResetTarget::All => self.reset_all(workspace_dir, &bootstrap),
        }
    }

    fn load_existing_workspace(&self, workspace_dir: &str) -> Result<PlanningWorkspaceLoadRecord> {
        let workspace = self
            .planning_workspace_port
            .load_planning_workspace_files(workspace_dir)?;
        if workspace.has_any_files() {
            Ok(workspace)
        } else {
            Err(anyhow!(
                "planning workspace is unavailable; initialize planning first"
            ))
        }
    }

    fn reset_queue(
        &self,
        workspace_dir: &str,
        workspace: &PlanningWorkspaceLoadRecord,
        bootstrap: &PlanningBootstrapArtifacts,
    ) -> Result<PlanningWorkspaceResetResult> {
        self.commit_task_authority_from_document(
            workspace_dir,
            None,
            &bootstrap.task_authority,
            workspace.result_output_markdown.as_deref(),
        )?;

        Ok(PlanningWorkspaceResetResult {
            target: PlanningResetTarget::Queue,
            rewritten_paths: Vec::new(),
            removed_paths: Vec::new(),
        })
    }

    fn ensure_directions_reset_is_safe(&self, workspace_dir: &str) -> Result<()> {
        let task_authority = self
            .planning_task_repository_port
            .load_task_authority_snapshot(workspace_dir)?
            .map(|snapshot| snapshot.task_authority)
            .unwrap_or_else(|| TaskAuthorityDocument {
                version: crate::domain::planning::PLANNING_FORMAT_VERSION,
                tasks: Vec::new(),
            });
        let live_tasks = task_authority
            .tasks
            .iter()
            .filter(|task| !matches!(task.status, TaskStatus::Done | TaskStatus::Cancelled))
            .map(|task| format!("{}({})", task.id.trim(), task.status.label()))
            .collect::<Vec<_>>();
        if live_tasks.is_empty() {
            return Ok(());
        }

        let live_task_summary = live_tasks
            .iter()
            .take(3)
            .cloned()
            .collect::<Vec<_>>()
            .join(", ");
        let extra_count = live_tasks.len().saturating_sub(3);
        let suffix = if extra_count == 0 {
            String::new()
        } else {
            format!(" (+{extra_count} more)")
        };
        Err(anyhow!(
            "planning directions reset is blocked by live tasks: {live_task_summary}{suffix}; use reset all to replace the full workspace instead"
        ))
    }

    fn reset_directions(
        &self,
        workspace_dir: &str,
        workspace: &PlanningWorkspaceLoadRecord,
        bootstrap: &PlanningBootstrapArtifacts,
    ) -> Result<PlanningWorkspaceResetResult> {
        self.reset_directions_side_artifacts(workspace_dir, bootstrap)?;
        let task_authority = self
            .planning_task_repository_port
            .load_task_authority_snapshot(workspace_dir)?
            .map(|snapshot| snapshot.task_authority)
            .unwrap_or_else(|| bootstrap.task_authority.clone());
        self.commit_task_authority_from_document(
            workspace_dir,
            Some(&bootstrap.directions),
            &task_authority,
            workspace.result_output_markdown.as_deref(),
        )?;

        Ok(PlanningWorkspaceResetResult {
            target: PlanningResetTarget::Directions,
            rewritten_paths: vec![DEFAULT_QUEUE_IDLE_PROMPT_FILE_PATH.to_string()],
            removed_paths: vec![
                PLANNING_DIRECTION_DOCS_DIRECTORY.to_string(),
                PLANNING_PROMPTS_DIRECTORY.to_string(),
            ],
        })
    }

    fn reset_all(
        &self,
        workspace_dir: &str,
        bootstrap: &PlanningBootstrapArtifacts,
    ) -> Result<PlanningWorkspaceResetResult> {
        self.reset_directions_side_artifacts(workspace_dir, bootstrap)?;
        self.planning_workspace_port
            .replace_planning_workspace_file(
                workspace_dir,
                RESULT_OUTPUT_FILE_PATH,
                Some(&bootstrap.result_output_markdown),
            )?;
        self.commit_task_authority_from_document(
            workspace_dir,
            Some(&bootstrap.directions),
            &bootstrap.task_authority,
            Some(&bootstrap.result_output_markdown),
        )?;

        Ok(PlanningWorkspaceResetResult {
            target: PlanningResetTarget::All,
            rewritten_paths: vec![
                RESULT_OUTPUT_FILE_PATH.to_string(),
                DEFAULT_QUEUE_IDLE_PROMPT_FILE_PATH.to_string(),
            ],
            removed_paths: vec![
                PLANNING_DIRECTION_DOCS_DIRECTORY.to_string(),
                PLANNING_PROMPTS_DIRECTORY.to_string(),
            ],
        })
    }

    fn reset_directions_side_artifacts(
        &self,
        workspace_dir: &str,
        bootstrap: &PlanningBootstrapArtifacts,
    ) -> Result<()> {
        self.planning_workspace_port
            .remove_planning_workspace_entry(workspace_dir, PLANNING_DIRECTION_DOCS_DIRECTORY)?;
        self.planning_workspace_port
            .remove_planning_workspace_entry(workspace_dir, PLANNING_PROMPTS_DIRECTORY)?;
        self.commit_direction_authority_from_bootstrap(workspace_dir, &bootstrap.directions)?;
        for supplemental_file in &bootstrap.supplemental_files {
            self.planning_workspace_port
                .replace_planning_workspace_file(
                    workspace_dir,
                    &supplemental_file.active_path,
                    Some(&supplemental_file.body),
                )?;
        }
        Ok(())
    }

    fn commit_task_authority_from_document(
        &self,
        workspace_dir: &str,
        directions: Option<&DirectionCatalogDocument>,
        task_authority: &TaskAuthorityDocument,
        result_output_markdown: Option<&str>,
    ) -> Result<()> {
        let loaded_directions;
        let directions = match directions {
            Some(directions) => Some(directions),
            None => {
                loaded_directions = self
                    .planning_task_repository_port
                    .load_direction_authority_snapshot(workspace_dir)?
                    .map(|snapshot| snapshot.directions);
                loaded_directions.as_ref()
            }
        };
        let (Some(directions), Some(result_output_markdown)) = (directions, result_output_markdown)
        else {
            return self
                .planning_task_repository_port
                .clear_task_authority_snapshot(workspace_dir);
        };
        let task_authority_json = serde_json::to_string(task_authority)?;
        let validation_result = self.planning_validation_service.validate_workspace_files(
            crate::domain::planning::PlanningWorkspaceFiles {
                directions,
                task_authority_json: &task_authority_json,
                result_output_markdown,
            },
        );
        if !validation_result.is_valid() {
            return Ok(());
        }
        let directions = validation_result
            .directions
            .as_ref()
            .ok_or_else(|| anyhow!("valid reset workspace did not include directions"))?;
        let task_authority = validation_result
            .task_authority
            .as_ref()
            .ok_or_else(|| anyhow!("valid reset workspace did not include task-authority"))?;
        let queue_projection = self
            .priority_queue_service
            .build_projection(directions, task_authority)
            .map_err(|error| anyhow!("valid reset queue build failed: {error}"))?;
        self.planning_task_repository_port
            .commit_task_authority_snapshot(
                workspace_dir,
                PlanningTaskAuthorityCommit {
                    observed_planning_revision: None,
                    task_authority,
                    queue_projection: &queue_projection,
                },
            )
            .map(|_| ())
    }

    fn commit_direction_authority_from_bootstrap(
        &self,
        workspace_dir: &str,
        directions: &DirectionCatalogDocument,
    ) -> Result<()> {
        self.planning_task_repository_port
            .commit_direction_authority_snapshot(
                workspace_dir,
                PlanningDirectionAuthorityCommit {
                    observed_planning_revision: None,
                    directions,
                },
            )
            .map(|_| ())
    }
}

#[cfg(test)]
mod tests {
    use super::PlanningResetTarget;

    #[test]
    fn reset_target_values_still_exist() {
        assert!(matches!(
            PlanningResetTarget::Queue,
            PlanningResetTarget::Queue
        ));
        assert!(matches!(
            PlanningResetTarget::Directions,
            PlanningResetTarget::Directions
        ));
        assert!(matches!(PlanningResetTarget::All, PlanningResetTarget::All));
    }
}
