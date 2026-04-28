use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use anyhow::{Context, Result, anyhow};

#[cfg(test)]
use crate::application::port::outbound::planning_task_repository_port::NoopPlanningTaskRepositoryPort;
use crate::application::port::outbound::planning_task_repository_port::{
    PlanningTaskAuthorityCommit, PlanningTaskAuthorityCommitResult, PlanningTaskRepositoryPort,
};
use crate::application::port::outbound::planning_workspace_port::{
    PlanningWorkspaceLoadRecord, PlanningWorkspacePort,
};
use crate::application::service::planning::runtime::validation::PlanningValidationService;
use crate::application::service::planning::shared::contract::DIRECTIONS_FILE_PATH;
use crate::domain::planning::PriorityQueueService;
use crate::domain::planning::{
    DirectionCatalogDocument, PlanningValidationReport, PlanningWorkspaceFiles,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanningTrackedDirectionsApplyResult {
    pub applied_paths: Vec<String>,
    pub validation_report: PlanningValidationReport,
}

impl PlanningTrackedDirectionsApplyResult {
    pub fn applied(&self) -> bool {
        !self.applied_paths.is_empty() && self.validation_report.is_valid()
    }
}

#[derive(Clone)]
pub struct PlanningDirectionsApplyService {
    planning_workspace_port: Arc<dyn PlanningWorkspacePort>,
    planning_validation_service: PlanningValidationService,
    priority_queue_service: PriorityQueueService,
    planning_task_repository_port: Arc<dyn PlanningTaskRepositoryPort>,
}

impl PlanningDirectionsApplyService {
    #[cfg(test)]
    #[allow(dead_code)]
    pub fn new(
        planning_workspace_port: Arc<dyn PlanningWorkspacePort>,
        planning_validation_service: PlanningValidationService,
    ) -> Self {
        Self::with_task_repository(
            planning_workspace_port,
            planning_validation_service,
            PriorityQueueService::new(),
            Arc::new(NoopPlanningTaskRepositoryPort),
        )
    }

    pub fn with_task_repository(
        planning_workspace_port: Arc<dyn PlanningWorkspacePort>,
        planning_validation_service: PlanningValidationService,
        priority_queue_service: PriorityQueueService,
        planning_task_repository_port: Arc<dyn PlanningTaskRepositoryPort>,
    ) -> Self {
        Self {
            planning_workspace_port,
            planning_validation_service,
            priority_queue_service,
            planning_task_repository_port,
        }
    }

    pub fn apply_tracked_directions(
        &self,
        workspace_dir: &str,
    ) -> Result<PlanningTrackedDirectionsApplyResult> {
        let active_workspace = self.load_active_workspace(workspace_dir)?;
        let candidate_directions_toml = self.load_candidate_directions_toml(workspace_dir)?;
        let authority_snapshot = self
            .planning_task_repository_port
            .load_task_authority_snapshot(workspace_dir)?
            .ok_or_else(|| {
                anyhow!(
                    "planning task authority is unavailable; initialize or repair the planning database before applying tracked directions"
                )
            })?;
        let task_authority_json = serde_json::to_string_pretty(&authority_snapshot.task_authority)
            .context("failed to serialize task authority ledger")?;
        let mut validation_result =
            self.planning_validation_service
                .validate_workspace_files(PlanningWorkspaceFiles {
                    directions_toml: &candidate_directions_toml,
                    task_authority_json: &task_authority_json,
                    result_output_markdown: required_workspace_body(
                        &active_workspace,
                        WorkspaceBody::ResultOutput,
                    )?,
                });

        let candidate_supporting_files =
            if let Some(directions) = validation_result.directions.as_ref() {
                let candidate_supporting_files =
                    self.load_candidate_supporting_files(workspace_dir, directions);
                self.planning_validation_service
                    .validate_direction_supporting_files(
                        directions,
                        |path| candidate_supporting_files.contains_key(path),
                        &mut validation_result.report,
                    );
                candidate_supporting_files
            } else {
                BTreeMap::new()
            };

        if !validation_result.is_valid() {
            return Ok(PlanningTrackedDirectionsApplyResult {
                applied_paths: Vec::new(),
                validation_report: validation_result.report,
            });
        }
        let directions = validation_result
            .directions
            .as_ref()
            .expect("valid tracked directions should include directions");
        let task_authority = validation_result
            .task_authority
            .as_ref()
            .expect("valid tracked directions should include task ledger");
        let queue_projection = self
            .priority_queue_service
            .build_projection(directions, task_authority)
            .context("failed to rebuild planning queue after tracked directions apply")?;

        match self
            .planning_task_repository_port
            .commit_task_authority_snapshot(
                workspace_dir,
                PlanningTaskAuthorityCommit {
                    observed_planning_revision: Some(authority_snapshot.planning_revision),
                    task_authority,
                    queue_projection: &queue_projection,
                },
            )? {
            PlanningTaskAuthorityCommitResult::Committed { .. } => {}
            PlanningTaskAuthorityCommitResult::Conflict {
                observed_planning_revision,
                current_planning_revision,
            } => {
                return Err(anyhow!(
                    "planning db changed while applying tracked directions (observed revision {observed_planning_revision}, current revision {current_planning_revision}); reload and retry"
                ));
            }
        }

        self.planning_workspace_port
            .replace_planning_workspace_file(
                workspace_dir,
                DIRECTIONS_FILE_PATH,
                Some(&candidate_directions_toml),
            )?;

        let mut applied_paths = vec![DIRECTIONS_FILE_PATH.to_string()];
        for (path, body) in candidate_supporting_files {
            self.planning_workspace_port
                .replace_planning_workspace_file(workspace_dir, &path, Some(&body))?;
            applied_paths.push(path);
        }
        applied_paths.sort();
        applied_paths.dedup();

        Ok(PlanningTrackedDirectionsApplyResult {
            applied_paths,
            validation_report: validation_result.report,
        })
    }

    fn load_active_workspace(&self, workspace_dir: &str) -> Result<PlanningWorkspaceLoadRecord> {
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

    fn load_candidate_directions_toml(&self, workspace_dir: &str) -> Result<String> {
        let candidate_workspace = self
            .planning_workspace_port
            .load_planning_workspace_candidate_files(workspace_dir)?;
        candidate_workspace.directions_toml.ok_or_else(|| {
            anyhow!(
                "tracked directions import requires .codex-exec-loop/planning/directions.toml in the workspace root"
            )
        })
    }

    fn load_candidate_supporting_files(
        &self,
        workspace_dir: &str,
        directions: &DirectionCatalogDocument,
    ) -> BTreeMap<String, String> {
        candidate_supporting_paths(directions)
            .into_iter()
            .filter_map(|path| {
                match self
                    .planning_workspace_port
                    .load_optional_planning_candidate_file(workspace_dir, &path)
                {
                    Ok(Some(body)) => Some((path, body)),
                    Ok(None) | Err(_) => None,
                }
            })
            .collect()
    }
}

#[derive(Clone, Copy)]
enum WorkspaceBody {
    ResultOutput,
}

fn required_workspace_body(
    workspace: &PlanningWorkspaceLoadRecord,
    body: WorkspaceBody,
) -> Result<&str> {
    match body {
        WorkspaceBody::ResultOutput => workspace
            .result_output_markdown
            .as_deref()
            .ok_or_else(|| anyhow!("planning workspace is missing result-output.md")),
    }
}

fn candidate_supporting_paths(directions: &DirectionCatalogDocument) -> BTreeSet<String> {
    let mut paths = BTreeSet::new();
    let prompt_path = directions.queue_idle.prompt_path.trim();
    if !prompt_path.is_empty() {
        paths.insert(prompt_path.to_string());
    }
    paths.extend(
        directions
            .directions
            .iter()
            .map(|direction| direction.detail_doc_path.trim())
            .filter(|path| !path.is_empty())
            .map(str::to_string),
    );
    paths
}

#[cfg(test)]
mod tests {
    #[test]
    fn test_module_compiles_after_task_authority_file_removal() {
        assert!(std::env::current_dir().is_ok());
    }
}
