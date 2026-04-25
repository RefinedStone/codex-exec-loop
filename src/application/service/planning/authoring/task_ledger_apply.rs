use std::sync::Arc;

use anyhow::{Result, anyhow};

use crate::application::port::outbound::planning_task_repository_port::{
    PlanningTaskAuthorityCommit, PlanningTaskAuthorityCommitResult, PlanningTaskRepositoryPort,
};
use crate::application::port::outbound::planning_workspace_port::{
    PlanningWorkspaceLoadRecord, PlanningWorkspacePort,
};
use crate::application::service::planning::runtime::validation::PlanningValidationService;
use crate::application::service::planning::shared::contract::{
    QUEUE_SNAPSHOT_FILE_PATH, TASK_LEDGER_FILE_PATH,
};
use crate::application::service::priority_queue_service::PriorityQueueService;
use crate::domain::planning::{PlanningValidationReport, PlanningWorkspaceFiles};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanningTrackedTaskLedgerApplyResult {
    pub applied_paths: Vec<String>,
    pub validation_report: PlanningValidationReport,
}

impl PlanningTrackedTaskLedgerApplyResult {
    pub fn applied(&self) -> bool {
        !self.applied_paths.is_empty() && self.validation_report.is_valid()
    }
}

#[derive(Clone)]
pub struct PlanningTaskLedgerApplyService {
    planning_workspace_port: Arc<dyn PlanningWorkspacePort>,
    planning_validation_service: PlanningValidationService,
    priority_queue_service: PriorityQueueService,
    planning_task_repository_port: Arc<dyn PlanningTaskRepositoryPort>,
}

impl PlanningTaskLedgerApplyService {
    #[cfg(test)]
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

    pub fn apply_tracked_task_ledger(
        &self,
        workspace_dir: &str,
    ) -> Result<PlanningTrackedTaskLedgerApplyResult> {
        let active_workspace = self.load_active_workspace(workspace_dir)?;
        let candidate_task_ledger = self.load_candidate_task_ledger(workspace_dir)?;
        let validation_result =
            self.planning_validation_service
                .validate_workspace_files(PlanningWorkspaceFiles {
                    directions_toml: required_workspace_body(
                        &active_workspace,
                        WorkspaceBody::Directions,
                    )?,
                    task_ledger_json: &candidate_task_ledger,
                    task_ledger_schema_json: required_workspace_body(
                        &active_workspace,
                        WorkspaceBody::TaskLedgerSchema,
                    )?,
                    result_output_markdown: required_workspace_body(
                        &active_workspace,
                        WorkspaceBody::ResultOutput,
                    )?,
                });

        if !validation_result.is_valid() {
            return Ok(PlanningTrackedTaskLedgerApplyResult {
                applied_paths: Vec::new(),
                validation_report: validation_result.report,
            });
        }

        let directions = validation_result.directions.as_ref().ok_or_else(|| {
            anyhow!("planning validation reported success without parsed directions.toml")
        })?;
        let task_ledger = validation_result.task_ledger.as_ref().ok_or_else(|| {
            anyhow!("planning validation reported success without parsed task-ledger.json")
        })?;
        let queue_snapshot = self
            .priority_queue_service
            .build_snapshot(directions, task_ledger)
            .map_err(|error| {
                anyhow!("planning validation passed but queue build failed: {error}")
            })?;
        let authority_snapshot = self
            .planning_task_repository_port
            .load_task_authority_snapshot(workspace_dir)?;
        if let Some(snapshot) = authority_snapshot {
            match self
                .planning_task_repository_port
                .commit_task_authority_snapshot(
                    workspace_dir,
                    PlanningTaskAuthorityCommit {
                        observed_planning_revision: Some(snapshot.planning_revision),
                        task_ledger,
                        queue_snapshot: &queue_snapshot,
                    },
                )? {
                PlanningTaskAuthorityCommitResult::Committed { .. } => {}
                PlanningTaskAuthorityCommitResult::Conflict {
                    observed_planning_revision,
                    current_planning_revision,
                } => {
                    return Err(anyhow!(
                        "planning db changed while applying tracked task ledger (observed revision {observed_planning_revision}, current revision {current_planning_revision}); reload and retry"
                    ));
                }
            }
        } else {
            let queue_snapshot_json = serde_json::to_string_pretty(&queue_snapshot)?;
            let mut committed_record = active_workspace;
            committed_record.task_ledger_json = Some(candidate_task_ledger);
            committed_record.queue_snapshot_json = Some(queue_snapshot_json);
            self.planning_workspace_port
                .commit_planning_workspace_files(workspace_dir, &committed_record)?;
        }

        Ok(PlanningTrackedTaskLedgerApplyResult {
            applied_paths: vec![
                TASK_LEDGER_FILE_PATH.to_string(),
                QUEUE_SNAPSHOT_FILE_PATH.to_string(),
            ],
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

    fn load_candidate_task_ledger(&self, workspace_dir: &str) -> Result<String> {
        let candidate_workspace = self
            .planning_workspace_port
            .load_planning_workspace_candidate_files(workspace_dir)?;
        candidate_workspace.task_ledger_json.ok_or_else(|| {
            anyhow!(
                "tracked task-ledger import requires .codex-exec-loop/planning/task-ledger.json in the workspace root"
            )
        })
    }
}

#[derive(Clone, Copy)]
enum WorkspaceBody {
    Directions,
    TaskLedgerSchema,
    ResultOutput,
}

fn required_workspace_body(
    workspace: &PlanningWorkspaceLoadRecord,
    body: WorkspaceBody,
) -> Result<&str> {
    match body {
        WorkspaceBody::Directions => workspace
            .directions_toml
            .as_deref()
            .ok_or_else(|| anyhow!("planning workspace is missing directions.toml")),
        WorkspaceBody::TaskLedgerSchema => workspace
            .task_ledger_schema_json
            .as_deref()
            .ok_or_else(|| anyhow!("planning workspace is missing task-ledger.schema.json")),
        WorkspaceBody::ResultOutput => workspace
            .result_output_markdown
            .as_deref()
            .ok_or_else(|| anyhow!("planning workspace is missing result-output.md")),
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;

    use super::PlanningTaskLedgerApplyService;
    use crate::adapter::outbound::filesystem::FilesystemPlanningWorkspaceAdapter;
    use crate::application::service::planning::authoring::bootstrap::{
        PlanningBootstrapMode, PlanningBootstrapService,
    };
    use crate::application::service::planning::runtime::validation::PlanningValidationService;
    use crate::application::service::planning::shared::contract::{
        QUEUE_SNAPSHOT_FILE_PATH, TASK_LEDGER_FILE_PATH,
    };
    use crate::application::service::priority_queue_service::PriorityQueueService;

    fn create_temp_workspace(prefix: &str) -> String {
        let unique_suffix = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system clock should be valid")
            .as_nanos();
        let path = std::env::temp_dir().join(format!("{prefix}-{unique_suffix}"));
        fs::create_dir_all(&path).expect("temp workspace should be created");
        path.display().to_string()
    }

    fn write_bootstrap_candidate_workspace(workspace_dir: &str) {
        let planning_dir = Path::new(workspace_dir).join(".codex-exec-loop/planning");
        fs::create_dir_all(&planning_dir).expect("planning directory should be created");
        let artifacts =
            PlanningBootstrapService::new().build_artifacts_for_mode(PlanningBootstrapMode::Detail);
        fs::write(
            planning_dir.join("directions.toml"),
            artifacts.directions_toml,
        )
        .expect("directions should write");
        fs::write(
            planning_dir.join("task-ledger.json"),
            artifacts.task_ledger_json,
        )
        .expect("task ledger should write");
        fs::write(
            planning_dir.join("task-ledger.schema.json"),
            artifacts.task_ledger_schema_json,
        )
        .expect("schema should write");
        fs::write(
            planning_dir.join("result-output.md"),
            artifacts.result_output_markdown,
        )
        .expect("result output should write");
    }

    #[test]
    fn applies_tracked_task_ledger_and_rebuilds_queue_snapshot() {
        let workspace_dir = create_temp_workspace("task-ledger-apply-success");
        write_bootstrap_candidate_workspace(&workspace_dir);
        let task_ledger = r#"{
  "version": 1,
  "tasks": [
    {
      "id": "task-1",
      "direction_id": "example-direction",
      "direction_relation_note": "queue apply test",
      "title": "Apply tracked task ledger",
      "description": "Sync the tracked task ledger into active planning.",
      "status": "ready",
      "base_priority": 42,
      "dynamic_priority_delta": 0,
      "priority_reason": "Repair authority drift.",
      "depends_on": [],
      "blocked_by": [],
      "created_by": "user",
      "last_updated_by": "user",
      "source_turn_id": null,
      "updated_at": "2026-04-23T00:00:00Z"
    }
  ]
}"#;
        fs::write(
            Path::new(&workspace_dir).join(TASK_LEDGER_FILE_PATH),
            task_ledger,
        )
        .expect("tracked task ledger should write");

        let result = PlanningTaskLedgerApplyService::new(
            Arc::new(FilesystemPlanningWorkspaceAdapter::new()),
            PlanningValidationService::new(),
            PriorityQueueService::new(),
        )
        .apply_tracked_task_ledger(&workspace_dir)
        .expect("task-ledger apply should succeed");

        let queue_snapshot =
            fs::read_to_string(Path::new(&workspace_dir).join(QUEUE_SNAPSHOT_FILE_PATH))
                .expect("queue snapshot should exist");

        assert!(result.applied());
        assert_eq!(
            result.applied_paths,
            vec![
                TASK_LEDGER_FILE_PATH.to_string(),
                QUEUE_SNAPSHOT_FILE_PATH.to_string()
            ]
        );
        assert!(queue_snapshot.contains("\"task_id\": \"task-1\""));

        fs::remove_dir_all(workspace_dir).expect("temp workspace should be removed");
    }

    #[test]
    fn tracked_task_ledger_apply_reports_validation_failure() {
        let workspace_dir = create_temp_workspace("task-ledger-apply-invalid");
        write_bootstrap_candidate_workspace(&workspace_dir);
        fs::write(
            Path::new(&workspace_dir).join(TASK_LEDGER_FILE_PATH),
            r#"{
  "version": 1,
  "tasks": [
    {
      "id": "task-1",
      "direction_id": "missing-direction",
      "direction_relation_note": "queue apply test",
      "title": "Apply tracked task ledger",
      "description": "Sync the tracked task ledger into active planning.",
      "status": "ready",
      "base_priority": 42,
      "dynamic_priority_delta": 0,
      "priority_reason": "Repair authority drift.",
      "depends_on": [],
      "blocked_by": [],
      "created_by": "user",
      "last_updated_by": "user",
      "source_turn_id": null,
      "updated_at": "2026-04-23T00:00:00Z"
    }
  ]
}"#,
        )
        .expect("invalid tracked task ledger should write");

        let result = PlanningTaskLedgerApplyService::new(
            Arc::new(FilesystemPlanningWorkspaceAdapter::new()),
            PlanningValidationService::new(),
            PriorityQueueService::new(),
        )
        .apply_tracked_task_ledger(&workspace_dir)
        .expect("task-ledger apply should return validation report");

        assert!(!result.applied());
        assert!(
            result
                .validation_report
                .errors()
                .iter()
                .any(|issue| issue.message.contains("references unknown direction_id"))
        );

        fs::remove_dir_all(workspace_dir).expect("temp workspace should be removed");
    }

    use std::sync::Arc;
}
