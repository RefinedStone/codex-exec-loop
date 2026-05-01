use std::sync::Arc;

use anyhow::Result;

use crate::application::port::outbound::planning_task_repository_port::PlanningTaskRepositoryPort;
use crate::application::port::outbound::planning_workspace_port::PlanningWorkspaceLoadRecord;
use crate::application::port::outbound::planning_workspace_port::PlanningWorkspacePort;
use crate::application::service::planning::shared::contract::{
    RESULT_OUTPUT_FILE_PATH, canonical_active_planning_file_path,
};
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
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PlanningExecutionSnapshot {
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
    pub direction_authority_json: String,
    pub accepted_task_authority_json: String,
    pub accepted_queue_projection_json: String,
    pub rejected_task_authority_json: Option<String>,
    pub rejected_archive_path: Option<String>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(super) struct PlanningChangeSet {
    pub(super) result_output_changed: bool,
}

impl PlanningChangeSet {
    fn from_paths(paths: &[String]) -> Self {
        let mut change_set = Self::default();
        for path in paths {
            if let Some(RESULT_OUTPUT_FILE_PATH) = canonical_active_planning_file_path(path) {
                change_set.result_output_changed = true;
            }
        }
        change_set
    }

    fn has_relevant_changes(self) -> bool {
        self.result_output_changed
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
        _planning_validation_service: PlanningValidationService,
        _priority_queue_service: PriorityQueueService,
        _planning_task_repository_port: Arc<dyn PlanningTaskRepositoryPort>,
    ) -> Self {
        Self {
            planning_workspace_port,
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
}

pub(super) fn execution_snapshot_to_workspace_record(
    execution_snapshot: &PlanningExecutionSnapshot,
) -> PlanningWorkspaceLoadRecord {
    PlanningWorkspaceLoadRecord {
        result_output_markdown: execution_snapshot.result_output_markdown.clone(),
    }
}

#[cfg(test)]
mod tests;
