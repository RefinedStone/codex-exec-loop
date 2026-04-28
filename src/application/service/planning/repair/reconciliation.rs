use std::sync::Arc;

use anyhow::Result;

use crate::application::port::outbound::planning_task_repository_port::{
    PlanningTaskAuthorityCommit, PlanningTaskAuthorityCommitResult, PlanningTaskRepositoryPort,
};
use crate::application::port::outbound::planning_workspace_port::PlanningWorkspaceLoadRecord;
use crate::application::port::outbound::planning_workspace_port::PlanningWorkspacePort;
use crate::application::service::planning::shared::contract::{
    DIRECTIONS_FILE_PATH, RESULT_OUTPUT_FILE_PATH, canonical_active_planning_file_path,
};
use crate::domain::planning::PlanningWorkspaceFiles;
use crate::domain::planning::PriorityQueueService;

pub use super::ledger_recovery::PlanningQueueProjectionAction;
pub use super::prompt::{
    PlanningRepairPromptHandoff, PlanningRepairRetryReason, build_planning_repair_prompt,
};
pub use super::protected_restore::PlanningProtectedFileRestoration;
use crate::application::service::planning::runtime::validation::PlanningValidationService;

#[derive(Clone)]
pub struct PlanningReconciliationService {
    planning_workspace_port: Arc<dyn PlanningWorkspacePort>,
    planning_validation_service: PlanningValidationService,
    priority_queue_service: PriorityQueueService,
    planning_task_repository_port: Arc<dyn PlanningTaskRepositoryPort>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PlanningExecutionSnapshot {
    pub directions_toml: Option<String>,
    pub result_output_markdown: Option<String>,
}

impl PlanningExecutionSnapshot {
    pub fn captures_path(path: &str) -> bool {
        canonical_active_planning_file_path(path).is_some()
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PlanningReconciliationResult {
    pub notices: Vec<String>,
    pub restored_protected_files: Vec<PlanningProtectedFileRestoration>,
    pub rejected_task_authority: bool,
    pub rejected_archive_path: Option<String>,
    pub queue_projection_action: Option<PlanningQueueProjectionAction>,
    pub repair_request: Option<PlanningRepairRequest>,
    pub auto_followup_block_reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanningRepairRequest {
    pub failure_summary: String,
    pub validation_errors: Vec<String>,
    pub directions_toml: String,
    pub accepted_task_authority_json: String,
    pub rejected_task_authority_json: Option<String>,
    pub rejected_archive_path: Option<String>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(super) struct PlanningChangeSet {
    pub(super) directions_changed: bool,
    pub(super) result_output_changed: bool,
}

impl PlanningChangeSet {
    fn from_paths(paths: &[String]) -> Self {
        let mut change_set = Self::default();
        for path in paths {
            match canonical_active_planning_file_path(path) {
                Some(DIRECTIONS_FILE_PATH) => change_set.directions_changed = true,
                Some(RESULT_OUTPUT_FILE_PATH) => change_set.result_output_changed = true,
                _ => {}
            }
        }
        change_set
    }

    fn has_relevant_changes(self) -> bool {
        self.directions_changed || self.result_output_changed
    }
}

impl PlanningReconciliationService {
    #[cfg(test)]
    #[allow(dead_code)]
    pub fn new(
        planning_workspace_port: Arc<dyn PlanningWorkspacePort>,
        planning_validation_service: PlanningValidationService,
        priority_queue_service: PriorityQueueService,
    ) -> Self {
        Self::with_task_repository(
            planning_workspace_port,
            planning_validation_service,
            priority_queue_service,
            Arc::new(crate::application::port::outbound::planning_task_repository_port::NoopPlanningTaskRepositoryPort),
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

    pub fn load_execution_snapshot(
        &self,
        workspace_dir: &str,
    ) -> Result<PlanningExecutionSnapshot> {
        let workspace_record = self
            .planning_workspace_port
            .load_planning_workspace_files(workspace_dir)?;

        Ok(PlanningExecutionSnapshot {
            directions_toml: workspace_record.directions_toml,
            result_output_markdown: workspace_record.result_output_markdown,
        })
    }

    pub fn reconcile_after_turn(
        &self,
        workspace_dir: &str,
        _turn_id: &str,
        changed_planning_file_paths: &[String],
        execution_snapshot: &PlanningExecutionSnapshot,
    ) -> Result<PlanningReconciliationResult> {
        let change_set = PlanningChangeSet::from_paths(changed_planning_file_paths);
        if !change_set.has_relevant_changes() {
            return Ok(PlanningReconciliationResult::default());
        }

        let mut result = PlanningReconciliationResult::default();
        self.planning_workspace_port
            .commit_planning_workspace_files(
                workspace_dir,
                &execution_snapshot_to_workspace_record(execution_snapshot),
            )?;
        result
            .notices
            .push("planning reconciliation restored protected planning files".to_string());

        Ok(result)
    }

    pub fn commit_task_authority_candidate(
        &self,
        workspace_dir: &str,
        candidate_task_authority_json: &str,
        execution_snapshot: &PlanningExecutionSnapshot,
    ) -> Result<PlanningReconciliationResult> {
        let mut result = PlanningReconciliationResult::default();
        let directions_toml = execution_snapshot
            .directions_toml
            .as_deref()
            .unwrap_or_default();
        let result_output_markdown = execution_snapshot
            .result_output_markdown
            .as_deref()
            .unwrap_or_default();
        let validation_result =
            self.planning_validation_service
                .validate_workspace_files(PlanningWorkspaceFiles {
                    directions_toml,
                    task_authority_json: candidate_task_authority_json,
                    result_output_markdown,
                });

        if !validation_result.is_valid() {
            let validation_errors = validation_error_summaries(&validation_result);
            let failure_summary = validation_errors
                .first()
                .cloned()
                .unwrap_or_else(|| "unknown validation failure".to_string());
            let accepted_task_authority_json = self
                .planning_task_repository_port
                .load_task_authority_snapshot(workspace_dir)?
                .map(|snapshot| serde_json::to_string_pretty(&snapshot.task_authority))
                .transpose()?
                .unwrap_or_default();
            result.repair_request = Some(PlanningRepairRequest {
                failure_summary: failure_summary.clone(),
                validation_errors,
                directions_toml: directions_toml.to_string(),
                accepted_task_authority_json,
                rejected_task_authority_json: Some(candidate_task_authority_json.to_string()),
                rejected_archive_path: None,
            });
            result.rejected_task_authority = true;
            result.notices.push(format!(
                "planning worker produced an invalid task authority update ({failure_summary})"
            ));
            return Ok(result);
        }

        let directions = validation_result.directions.as_ref().ok_or_else(|| {
            anyhow::anyhow!("planning validation reported success without parsed directions.toml")
        })?;
        let task_authority = validation_result.task_authority.as_ref().ok_or_else(|| {
            anyhow::anyhow!("planning validation reported success without parsed task authority")
        })?;
        let queue_projection = self
            .priority_queue_service
            .build_projection(directions, task_authority)
            .map_err(|error| {
                anyhow::anyhow!("planning validation passed but queue build failed: {error}")
            })?;
        let authority_snapshot = self
            .planning_task_repository_port
            .load_task_authority_snapshot(workspace_dir)?;
        match self
            .planning_task_repository_port
            .commit_task_authority_snapshot(
                workspace_dir,
                PlanningTaskAuthorityCommit {
                    observed_planning_revision: authority_snapshot
                        .as_ref()
                        .map(|snapshot| snapshot.planning_revision),
                    task_authority,
                    queue_projection: &queue_projection,
                },
            )? {
            PlanningTaskAuthorityCommitResult::Committed { .. } => {}
            PlanningTaskAuthorityCommitResult::Conflict {
                observed_planning_revision,
                current_planning_revision,
            } => {
                anyhow::bail!(
                    "planning db changed while committing worker task authority update (observed revision {observed_planning_revision}, current revision {current_planning_revision}); reload and retry"
                );
            }
        }

        result.queue_projection_action =
            Some(PlanningQueueProjectionAction::RebuiltFromAcceptedPlanning);
        result
            .notices
            .push("planning worker committed DB task authority update".to_string());
        Ok(result)
    }
}

pub(super) fn execution_snapshot_to_workspace_record(
    execution_snapshot: &PlanningExecutionSnapshot,
) -> PlanningWorkspaceLoadRecord {
    PlanningWorkspaceLoadRecord {
        directions_toml: execution_snapshot.directions_toml.clone(),
        result_output_markdown: execution_snapshot.result_output_markdown.clone(),
    }
}

fn validation_error_summaries(
    validation_result: &crate::domain::planning::PlanningValidationResult,
) -> Vec<String> {
    validation_result
        .report
        .issues
        .iter()
        .filter(|issue| {
            issue.severity == crate::domain::planning::PlanningValidationSeverity::Error
        })
        .map(|issue| issue.message.clone())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{PlanningChangeSet, PlanningRepairRequest, build_planning_repair_prompt};

    #[test]
    fn change_set_ignores_legacy_task_file_paths() {
        let paths = vec![
            "DB task authority".to_string(),
            ".codex-exec-loop/planning/legacy-queue-snapshot.json".to_string(),
        ];

        let change_set = PlanningChangeSet::from_paths(&paths);

        assert!(!change_set.has_relevant_changes());
    }

    #[test]
    fn repair_prompt_requests_task_authority_payload_without_file_artifacts() {
        let prompt = build_planning_repair_prompt(
            &PlanningRepairRequest {
                failure_summary: "invalid task".to_string(),
                validation_errors: vec!["task has unknown direction".to_string()],
                directions_toml: "version = 1".to_string(),
                accepted_task_authority_json: "{\"version\":1,\"tasks\":[]}".to_string(),
                rejected_task_authority_json: Some("{ invalid json".to_string()),
                rejected_archive_path: None,
            },
            None,
            1,
            2,
            None,
        );

        assert!(prompt.contains("\"task_authority\""));
        assert!(!prompt.contains("task authority schema file"));
        assert!(!prompt.contains("queue snapshot artifact"));
    }
}
