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
        let queue_projection = self
            .priority_queue_service
            .build_projection(directions, task_ledger)
            .map_err(|error| {
                anyhow!("planning validation passed but queue build failed: {error}")
            })?;
        let authority_snapshot = self
            .planning_task_repository_port
            .load_task_authority_snapshot(workspace_dir)?;
        if let Some(snapshot) = authority_snapshot.as_ref() {
            match self
                .planning_task_repository_port
                .commit_task_authority_snapshot(
                    workspace_dir,
                    PlanningTaskAuthorityCommit {
                        observed_planning_revision: Some(snapshot.planning_revision),
                        task_ledger,
                        queue_projection: &queue_projection,
                    },
                )? {
                PlanningTaskAuthorityCommitResult::Committed { .. } => {}
                PlanningTaskAuthorityCommitResult::Conflict {
                    observed_planning_revision,
                    current_planning_revision,
                } => {
                    return Err(anyhow!(
                        "planning db changed while applying tracked task catalog (observed revision {observed_planning_revision}, current revision {current_planning_revision}); reload and retry"
                    ));
                }
            }
        }

        self.planning_workspace_port
            .replace_planning_workspace_file(
                workspace_dir,
                TASK_LEDGER_FILE_PATH,
                Some(&candidate_task_ledger),
            )?;
        let queue_snapshot_json = serde_json::to_string_pretty(&queue_projection)?;
        self.planning_workspace_port
            .replace_planning_workspace_file(
                workspace_dir,
                QUEUE_SNAPSHOT_FILE_PATH,
                Some(&queue_snapshot_json),
            )?;

        if authority_snapshot.is_none() {
            self.planning_task_repository_port
                .commit_task_authority_snapshot(
                    workspace_dir,
                    PlanningTaskAuthorityCommit {
                        observed_planning_revision: None,
                        task_ledger,
                        queue_projection: &queue_projection,
                    },
                )?;
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
    use std::sync::{Arc, Mutex};

    use anyhow::{Result, anyhow};

    use super::PlanningTaskLedgerApplyService;
    use crate::adapter::outbound::filesystem::FilesystemPlanningWorkspaceAdapter;
    use crate::application::port::outbound::planning_task_repository_port::{
        PlanningTaskAuthorityCommit, PlanningTaskAuthorityCommitResult,
        PlanningTaskAuthoritySnapshot, PlanningTaskRepositoryPort,
    };
    use crate::application::port::outbound::planning_workspace_port::{
        PlanningDraftFileRecord, PlanningDraftLoadRecord, PlanningDraftStageRecord,
        PlanningWorkspaceLoadRecord, PlanningWorkspacePort,
    };
    use crate::application::service::planning::authoring::bootstrap::{
        PlanningBootstrapMode, PlanningBootstrapService,
    };
    use crate::application::service::planning::runtime::validation::PlanningValidationService;
    use crate::application::service::planning::shared::contract::{
        QUEUE_SNAPSHOT_FILE_PATH, TASK_LEDGER_FILE_PATH,
    };
    use crate::application::service::priority_queue_service::PriorityQueueService;
    use crate::domain::planning::{PriorityQueueProjection, TaskLedgerDocument};

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

    fn task_ledger_with_one_ready_task() -> String {
        r#"{
  "version": 1,
  "tasks": [
    {
      "id": "task-1",
      "direction_id": "example-direction",
      "direction_relation_note": "queue apply test",
      "title": "Apply tracked task catalog",
      "description": "Sync the tracked task catalog into active planning.",
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
}"#
        .to_string()
    }

    fn bootstrap_workspace_record() -> PlanningWorkspaceLoadRecord {
        let artifacts =
            PlanningBootstrapService::new().build_artifacts_for_mode(PlanningBootstrapMode::Detail);
        PlanningWorkspaceLoadRecord {
            directions_toml: Some(artifacts.directions_toml),
            task_ledger_json: Some(artifacts.task_ledger_json),
            task_ledger_schema_json: Some(artifacts.task_ledger_schema_json),
            queue_snapshot_json: None,
            result_output_markdown: Some(artifacts.result_output_markdown),
        }
    }

    fn empty_queue_projection() -> PriorityQueueProjection {
        PriorityQueueProjection {
            next_task: None,
            active_tasks: Vec::new(),
            proposed_tasks: Vec::new(),
            skipped_tasks: Vec::new(),
        }
    }

    #[derive(Default)]
    struct RecordingPlanningWorkspacePort {
        active: Mutex<PlanningWorkspaceLoadRecord>,
        candidate: Mutex<PlanningWorkspaceLoadRecord>,
        committed_records: Mutex<Vec<PlanningWorkspaceLoadRecord>>,
        replaced_files: Mutex<Vec<(String, Option<String>)>>,
    }

    impl RecordingPlanningWorkspacePort {
        fn new(
            active: PlanningWorkspaceLoadRecord,
            candidate: PlanningWorkspaceLoadRecord,
        ) -> Self {
            Self {
                active: Mutex::new(active),
                candidate: Mutex::new(candidate),
                committed_records: Mutex::new(Vec::new()),
                replaced_files: Mutex::new(Vec::new()),
            }
        }

        fn committed_records(&self) -> Vec<PlanningWorkspaceLoadRecord> {
            self.committed_records
                .lock()
                .expect("committed records lock should be available")
                .clone()
        }

        fn replaced_files(&self) -> Vec<(String, Option<String>)> {
            self.replaced_files
                .lock()
                .expect("replaced files lock should be available")
                .clone()
        }
    }

    impl PlanningWorkspacePort for RecordingPlanningWorkspacePort {
        fn stage_planning_draft_files(
            &self,
            _workspace_dir: &str,
            _draft_name: &str,
            _files: &[PlanningDraftFileRecord],
        ) -> Result<PlanningDraftStageRecord> {
            Err(anyhow!("unused"))
        }

        fn load_planning_draft_files(
            &self,
            _workspace_dir: &str,
            _draft_name: &str,
        ) -> Result<PlanningDraftLoadRecord> {
            Err(anyhow!("unused"))
        }

        fn replace_planning_draft_file(
            &self,
            _workspace_dir: &str,
            _draft_name: &str,
            _active_path: &str,
            _body: &str,
        ) -> Result<String> {
            Err(anyhow!("unused"))
        }

        fn load_planning_workspace_files(
            &self,
            _workspace_dir: &str,
        ) -> Result<PlanningWorkspaceLoadRecord> {
            Ok(self
                .active
                .lock()
                .expect("active workspace lock should be available")
                .clone())
        }

        fn load_planning_workspace_candidate_files(
            &self,
            _workspace_dir: &str,
        ) -> Result<PlanningWorkspaceLoadRecord> {
            Ok(self
                .candidate
                .lock()
                .expect("candidate workspace lock should be available")
                .clone())
        }

        fn commit_planning_workspace_files(
            &self,
            _workspace_dir: &str,
            record: &PlanningWorkspaceLoadRecord,
        ) -> Result<()> {
            *self
                .active
                .lock()
                .expect("active workspace lock should be available") = record.clone();
            self.committed_records
                .lock()
                .expect("committed records lock should be available")
                .push(record.clone());
            Ok(())
        }

        fn load_optional_planning_file(
            &self,
            _workspace_dir: &str,
            _relative_path: &str,
        ) -> Result<Option<String>> {
            Err(anyhow!("unused"))
        }

        fn load_optional_planning_candidate_file(
            &self,
            _workspace_dir: &str,
            _relative_path: &str,
        ) -> Result<Option<String>> {
            Err(anyhow!("unused"))
        }

        fn replace_planning_workspace_file(
            &self,
            _workspace_dir: &str,
            relative_path: &str,
            body: Option<&str>,
        ) -> Result<()> {
            let mut active = self
                .active
                .lock()
                .expect("active workspace lock should be available");
            match relative_path {
                TASK_LEDGER_FILE_PATH => active.task_ledger_json = body.map(str::to_string),
                QUEUE_SNAPSHOT_FILE_PATH => active.queue_snapshot_json = body.map(str::to_string),
                _ => return Err(anyhow!("unexpected replacement path: {relative_path}")),
            }
            self.replaced_files
                .lock()
                .expect("replaced files lock should be available")
                .push((relative_path.to_string(), body.map(str::to_string)));
            Ok(())
        }

        fn remove_planning_workspace_entry(
            &self,
            _workspace_dir: &str,
            _relative_path: &str,
        ) -> Result<()> {
            Err(anyhow!("unused"))
        }

        fn archive_rejected_planning_file(
            &self,
            _workspace_dir: &str,
            _archive_name: &str,
            _active_path: &str,
            _body: &str,
        ) -> Result<String> {
            Err(anyhow!("unused"))
        }
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    struct RecordedAuthorityCommit {
        observed_planning_revision: Option<i64>,
        task_ids: Vec<String>,
        next_task_id: Option<String>,
    }

    #[derive(Default)]
    struct RecordingTaskRepositoryPort {
        snapshot: Mutex<Option<PlanningTaskAuthoritySnapshot>>,
        commits: Mutex<Vec<RecordedAuthorityCommit>>,
    }

    impl RecordingTaskRepositoryPort {
        fn with_snapshot(snapshot: Option<PlanningTaskAuthoritySnapshot>) -> Self {
            Self {
                snapshot: Mutex::new(snapshot),
                commits: Mutex::new(Vec::new()),
            }
        }

        fn commits(&self) -> Vec<RecordedAuthorityCommit> {
            self.commits
                .lock()
                .expect("commits lock should be available")
                .clone()
        }
    }

    impl PlanningTaskRepositoryPort for RecordingTaskRepositoryPort {
        fn load_task_authority_snapshot(
            &self,
            _workspace_dir: &str,
        ) -> Result<Option<PlanningTaskAuthoritySnapshot>> {
            Ok(self
                .snapshot
                .lock()
                .expect("snapshot lock should be available")
                .clone())
        }

        fn commit_task_authority_snapshot(
            &self,
            _workspace_dir: &str,
            commit: PlanningTaskAuthorityCommit<'_>,
        ) -> Result<PlanningTaskAuthorityCommitResult> {
            self.commits
                .lock()
                .expect("commits lock should be available")
                .push(RecordedAuthorityCommit {
                    observed_planning_revision: commit.observed_planning_revision,
                    task_ids: commit
                        .task_ledger
                        .tasks
                        .iter()
                        .map(|task| task.id.clone())
                        .collect(),
                    next_task_id: commit
                        .queue_projection
                        .next_task
                        .as_ref()
                        .map(|task| task.task_id.clone()),
                });
            let next_revision = commit.observed_planning_revision.unwrap_or(0) + 1;
            *self
                .snapshot
                .lock()
                .expect("snapshot lock should be available") =
                Some(PlanningTaskAuthoritySnapshot {
                    planning_revision: next_revision,
                    task_ledger: commit.task_ledger.clone(),
                    queue_projection: commit.queue_projection.clone(),
                });
            Ok(PlanningTaskAuthorityCommitResult::Committed {
                planning_revision: next_revision,
            })
        }

        fn clear_task_authority_snapshot(&self, _workspace_dir: &str) -> Result<()> {
            *self
                .snapshot
                .lock()
                .expect("snapshot lock should be available") = None;
            Ok(())
        }
    }

    #[test]
    fn applies_tracked_task_ledger_and_rebuilds_queue_projection() {
        let workspace_dir = create_temp_workspace("task-ledger-apply-success");
        write_bootstrap_candidate_workspace(&workspace_dir);
        fs::write(
            Path::new(&workspace_dir).join(TASK_LEDGER_FILE_PATH),
            task_ledger_with_one_ready_task(),
        )
        .expect("tracked task ledger should write");

        let result = PlanningTaskLedgerApplyService::new(
            Arc::new(FilesystemPlanningWorkspaceAdapter::new()),
            PlanningValidationService::new(),
            PriorityQueueService::new(),
        )
        .apply_tracked_task_ledger(&workspace_dir)
        .expect("task-ledger apply should succeed");

        let queue_projection =
            fs::read_to_string(Path::new(&workspace_dir).join(QUEUE_SNAPSHOT_FILE_PATH))
                .expect("queue projection should exist");

        assert!(result.applied());
        assert_eq!(
            result.applied_paths,
            vec![
                TASK_LEDGER_FILE_PATH.to_string(),
                QUEUE_SNAPSHOT_FILE_PATH.to_string()
            ]
        );
        assert!(queue_projection.contains("\"task_id\": \"task-1\""));

        fs::remove_dir_all(workspace_dir).expect("temp workspace should be removed");
    }

    #[test]
    fn apply_updates_active_files_even_when_authority_snapshot_exists() {
        let active = bootstrap_workspace_record();
        let mut candidate = active.clone();
        candidate.task_ledger_json = Some(task_ledger_with_one_ready_task());
        let workspace_port = Arc::new(RecordingPlanningWorkspacePort::new(
            active.clone(),
            candidate,
        ));
        let repository_port = Arc::new(RecordingTaskRepositoryPort::with_snapshot(Some(
            PlanningTaskAuthoritySnapshot {
                planning_revision: 7,
                task_ledger: serde_json::from_str::<TaskLedgerDocument>(
                    active
                        .task_ledger_json
                        .as_deref()
                        .expect("bootstrap task catalog should exist"),
                )
                .expect("bootstrap task catalog should parse"),
                queue_projection: empty_queue_projection(),
            },
        )));

        let result = PlanningTaskLedgerApplyService::with_task_repository(
            workspace_port.clone(),
            PlanningValidationService::new(),
            PriorityQueueService::new(),
            repository_port.clone(),
        )
        .apply_tracked_task_ledger("memory-workspace")
        .expect("task catalog apply should succeed");

        let committed_records = workspace_port.committed_records();
        let replaced_files = workspace_port.replaced_files();
        let authority_commits = repository_port.commits();
        assert!(result.applied());
        assert!(committed_records.is_empty());
        assert_eq!(
            replaced_files
                .iter()
                .map(|(path, _)| path.as_str())
                .collect::<Vec<_>>(),
            vec![TASK_LEDGER_FILE_PATH, QUEUE_SNAPSHOT_FILE_PATH]
        );
        assert!(
            replaced_files[0]
                .1
                .as_ref()
                .expect("task catalog should be committed")
                .contains("\"id\": \"task-1\"")
        );
        assert!(
            replaced_files[1]
                .1
                .as_ref()
                .expect("queue projection should be committed")
                .contains("\"task_id\": \"task-1\"")
        );
        assert_eq!(authority_commits.len(), 1);
        assert_eq!(authority_commits[0].observed_planning_revision, Some(7));
        assert_eq!(authority_commits[0].task_ids, vec!["task-1"]);
    }

    #[test]
    fn apply_initializes_authority_snapshot_when_missing() {
        let active = bootstrap_workspace_record();
        let mut candidate = active.clone();
        candidate.task_ledger_json = Some(task_ledger_with_one_ready_task());
        let workspace_port = Arc::new(RecordingPlanningWorkspacePort::new(active, candidate));
        let repository_port = Arc::new(RecordingTaskRepositoryPort::with_snapshot(None));

        let result = PlanningTaskLedgerApplyService::with_task_repository(
            workspace_port.clone(),
            PlanningValidationService::new(),
            PriorityQueueService::new(),
            repository_port.clone(),
        )
        .apply_tracked_task_ledger("memory-workspace")
        .expect("task catalog apply should succeed");

        let committed_records = workspace_port.committed_records();
        let replaced_files = workspace_port.replaced_files();
        let authority_commits = repository_port.commits();
        assert!(result.applied());
        assert!(committed_records.is_empty());
        assert_eq!(
            replaced_files
                .iter()
                .map(|(path, _)| path.as_str())
                .collect::<Vec<_>>(),
            vec![TASK_LEDGER_FILE_PATH, QUEUE_SNAPSHOT_FILE_PATH]
        );
        assert_eq!(authority_commits.len(), 1);
        assert_eq!(authority_commits[0].observed_planning_revision, None);
        assert_eq!(authority_commits[0].task_ids, vec!["task-1"]);
        assert_eq!(authority_commits[0].next_task_id.as_deref(), Some("task-1"));
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
}
