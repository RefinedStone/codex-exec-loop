/*
 * Post-turn reconciliation protects planning state after a Codex execution has finished.
 * DB-backed direction/task authority is now the source of truth, so this service currently focuses on
 * active planning support files that must not be casually rewritten by a turn. runtime/facade.rs captures
 * a PlanningExecutionSnapshot before execution and calls reconcile_after_turn afterward with the planning
 * paths touched by the turn; this module compares those two facts and restores protected files when needed.
 */
use std::sync::Arc;

use anyhow::Result;

use crate::application::port::outbound::planning_task_repository_port::PlanningTaskRepositoryPort;
use crate::application::port::outbound::planning_workspace_port::PlanningWorkspaceLoadRecord;
use crate::application::port::outbound::planning_workspace_port::PlanningWorkspacePort;
use crate::application::service::planning::runtime::validation::PlanningValidationService;
use crate::application::service::planning::shared::contract::{
    RESULT_OUTPUT_FILE_PATH, canonical_active_planning_file_path,
};
use crate::domain::planning::PriorityQueueService;

pub use super::ledger_recovery::PlanningQueueProjectionAction;
pub use super::prompt::{
    PlanningRepairPromptHandoff, PlanningRepairRetryReason, build_planning_repair_prompt,
};
pub use super::protected_restore::PlanningProtectedFileRestoration;

#[derive(Clone)]
/*
 * Reconciliation is intentionally small in the current DB-authority model.
 * The workspace port is enough to reload and restore support files; validation, queue, and task repository
 * dependencies remain in the constructor contract so older composition and future authority repair flows can
 * share the same service boundary without changing facade wiring again.
 */
pub struct PlanningReconciliationService {
    planning_workspace_port: Arc<dyn PlanningWorkspacePort>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
/*
 * Snapshot of planning files that should survive the user turn unchanged.
 * Only result-output is captured today because task/direction authority moved to DB ports, but the type is
 * deliberately separate from PlanningWorkspaceLoadRecord so it represents an execution guard, not a generic load.
 */
pub struct PlanningExecutionSnapshot {
    // result-output.md defines completion copy, so unexpected edits are restored after the turn.
    pub result_output_markdown: Option<String>,
}

impl PlanningExecutionSnapshot {
    // TUI post-turn code uses this cheap path check before it asks the service to reconcile.
    pub fn captures_path(path: &str) -> bool {
        canonical_active_planning_file_path(path).is_some()
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
/*
 * Post-turn reconciliation returns an operational report, not just success/failure.
 * UI callers use notices and restored file lists for status copy; worker orchestration can use repair_request
 * or auto_followup_block_reason when a future authority repair path detects invalid generated planning state.
 */
pub struct PlanningReconciliationResult {
    // Human-readable status lines surfaced by post-turn UI paths.
    pub notices: Vec<String>,
    // Structured restoration details for protected files when a restore path records per-file outcomes.
    pub restored_protected_files: Vec<PlanningProtectedFileRestoration>,
    // True when a generated task authority candidate was rejected instead of accepted.
    pub rejected_task_authority: bool,
    // Archive path for the rejected candidate, if the recovery path persisted one for operator inspection.
    pub rejected_archive_path: Option<String>,
    // Queue projection adjustment made during authority recovery.
    pub queue_projection_action: Option<PlanningQueueProjectionAction>,
    // Prompt payload for a repair worker when automatic reconciliation cannot safely accept a candidate.
    pub repair_request: Option<PlanningRepairRequest>,
    // Reason auto-follow should stop after reconciliation notices unsafe planning state.
    pub auto_followup_block_reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
/*
 * Repair request is the serialized context handed to planning repair prompt generation.
 * It carries accepted authority and queue projection as the trusted baseline, plus the rejected candidate and
 * validation messages so the repair worker can emit task mutation commands without guessing what failed.
 */
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
// Compact description of which protected active planning files were touched by a turn.
pub(super) struct PlanningChangeSet {
    pub(super) result_output_changed: bool,
}

impl PlanningChangeSet {
    // Normalize reported paths through the shared active-planning contract before deciding relevance.
    fn from_paths(paths: &[String]) -> Self {
        let mut change_set = Self::default();
        for path in paths {
            if let Some(RESULT_OUTPUT_FILE_PATH) = canonical_active_planning_file_path(path) {
                change_set.result_output_changed = true;
            }
        }
        change_set
    }

    // Reconciliation can be skipped entirely when no protected file changed.
    fn has_relevant_changes(self) -> bool {
        self.result_output_changed
    }
}

impl PlanningReconciliationService {
    #[cfg(test)]
    #[allow(dead_code)]
    // Test constructor keeps historical dependency shape while routing through the production constructor.
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

    /*
     * Production constructor accepts the full reconciliation dependency set.
     * Current protected-file restoration only stores the workspace port, while the prefixed arguments mark
     * temporarily dormant authority repair collaborators without leaking that choice into composition.
     */
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

    /*
     * Capture protected file content before Codex is allowed to execute the turn.
     * The snapshot is later converted back into a workspace record for restore, so the restore operation uses
     * the same workspace-port write path as normal planning file commits.
     */
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

    /*
     * Restore protected planning files only when the turn actually touched a captured active path.
     * The changed path list is the cheap guard; if it contains result-output, the service commits the
     * pre-turn snapshot and records a notice so TUI/auto-follow callers can explain that reconciliation
     * intentionally discarded the turn's edit to protected planning copy.
     */
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

// Convert the execution guard back into the minimal workspace-port payload needed for protected-file restore.
pub(super) fn execution_snapshot_to_workspace_record(
    execution_snapshot: &PlanningExecutionSnapshot,
) -> PlanningWorkspaceLoadRecord {
    PlanningWorkspaceLoadRecord {
        result_output_markdown: execution_snapshot.result_output_markdown.clone(),
    }
}

#[cfg(test)]
mod tests;
