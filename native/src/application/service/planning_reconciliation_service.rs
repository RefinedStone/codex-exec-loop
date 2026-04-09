use std::sync::Arc;

use anyhow::{Context, Result};

use crate::application::port::outbound::planning_workspace_port::{
    PlanningWorkspaceLoadRecord, PlanningWorkspacePort,
};
use crate::domain::planning::{
    DIRECTIONS_FILE_PATH, PlanningWorkspaceFiles, QUEUE_SNAPSHOT_FILE_PATH, TASK_LEDGER_FILE_PATH,
};

use super::planning_validation_service::PlanningValidationService;
use super::priority_queue_service::PriorityQueueService;

#[derive(Clone)]
pub struct PlanningReconciliationService {
    planning_workspace_port: Arc<dyn PlanningWorkspacePort>,
    planning_validation_service: PlanningValidationService,
    priority_queue_service: PriorityQueueService,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PlanningExecutionSnapshot {
    pub directions_toml: Option<String>,
    pub task_ledger_json: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PlanningReconciliationResult {
    pub notices: Vec<String>,
    pub rejected_task_ledger: bool,
    pub rejected_archive_path: Option<String>,
    pub queue_snapshot_rebuilt: bool,
    pub auto_followup_block_reason: Option<String>,
}

impl PlanningReconciliationService {
    pub fn new(
        planning_workspace_port: Arc<dyn PlanningWorkspacePort>,
        planning_validation_service: PlanningValidationService,
        priority_queue_service: PriorityQueueService,
    ) -> Self {
        Self {
            planning_workspace_port,
            planning_validation_service,
            priority_queue_service,
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
            task_ledger_json: workspace_record.task_ledger_json,
        })
    }

    pub fn reconcile_after_turn(
        &self,
        workspace_dir: &str,
        turn_id: &str,
        changed_planning_file_paths: &[String],
        execution_snapshot: &PlanningExecutionSnapshot,
    ) -> Result<PlanningReconciliationResult> {
        let directions_changed = changed_planning_file_paths
            .iter()
            .any(|path| path == DIRECTIONS_FILE_PATH);
        let task_ledger_changed = changed_planning_file_paths
            .iter()
            .any(|path| path == TASK_LEDGER_FILE_PATH);

        if !directions_changed && !task_ledger_changed {
            return Ok(PlanningReconciliationResult::default());
        }

        let mut result = PlanningReconciliationResult::default();
        let workspace_record = self
            .planning_workspace_port
            .load_planning_workspace_files(workspace_dir)?;

        let directions_toml = if directions_changed {
            self.planning_workspace_port
                .replace_planning_workspace_file(
                    workspace_dir,
                    DIRECTIONS_FILE_PATH,
                    execution_snapshot.directions_toml.as_deref(),
                )?;
            result
                .notices
                .push("planning reconciliation restored protected directions.toml".to_string());
            execution_snapshot
                .directions_toml
                .as_deref()
                .unwrap_or_default()
        } else {
            workspace_record
                .directions_toml
                .as_deref()
                .unwrap_or_default()
        };

        if task_ledger_changed {
            self.reconcile_task_ledger(
                workspace_dir,
                turn_id,
                &workspace_record,
                execution_snapshot,
                directions_toml,
                &mut result,
            )?;
        }

        Ok(result)
    }

    fn reconcile_task_ledger(
        &self,
        workspace_dir: &str,
        turn_id: &str,
        workspace_record: &PlanningWorkspaceLoadRecord,
        execution_snapshot: &PlanningExecutionSnapshot,
        directions_toml: &str,
        result: &mut PlanningReconciliationResult,
    ) -> Result<()> {
        let task_ledger_candidate = workspace_record
            .task_ledger_json
            .as_deref()
            .unwrap_or_default();
        let validation_result =
            self.planning_validation_service
                .validate_workspace_files(PlanningWorkspaceFiles {
                    directions_toml,
                    task_ledger_json: task_ledger_candidate,
                    task_ledger_schema_json: workspace_record
                        .task_ledger_schema_json
                        .as_deref()
                        .unwrap_or_default(),
                    result_output_markdown: workspace_record
                        .result_output_markdown
                        .as_deref()
                        .unwrap_or_default(),
                });

        if validation_result.is_valid() {
            let queue_snapshot = self.priority_queue_service.build_snapshot(
                validation_result
                    .directions
                    .as_ref()
                    .expect("valid planning reconciliation should include directions"),
                validation_result
                    .task_ledger
                    .as_ref()
                    .expect("valid planning reconciliation should include task ledger"),
            );
            let queue_snapshot_json = serde_json::to_string_pretty(&queue_snapshot)
                .context("failed to serialize queue snapshot")?;
            self.planning_workspace_port
                .replace_planning_workspace_file(
                    workspace_dir,
                    QUEUE_SNAPSHOT_FILE_PATH,
                    Some(queue_snapshot_json.as_str()),
                )?;
            result.queue_snapshot_rebuilt = true;
            result.notices.push(
                "planning reconciliation accepted task-ledger.json and rebuilt queue.snapshot.json"
                    .to_string(),
            );
            return Ok(());
        }

        if let Some(task_ledger_json) = workspace_record.task_ledger_json.as_deref() {
            let archive_path = self
                .planning_workspace_port
                .archive_rejected_planning_file(
                    workspace_dir,
                    turn_id,
                    TASK_LEDGER_FILE_PATH,
                    task_ledger_json,
                )?;
            result.rejected_archive_path = Some(archive_path);
        }

        self.planning_workspace_port
            .replace_planning_workspace_file(
                workspace_dir,
                TASK_LEDGER_FILE_PATH,
                execution_snapshot.task_ledger_json.as_deref(),
            )?;
        result.rejected_task_ledger = true;
        result.auto_followup_block_reason = Some(
            "planning reconciliation rejected task-ledger.json; auto follow-up stays paused until repair flow is wired"
                .to_string(),
        );
        result.notices.push(format!(
            "planning reconciliation rejected task-ledger.json and restored the last accepted ledger ({})",
            first_validation_error_summary(&validation_result)
        ));
        if let Some(rejected_archive_path) = result.rejected_archive_path.as_deref() {
            result.notices.push(format!(
                "planning reconciliation archived rejected task-ledger at {rejected_archive_path}"
            ));
        }

        Ok(())
    }
}

fn first_validation_error_summary(
    validation_result: &crate::domain::planning::PlanningValidationResult,
) -> String {
    validation_result
        .report
        .errors()
        .into_iter()
        .next()
        .map(|issue| issue.message.clone())
        .unwrap_or_else(|| "unknown validation failure".to_string())
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;
    use std::time::{SystemTime, UNIX_EPOCH};

    use serde_json::json;

    use super::{PlanningExecutionSnapshot, PlanningReconciliationService};
    use crate::adapter::outbound::filesystem_planning_workspace_adapter::FilesystemPlanningWorkspaceAdapter;
    use crate::application::service::planning_bootstrap_service::PlanningBootstrapService;
    use crate::application::service::planning_validation_service::PlanningValidationService;
    use crate::application::service::priority_queue_service::PriorityQueueService;
    use crate::domain::planning::{
        DIRECTIONS_FILE_PATH, QUEUE_SNAPSHOT_FILE_PATH, TASK_LEDGER_FILE_PATH,
    };

    fn create_temp_workspace(prefix: &str) -> String {
        let unique_suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock should be valid")
            .as_nanos();
        let path = std::env::temp_dir().join(format!("{prefix}-{unique_suffix}"));
        fs::create_dir_all(&path).expect("temp workspace should be created");
        path.display().to_string()
    }

    fn write_bootstrap_workspace(workspace_dir: &str) -> PlanningExecutionSnapshot {
        let planning_dir = Path::new(workspace_dir).join(".codex-exec-loop/planning");
        fs::create_dir_all(&planning_dir).expect("planning directory should be created");
        let artifacts = PlanningBootstrapService::new().build_artifacts();
        fs::write(
            planning_dir.join("directions.toml"),
            &artifacts.directions_toml,
        )
        .expect("directions should write");
        fs::write(
            planning_dir.join("task-ledger.json"),
            &artifacts.task_ledger_json,
        )
        .expect("task ledger should write");
        fs::write(
            planning_dir.join("task-ledger.schema.json"),
            &artifacts.task_ledger_schema_json,
        )
        .expect("schema should write");
        fs::write(
            planning_dir.join("result-output.md"),
            &artifacts.result_output_markdown,
        )
        .expect("result output should write");

        PlanningExecutionSnapshot {
            directions_toml: Some(artifacts.directions_toml),
            task_ledger_json: Some(artifacts.task_ledger_json),
        }
    }

    fn service() -> PlanningReconciliationService {
        PlanningReconciliationService::new(
            Arc::new(FilesystemPlanningWorkspaceAdapter::new()),
            PlanningValidationService::new(),
            PriorityQueueService::new(),
        )
    }

    use std::sync::Arc;

    #[test]
    fn valid_task_ledger_change_rebuilds_queue_snapshot() {
        let workspace_dir = create_temp_workspace("planning-reconcile-valid");
        let execution_snapshot = write_bootstrap_workspace(&workspace_dir);
        let valid_task_ledger = serde_json::to_string_pretty(&json!({
            "version": 1,
            "tasks": [
                {
                    "id": "task-1",
                    "direction_id": "example-direction",
                    "direction_relation_note": "implements the active example direction",
                    "title": "Do the thing",
                    "description": "Implement the next queued step.",
                    "status": "ready",
                    "base_priority": 10,
                    "dynamic_priority_delta": 0,
                    "priority_reason": "",
                    "depends_on": [],
                    "blocked_by": [],
                    "created_by": "llm",
                    "last_updated_by": "llm",
                    "source_turn_id": "turn-1",
                    "updated_at": "2026-04-09T10:00:00Z"
                }
            ]
        }))
        .expect("valid task ledger should serialize");
        fs::write(
            Path::new(&workspace_dir).join(".codex-exec-loop/planning/task-ledger.json"),
            valid_task_ledger,
        )
        .expect("task ledger candidate should write");

        let result = service()
            .reconcile_after_turn(
                &workspace_dir,
                "turn-1",
                &[TASK_LEDGER_FILE_PATH.to_string()],
                &execution_snapshot,
            )
            .expect("reconciliation should succeed");

        let queue_snapshot = fs::read_to_string(
            Path::new(&workspace_dir).join(".codex-exec-loop/planning/queue.snapshot.json"),
        )
        .expect("queue snapshot should exist");

        assert!(result.queue_snapshot_rebuilt);
        assert!(!result.rejected_task_ledger);
        assert!(queue_snapshot.contains("\"task_id\": \"task-1\""));

        fs::remove_dir_all(workspace_dir).expect("temp workspace should be removed");
    }

    #[test]
    fn invalid_task_ledger_change_is_archived_and_restored() {
        let workspace_dir = create_temp_workspace("planning-reconcile-invalid");
        let execution_snapshot = write_bootstrap_workspace(&workspace_dir);
        let planning_dir = Path::new(&workspace_dir).join(".codex-exec-loop/planning");
        fs::write(planning_dir.join("task-ledger.json"), "{ invalid json")
            .expect("invalid task ledger should write");

        let result = service()
            .reconcile_after_turn(
                &workspace_dir,
                "turn-2",
                &[TASK_LEDGER_FILE_PATH.to_string()],
                &execution_snapshot,
            )
            .expect("reconciliation should succeed");

        let restored_task_ledger = fs::read_to_string(planning_dir.join("task-ledger.json"))
            .expect("restored task ledger should be readable");

        assert!(result.rejected_task_ledger);
        assert!(result.rejected_archive_path.is_some());
        assert_eq!(
            restored_task_ledger,
            execution_snapshot
                .task_ledger_json
                .expect("execution snapshot should keep the accepted task ledger")
        );
        assert!(
            Path::new(
                result
                    .rejected_archive_path
                    .as_deref()
                    .expect("archive path should be present")
            )
            .exists()
        );

        fs::remove_dir_all(workspace_dir).expect("temp workspace should be removed");
    }

    #[test]
    fn changed_directions_are_restored_from_execution_snapshot() {
        let workspace_dir = create_temp_workspace("planning-reconcile-directions");
        let execution_snapshot = write_bootstrap_workspace(&workspace_dir);
        let planning_dir = Path::new(&workspace_dir).join(".codex-exec-loop/planning");
        fs::write(planning_dir.join("directions.toml"), "version = 1\n")
            .expect("mutated directions should write");

        let result = service()
            .reconcile_after_turn(
                &workspace_dir,
                "turn-3",
                &[DIRECTIONS_FILE_PATH.to_string()],
                &execution_snapshot,
            )
            .expect("reconciliation should succeed");

        let restored_directions = fs::read_to_string(planning_dir.join("directions.toml"))
            .expect("restored directions should be readable");

        assert!(!result.rejected_task_ledger);
        assert_eq!(
            restored_directions,
            execution_snapshot
                .directions_toml
                .expect("execution snapshot should keep the accepted directions")
        );
        assert_eq!(
            Path::new(&workspace_dir)
                .join(QUEUE_SNAPSHOT_FILE_PATH)
                .exists(),
            false
        );

        fs::remove_dir_all(workspace_dir).expect("temp workspace should be removed");
    }
}
