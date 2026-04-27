use std::sync::Arc;

use anyhow::Result;

use crate::application::port::outbound::planning_task_repository_port::PlanningTaskRepositoryPort;
use crate::application::port::outbound::planning_workspace_port::PlanningWorkspaceLoadRecord;
use crate::application::port::outbound::planning_workspace_port::PlanningWorkspacePort;
use crate::application::service::planning::shared::contract::{
    DIRECTIONS_FILE_PATH, QUEUE_SNAPSHOT_FILE_PATH, RESULT_OUTPUT_FILE_PATH, TASK_LEDGER_FILE_PATH,
    TASK_LEDGER_SCHEMA_FILE_PATH, canonical_active_planning_file_path,
};
use crate::application::service::priority_queue_service::PriorityQueueService;
use crate::domain::planning::PlanningWorkspaceFiles;

use super::accepted_ledger::{PlanningAcceptedLedgerCommitRequest, commit_accepted_task_ledger};
pub use super::ledger_recovery::PlanningQueueProjectionAction;
use super::ledger_recovery::{
    PlanningLedgerRejectionRequest, recover_queue_projection, reject_invalid_task_ledger,
};
pub use super::prompt::{
    PlanningRepairPromptHandoff, PlanningRepairRetryReason, build_planning_repair_prompt,
};
pub use super::protected_restore::PlanningProtectedFileRestoration;
use super::protected_restore::{
    ReconciledPlanningWorkspaceFiles, restore_protected_workspace_files,
};
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
    pub task_ledger_json: Option<String>,
    pub task_ledger_schema_json: Option<String>,
    pub result_output_markdown: Option<String>,
    pub queue_snapshot_json: Option<String>,
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
    pub rejected_task_ledger: bool,
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
    pub task_ledger_schema_json: String,
    pub accepted_task_ledger_json: String,
    pub rejected_task_ledger_json: Option<String>,
    pub rejected_archive_path: Option<String>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(super) struct PlanningChangeSet {
    pub(super) directions_changed: bool,
    pub(super) task_ledger_changed: bool,
    pub(super) task_ledger_schema_changed: bool,
    pub(super) result_output_changed: bool,
    pub(super) queue_projection_changed: bool,
}

impl PlanningChangeSet {
    fn from_paths(paths: &[String]) -> Self {
        let mut change_set = Self::default();
        for path in paths {
            match canonical_active_planning_file_path(path) {
                Some(DIRECTIONS_FILE_PATH) => change_set.directions_changed = true,
                Some(TASK_LEDGER_FILE_PATH) => change_set.task_ledger_changed = true,
                Some(TASK_LEDGER_SCHEMA_FILE_PATH) => change_set.task_ledger_schema_changed = true,
                Some(RESULT_OUTPUT_FILE_PATH) => change_set.result_output_changed = true,
                Some(QUEUE_SNAPSHOT_FILE_PATH) => change_set.queue_projection_changed = true,
                _ => {}
            }
        }
        change_set
    }

    fn has_relevant_changes(self) -> bool {
        self.directions_changed
            || self.task_ledger_changed
            || self.task_ledger_schema_changed
            || self.result_output_changed
            || self.queue_projection_changed
    }
}

impl PlanningReconciliationService {
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
            task_ledger_schema_json: workspace_record.task_ledger_schema_json,
            result_output_markdown: workspace_record.result_output_markdown,
            queue_snapshot_json: workspace_record.queue_snapshot_json,
        })
    }

    pub fn reconcile_after_turn(
        &self,
        workspace_dir: &str,
        turn_id: &str,
        changed_planning_file_paths: &[String],
        execution_snapshot: &PlanningExecutionSnapshot,
    ) -> Result<PlanningReconciliationResult> {
        let change_set = PlanningChangeSet::from_paths(changed_planning_file_paths);
        if !change_set.has_relevant_changes() {
            return Ok(PlanningReconciliationResult::default());
        }

        let mut result = PlanningReconciliationResult::default();
        let candidate_workspace_record = self
            .planning_workspace_port
            .load_planning_workspace_candidate_files(workspace_dir)?;

        let protected_restore = restore_protected_workspace_files(
            self.planning_workspace_port.as_ref(),
            workspace_dir,
            turn_id,
            &candidate_workspace_record,
            execution_snapshot,
            change_set,
        )?;
        result
            .restored_protected_files
            .extend(protected_restore.restorations);
        result.notices.extend(protected_restore.notices);
        let reconciled_workspace = protected_restore.workspace_files;

        if change_set.task_ledger_changed {
            self.reconcile_task_ledger(
                workspace_dir,
                turn_id,
                &candidate_workspace_record,
                execution_snapshot,
                &reconciled_workspace,
                &mut result,
            )?;
        } else if change_set.queue_projection_changed {
            let queue_recovery = recover_queue_projection(
                candidate_workspace_record.queue_snapshot_json.as_deref(),
                execution_snapshot,
            );
            result.queue_projection_action = queue_recovery.action;
            result.notices.extend(queue_recovery.notices);
        }

        if !change_set.task_ledger_changed
            && (change_set.queue_projection_changed || !result.restored_protected_files.is_empty())
        {
            self.planning_workspace_port
                .commit_planning_workspace_files(
                    workspace_dir,
                    &execution_snapshot_to_workspace_record(execution_snapshot),
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
        reconciled_workspace: &ReconciledPlanningWorkspaceFiles,
        result: &mut PlanningReconciliationResult,
    ) -> Result<()> {
        let task_ledger_candidate = workspace_record
            .task_ledger_json
            .as_deref()
            .unwrap_or_default();
        let validation_result =
            self.planning_validation_service
                .validate_workspace_files(PlanningWorkspaceFiles {
                    directions_toml: &reconciled_workspace.directions_toml,
                    task_ledger_json: task_ledger_candidate,
                    task_ledger_schema_json: &reconciled_workspace.task_ledger_schema_json,
                    result_output_markdown: &reconciled_workspace.result_output_markdown,
                });

        if validation_result.is_valid() {
            let accepted_commit = commit_accepted_task_ledger(
                self.planning_workspace_port.as_ref(),
                self.planning_task_repository_port.as_ref(),
                &self.priority_queue_service,
                PlanningAcceptedLedgerCommitRequest {
                    workspace_dir,
                    workspace_record,
                    execution_snapshot,
                    validation_result: &validation_result,
                },
            )?;
            result.queue_projection_action = Some(accepted_commit.queue_projection_action);
            result.notices.extend(accepted_commit.notices);
            return Ok(());
        }

        let rejection = reject_invalid_task_ledger(
            self.planning_workspace_port.as_ref(),
            PlanningLedgerRejectionRequest {
                workspace_dir,
                turn_id,
                workspace_record,
                execution_snapshot,
                reconciled_workspace,
                validation_result: &validation_result,
            },
        )?;
        result.rejected_task_ledger = true;
        result.rejected_archive_path = rejection.rejected_archive_path;
        result.queue_projection_action = rejection.queue_projection_action;
        result.repair_request = Some(rejection.repair_request);
        result.notices.extend(rejection.notices);

        Ok(())
    }
}

pub(super) fn execution_snapshot_to_workspace_record(
    execution_snapshot: &PlanningExecutionSnapshot,
) -> PlanningWorkspaceLoadRecord {
    PlanningWorkspaceLoadRecord {
        directions_toml: execution_snapshot.directions_toml.clone(),
        task_ledger_json: execution_snapshot.task_ledger_json.clone(),
        task_ledger_schema_json: execution_snapshot.task_ledger_schema_json.clone(),
        queue_snapshot_json: execution_snapshot.queue_snapshot_json.clone(),
        result_output_markdown: execution_snapshot.result_output_markdown.clone(),
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;
    use std::path::PathBuf;
    use std::process::Command;
    use std::time::{SystemTime, UNIX_EPOCH};

    use serde_json::json;

    use super::{
        PlanningExecutionSnapshot, PlanningQueueProjectionAction, PlanningReconciliationService,
        PlanningRepairPromptHandoff, PlanningRepairRetryReason, build_planning_repair_prompt,
    };
    use crate::adapter::outbound::filesystem::FilesystemPlanningWorkspaceAdapter;
    use crate::application::port::outbound::planning_workspace_port::{
        PlanningWorkspaceLoadRecord, PlanningWorkspacePort,
    };
    use crate::application::service::planning::authoring::bootstrap::{
        PlanningBootstrapMode, PlanningBootstrapService,
    };
    use crate::application::service::planning::runtime::validation::PlanningValidationService;
    use crate::application::service::planning::shared::contract::{
        DIRECTIONS_FILE_PATH, QUEUE_SNAPSHOT_FILE_PATH, TASK_LEDGER_FILE_PATH,
        TASK_LEDGER_SCHEMA_FILE_PATH,
    };
    use crate::application::service::priority_queue_service::PriorityQueueService;

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
        let artifacts =
            PlanningBootstrapService::new().build_artifacts_for_mode(PlanningBootstrapMode::Detail);
        let directions =
            toml::from_str(&artifacts.directions_toml).expect("bootstrap directions should parse");
        let task_ledger = serde_json::from_str(&artifacts.task_ledger_json)
            .expect("bootstrap task ledger should parse");
        let queue_projection = PriorityQueueService::new()
            .build_projection(&directions, &task_ledger)
            .expect("bootstrap queue projection should build");
        let queue_snapshot_json = serde_json::to_string_pretty(&queue_projection)
            .expect("queue projection should serialize");
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
            planning_dir.join("queue.snapshot.json"),
            &queue_snapshot_json,
        )
        .expect("queue projection should write");
        fs::write(
            planning_dir.join("result-output.md"),
            &artifacts.result_output_markdown,
        )
        .expect("result output should write");

        PlanningExecutionSnapshot {
            directions_toml: Some(artifacts.directions_toml),
            task_ledger_json: Some(artifacts.task_ledger_json),
            task_ledger_schema_json: Some(artifacts.task_ledger_schema_json),
            result_output_markdown: Some(artifacts.result_output_markdown),
            queue_snapshot_json: Some(queue_snapshot_json),
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

    struct TempGitRepo {
        root: PathBuf,
        worktree_root: PathBuf,
    }

    impl TempGitRepo {
        fn new(label: &str) -> Self {
            let unique = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("system clock should be valid")
                .as_nanos();
            let root = std::env::temp_dir().join(format!("{label}-{unique}"));
            let repo_root = root.join("repo");
            let worktree_root = root.join("worktrees").join("linked");
            fs::create_dir_all(&repo_root).expect("temp repo root should exist");
            run_git(&repo_root, &["init", "-q"]);
            run_git(&repo_root, &["config", "user.name", "RefinedStone"]);
            run_git(
                &repo_root,
                &["config", "user.email", "chem.en.9273@gmail.com"],
            );
            fs::write(repo_root.join("README.md"), "seed\n").expect("seed file should write");
            run_git(&repo_root, &["add", "README.md"]);
            run_git(&repo_root, &["commit", "-qm", "init"]);
            fs::create_dir_all(
                worktree_root
                    .parent()
                    .expect("worktree parent should exist"),
            )
            .expect("worktree parent should exist");
            run_git(
                &repo_root,
                &[
                    "worktree",
                    "add",
                    "-b",
                    "feature/worktree",
                    worktree_root.to_str().expect("valid worktree path"),
                ],
            );

            Self {
                root,
                worktree_root,
            }
        }
    }

    impl Drop for TempGitRepo {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    fn run_git(repo_root: &Path, args: &[&str]) {
        let status = Command::new("git")
            .current_dir(repo_root)
            .args(args)
            .status()
            .expect("git command should spawn");
        assert!(
            status.success(),
            "git command should succeed: git {}",
            args.join(" ")
        );
    }

    #[test]
    fn valid_task_ledger_change_rebuilds_queue_projection() {
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

        let queue_projection = fs::read_to_string(
            Path::new(&workspace_dir).join(".codex-exec-loop/planning/queue.snapshot.json"),
        )
        .expect("queue projection should exist");

        assert_eq!(
            result.queue_projection_action,
            Some(PlanningQueueProjectionAction::RebuiltFromAcceptedPlanning)
        );
        assert!(!result.rejected_task_ledger);
        assert!(queue_projection.contains("\"task_id\": \"task-1\""));

        fs::remove_dir_all(workspace_dir).expect("temp workspace should be removed");
    }

    #[test]
    fn git_backed_task_ledger_change_accepts_tracked_candidate_into_authority() {
        let repo = TempGitRepo::new("planning-reconcile-git-backed");
        let workspace_dir = repo.worktree_root.to_str().expect("valid worktree path");
        let artifacts =
            PlanningBootstrapService::new().build_artifacts_for_mode(PlanningBootstrapMode::Detail);
        let empty_task_ledger_json = serde_json::to_string_pretty(&json!({
            "version": 1,
            "tasks": []
        }))
        .expect("empty task ledger should serialize");
        let directions =
            toml::from_str(&artifacts.directions_toml).expect("bootstrap directions should parse");
        let empty_task_ledger =
            serde_json::from_str(&empty_task_ledger_json).expect("empty task ledger should parse");
        let empty_queue_projection = PriorityQueueService::new()
            .build_projection(&directions, &empty_task_ledger)
            .expect("empty queue projection should build");
        let empty_queue_snapshot_json = serde_json::to_string_pretty(&empty_queue_projection)
            .expect("empty queue projection should serialize");
        FilesystemPlanningWorkspaceAdapter::new()
            .commit_planning_workspace_files(
                workspace_dir,
                &PlanningWorkspaceLoadRecord {
                    directions_toml: Some(artifacts.directions_toml.clone()),
                    task_ledger_json: Some(empty_task_ledger_json.clone()),
                    task_ledger_schema_json: Some(artifacts.task_ledger_schema_json.clone()),
                    queue_snapshot_json: Some(empty_queue_snapshot_json),
                    result_output_markdown: Some(artifacts.result_output_markdown.clone()),
                },
            )
            .expect("active authority should seed");

        fs::create_dir_all(repo.worktree_root.join(".codex-exec-loop/planning"))
            .expect("tracked planning directory should exist");
        fs::write(
            repo.worktree_root.join(TASK_LEDGER_FILE_PATH),
            serde_json::to_string_pretty(&json!({
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
                        "source_turn_id": "turn-git-backed",
                        "updated_at": "2026-04-23T10:00:00Z"
                    }
                ]
            }))
            .expect("valid task ledger should serialize"),
        )
        .expect("tracked task ledger should write");

        let execution_snapshot = service()
            .load_execution_snapshot(workspace_dir)
            .expect("execution snapshot should load from active authority");
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(
                execution_snapshot
                    .task_ledger_json
                    .as_deref()
                    .expect("execution task ledger should exist")
            )
            .expect("execution task ledger should parse"),
            serde_json::from_str::<serde_json::Value>(&empty_task_ledger_json)
                .expect("empty task ledger should parse")
        );

        let result = service()
            .reconcile_after_turn(
                workspace_dir,
                "turn-git-backed",
                &[TASK_LEDGER_FILE_PATH.to_string()],
                &execution_snapshot,
            )
            .expect("reconciliation should succeed");
        let active_workspace = FilesystemPlanningWorkspaceAdapter::new()
            .load_planning_workspace_files(workspace_dir)
            .expect("active authority should load");

        assert_eq!(
            result.queue_projection_action,
            Some(PlanningQueueProjectionAction::RebuiltFromAcceptedPlanning)
        );
        assert!(!result.rejected_task_ledger);
        assert!(
            active_workspace
                .task_ledger_json
                .as_deref()
                .is_some_and(|body| body.contains("\"task-1\""))
        );
        assert!(
            active_workspace
                .queue_snapshot_json
                .as_deref()
                .is_some_and(|body| body.contains("\"task_id\": \"task-1\""))
        );
    }

    #[test]
    fn invalid_task_ledger_change_is_archived_and_restored() {
        let workspace_dir = create_temp_workspace("planning-reconcile-invalid");
        let execution_snapshot = write_bootstrap_workspace(&workspace_dir);
        let planning_dir = Path::new(&workspace_dir).join(".codex-exec-loop/planning");
        fs::write(
            planning_dir.join("queue.snapshot.json"),
            "{\"next_task\":\"broken\"}",
        )
        .expect("mutated queue projection should write");
        fs::write(planning_dir.join("task-ledger.json"), "{ invalid json")
            .expect("invalid task ledger should write");

        let result = service()
            .reconcile_after_turn(
                &workspace_dir,
                "turn-2",
                &[
                    TASK_LEDGER_FILE_PATH.to_string(),
                    QUEUE_SNAPSHOT_FILE_PATH.to_string(),
                ],
                &execution_snapshot,
            )
            .expect("reconciliation should succeed");

        let restored_task_ledger = fs::read_to_string(planning_dir.join("task-ledger.json"))
            .expect("restored task ledger should be readable");
        let restored_queue_projection =
            fs::read_to_string(planning_dir.join("queue.snapshot.json"))
                .expect("restored queue projection should be readable");

        assert!(result.rejected_task_ledger);
        assert!(result.rejected_archive_path.is_some());
        assert!(result.repair_request.is_some());
        assert_eq!(
            result.queue_projection_action,
            Some(PlanningQueueProjectionAction::RestoredFromExecutionSnapshot)
        );
        assert_eq!(
            restored_task_ledger,
            execution_snapshot
                .task_ledger_json
                .expect("execution snapshot should keep the accepted task ledger")
        );
        assert_eq!(
            restored_queue_projection,
            execution_snapshot
                .queue_snapshot_json
                .expect("execution snapshot should keep the accepted queue projection")
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
    fn repair_prompt_includes_validation_errors_and_rejected_excerpt() {
        let prompt = build_planning_repair_prompt(
            &super::PlanningRepairRequest {
                failure_summary: "failed to parse task-ledger.json: expected value".to_string(),
                validation_errors: vec![
                    "failed to parse task-ledger.json: expected value".to_string(),
                    "task-ledger.schema.json must not be blank".to_string(),
                ],
                directions_toml: "version = 1".to_string(),
                task_ledger_schema_json: "{\"type\":\"object\"}".to_string(),
                accepted_task_ledger_json: "{\"version\":1,\"tasks\":[]}".to_string(),
                rejected_task_ledger_json: Some("{ invalid json".to_string()),
                rejected_archive_path: Some(
                    "/tmp/workspace/.codex-exec-loop/planning/rejected/turn-1/task-ledger.json"
                        .to_string(),
                ),
            },
            None,
            1,
            2,
            Some(PlanningRepairRetryReason::TaskLedgerStillInvalid),
        );

        assert!(prompt.contains("planning repair 1/2"));
        assert!(prompt.contains("failed to parse task-ledger.json"));
        assert!(prompt.contains("rejected archive"));
        assert!(prompt.contains("Rejected candidate excerpt"));
        assert!(prompt.contains("수정했지만 여전히 유효하지 않습니다"));
    }

    #[test]
    fn repair_prompt_surfaces_previous_handoff_and_changed_task_context_from_large_ledger() {
        let filler_tasks = (0..40)
            .map(|index| {
                json!({
                    "id": format!("filler-task-{index:02}"),
                    "direction_id": "example-direction",
                    "direction_relation_note": format!("Filler relation note {index}"),
                    "title": format!("Filler task {index}"),
                    "description": "This filler task makes the accepted ledger large enough that naive truncation would hide the real repair targets.".repeat(3),
                    "status": "done",
                    "base_priority": 10,
                    "dynamic_priority_delta": 0,
                    "priority_reason": "",
                    "depends_on": [],
                    "blocked_by": [],
                    "created_by": "llm",
                    "last_updated_by": "llm",
                    "source_turn_id": null,
                    "updated_at": "2026-04-22T00:00:00Z"
                })
            })
            .collect::<Vec<_>>();
        let accepted_task_ledger_json = serde_json::to_string_pretty(&json!({
            "version": 1,
            "tasks": filler_tasks.iter().cloned().chain([
                json!({
                    "id": "context-first-bridge-adapter-attachment-event-reuse",
                    "direction_id": "context-first-architecture-and-doc-coherence",
                    "direction_relation_note": "Current queue head before repair.",
                    "title": "Reuse attachment event and profiles across remaining bridge adapters",
                    "description": "Carry the same attachment truth through remaining bridge adapters.",
                    "status": "ready",
                    "base_priority": 87,
                    "dynamic_priority_delta": 2,
                    "priority_reason": "Current top executable task.",
                    "depends_on": [],
                    "blocked_by": [],
                    "created_by": "llm",
                    "last_updated_by": "llm",
                    "source_turn_id": null,
                    "updated_at": "2026-04-22T23:30:00Z"
                }),
                json!({
                    "id": "terminal-bridge-local-spike-readiness-gate",
                    "direction_id": "terminal-agent-bridge-research-and-capability-boundary",
                    "direction_relation_note": "Immediate gate before implementation.",
                    "title": "Gate tmux local-attach spike on capability audit and evidence",
                    "description": "Hold the local spike until evidence exists.",
                    "status": "blocked",
                    "base_priority": 89,
                    "dynamic_priority_delta": 2,
                    "priority_reason": "Research gate remains closed.",
                    "depends_on": [],
                    "blocked_by": [],
                    "created_by": "llm",
                    "last_updated_by": "llm",
                    "source_turn_id": null,
                    "updated_at": "2026-04-22T23:50:00Z"
                })
            ]).collect::<Vec<_>>()
        }))
        .expect("accepted task ledger should serialize");
        let rejected_task_ledger_json = serde_json::to_string_pretty(&json!({
            "version": 1,
            "tasks": filler_tasks.into_iter().chain([
                json!({
                    "id": "context-first-bridge-adapter-attachment-event-reuse",
                    "direction_id": "context-first-architecture-and-doc-coherence",
                    "direction_relation_note": "Repair candidate incorrectly left the old queue head untouched.",
                    "title": "Reuse attachment event and profiles across remaining bridge adapters",
                    "description": "Carry the same attachment truth through remaining bridge adapters.",
                    "status": "ready",
                    "base_priority": 87,
                    "dynamic_priority_delta": 2,
                    "priority_reason": "Current top executable task.",
                    "depends_on": [],
                    "blocked_by": [],
                    "created_by": "llm",
                    "last_updated_by": "llm",
                    "source_turn_id": null,
                    "updated_at": "2026-04-22T23:30:00Z"
                }),
                json!({
                    "id": "terminal-bridge-local-spike-readiness-gate",
                    "direction_id": "terminal-agent-bridge-research-and-capability-boundary",
                    "direction_relation_note": "Immediate gate before implementation.",
                    "title": "Gate tmux local-attach spike on capability audit and evidence",
                    "description": "Hold the local spike until evidence exists.",
                    "status": "blocked",
                    "base_priority": 89,
                    "dynamic_priority_delta": 2,
                    "priority_reason": "Research gate remains closed.",
                    "depends_on": [],
                    "blocked_by": [],
                    "created_by": "llm",
                    "last_updated_by": "llm",
                    "source_turn_id": null,
                    "updated_at": "2026-04-22T23:50:00Z"
                }),
                json!({
                    "id": "terminal-bridge-primary-implementation-slice",
                    "direction_id": "terminal-agent-bridge-research-and-capability-boundary",
                    "direction_relation_note": "First real implementation slice.",
                    "title": "Implement the first real terminal bridge slice",
                    "description": "Start the first real implementation slice.",
                    "status": "blocked",
                    "base_priority": 90,
                    "dynamic_priority_delta": 2,
                    "priority_reason": "Implementation waits for readiness gate.",
                    "depends_on": ["terminal-bridge-local-spike-readiness-gate"],
                    "blocked_by": ["terminal-bridge-local-spike-readiness-gate"],
                    "created_by": "llm",
                    "last_updated_by": "llm",
                    "source_turn_id": null,
                    "updated_at": "2026-04-22T23:50:00Z"
                })
            ]).collect::<Vec<_>>()
        }))
        .expect("rejected task ledger should serialize");
        let prompt = build_planning_repair_prompt(
            &super::PlanningRepairRequest {
                failure_summary: "task terminal-bridge-primary-implementation-slice cannot list terminal-bridge-local-spike-readiness-gate in both depends_on and blocked_by".to_string(),
                validation_errors: vec![
                    "task terminal-bridge-primary-implementation-slice cannot list terminal-bridge-local-spike-readiness-gate in both depends_on and blocked_by".to_string(),
                ],
                directions_toml: "version = 1".to_string(),
                task_ledger_schema_json: "{\"type\":\"object\"}".to_string(),
                accepted_task_ledger_json,
                rejected_task_ledger_json: Some(rejected_task_ledger_json),
                rejected_archive_path: Some(
                    "/tmp/workspace/.codex-exec-loop/planning/rejected/turn-1/task-ledger.json"
                        .to_string(),
                ),
            },
            Some(PlanningRepairPromptHandoff {
                task_id: "context-first-bridge-adapter-attachment-event-reuse",
                task_title: "Reuse attachment event and profiles across remaining bridge adapters",
                updated_at: "2026-04-22T23:30:00Z",
                status_label: "ready",
            }),
            1,
            2,
            Some(PlanningRepairRetryReason::TaskLedgerStillInvalid),
        );

        assert!(prompt.contains("직전에 main session으로 넘긴 task:"));
        assert!(prompt.contains("Current accepted `task-ledger.json` focus"));
        assert!(prompt.contains("Rejected candidate focus"));
        assert!(prompt.contains("context-first-bridge-adapter-attachment-event-reuse"));
        assert!(prompt.contains("terminal-bridge-primary-implementation-slice"));
        assert!(prompt.contains("terminal-bridge-local-spike-readiness-gate"));
    }

    #[test]
    fn changed_directions_are_restored_from_execution_snapshot() {
        let workspace_dir = create_temp_workspace("planning-reconcile-directions");
        let execution_snapshot = write_bootstrap_workspace(&workspace_dir);
        let planning_dir = Path::new(&workspace_dir).join(".codex-exec-loop/planning");
        fs::write(
            planning_dir.join("directions.toml"),
            "version = 1\n[[directions]]\nid = \"mutated\"\ntitle = \"Mutated\"\nsummary = \"mutated\"\nsuccess_criteria = [\"mutated\"]\nstate = \"active\"\n",
        )
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
        assert_eq!(result.restored_protected_files.len(), 1);
        assert_eq!(
            result.restored_protected_files[0].relative_path,
            DIRECTIONS_FILE_PATH
        );
        assert!(
            result.restored_protected_files[0]
                .archived_candidate_path
                .as_deref()
                .is_some()
        );
        assert_eq!(result.queue_projection_action, None);

        fs::remove_dir_all(workspace_dir).expect("temp workspace should be removed");
    }

    #[test]
    fn deleted_directions_are_restored_from_execution_snapshot() {
        let workspace_dir = create_temp_workspace("planning-reconcile-directions-delete");
        let execution_snapshot = write_bootstrap_workspace(&workspace_dir);
        let planning_dir = Path::new(&workspace_dir).join(".codex-exec-loop/planning");
        fs::remove_file(planning_dir.join("directions.toml"))
            .expect("directions should be deleted to simulate protected-file removal");

        let result = service()
            .reconcile_after_turn(
                &workspace_dir,
                "turn-delete-directions",
                &[DIRECTIONS_FILE_PATH.to_string()],
                &execution_snapshot,
            )
            .expect("reconciliation should succeed");

        let restored_directions = fs::read_to_string(planning_dir.join("directions.toml"))
            .expect("deleted directions should be restored");

        assert_eq!(
            restored_directions,
            execution_snapshot
                .directions_toml
                .expect("execution snapshot should keep the accepted directions")
        );
        assert_eq!(result.restored_protected_files.len(), 1);
        assert_eq!(
            result.restored_protected_files[0].relative_path,
            DIRECTIONS_FILE_PATH
        );
        assert_eq!(
            result.restored_protected_files[0].archived_candidate_path,
            None
        );
        assert_eq!(result.queue_projection_action, None);

        fs::remove_dir_all(workspace_dir).expect("temp workspace should be removed");
    }

    #[test]
    fn task_ledger_acceptance_uses_restored_schema_baseline() {
        let workspace_dir = create_temp_workspace("planning-reconcile-schema-restore");
        let execution_snapshot = write_bootstrap_workspace(&workspace_dir);
        let planning_dir = Path::new(&workspace_dir).join(".codex-exec-loop/planning");
        let valid_task_ledger = serde_json::to_string_pretty(&json!({
            "version": 1,
            "tasks": [
                {
                    "id": "task-restore-schema",
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
                    "source_turn_id": "turn-restore-schema",
                    "updated_at": "2026-04-09T10:00:00Z"
                }
            ]
        }))
        .expect("valid task ledger should serialize");
        fs::write(planning_dir.join("task-ledger.schema.json"), "")
            .expect("mutated schema should write");
        fs::write(planning_dir.join("task-ledger.json"), valid_task_ledger)
            .expect("task ledger candidate should write");

        let result = service()
            .reconcile_after_turn(
                &workspace_dir,
                "turn-schema-restore",
                &[
                    TASK_LEDGER_SCHEMA_FILE_PATH.to_string(),
                    TASK_LEDGER_FILE_PATH.to_string(),
                ],
                &execution_snapshot,
            )
            .expect("reconciliation should succeed");

        let restored_schema = fs::read_to_string(planning_dir.join("task-ledger.schema.json"))
            .expect("restored schema should read");
        let queue_projection = fs::read_to_string(planning_dir.join("queue.snapshot.json"))
            .expect("rebuilt queue projection should read");

        assert_eq!(
            restored_schema,
            execution_snapshot
                .task_ledger_schema_json
                .expect("execution snapshot should keep the accepted task-ledger schema")
        );
        assert_eq!(
            result.queue_projection_action,
            Some(PlanningQueueProjectionAction::RebuiltFromAcceptedPlanning)
        );
        assert!(!result.rejected_task_ledger);
        assert!(queue_projection.contains("\"task_id\": \"task-restore-schema\""));

        fs::remove_dir_all(workspace_dir).expect("temp workspace should be removed");
    }

    #[test]
    fn queue_projection_change_without_task_ledger_change_is_restored() {
        let workspace_dir = create_temp_workspace("planning-reconcile-queue-only");
        let execution_snapshot = write_bootstrap_workspace(&workspace_dir);
        let planning_dir = Path::new(&workspace_dir).join(".codex-exec-loop/planning");
        fs::write(
            planning_dir.join("queue.snapshot.json"),
            "{\"next_task\":\"stale\"}",
        )
        .expect("mutated queue projection should write");

        let result = service()
            .reconcile_after_turn(
                &workspace_dir,
                "turn-queue-only",
                &[QUEUE_SNAPSHOT_FILE_PATH.to_string()],
                &execution_snapshot,
            )
            .expect("reconciliation should succeed");

        let restored_queue_projection =
            fs::read_to_string(planning_dir.join("queue.snapshot.json"))
                .expect("restored queue projection should read");

        assert_eq!(
            result.queue_projection_action,
            Some(PlanningQueueProjectionAction::RestoredFromExecutionSnapshot)
        );
        assert_eq!(
            restored_queue_projection,
            execution_snapshot
                .queue_snapshot_json
                .expect("execution snapshot should keep the accepted queue projection")
        );

        fs::remove_dir_all(workspace_dir).expect("temp workspace should be removed");
    }

    #[test]
    fn absolute_queue_projection_path_is_canonicalized_for_change_detection() {
        let workspace_dir = create_temp_workspace("planning-reconcile-absolute-queue");
        let execution_snapshot = write_bootstrap_workspace(&workspace_dir);
        let planning_dir = Path::new(&workspace_dir).join(".codex-exec-loop/planning");
        fs::write(
            planning_dir.join("queue.snapshot.json"),
            "{\"next_task\":\"stale\"}",
        )
        .expect("mutated queue projection should write");

        let absolute_queue_projection_path = planning_dir
            .join("queue.snapshot.json")
            .display()
            .to_string();
        assert!(PlanningExecutionSnapshot::captures_path(
            absolute_queue_projection_path.as_str()
        ));

        let result = service()
            .reconcile_after_turn(
                &workspace_dir,
                "turn-absolute-queue",
                &[absolute_queue_projection_path],
                &execution_snapshot,
            )
            .expect("reconciliation should succeed");

        let restored_queue_projection =
            fs::read_to_string(planning_dir.join("queue.snapshot.json"))
                .expect("restored queue projection should read");

        assert_eq!(
            result.queue_projection_action,
            Some(PlanningQueueProjectionAction::RestoredFromExecutionSnapshot)
        );
        assert_eq!(
            restored_queue_projection,
            execution_snapshot
                .queue_snapshot_json
                .expect("execution snapshot should keep the accepted queue projection")
        );

        fs::remove_dir_all(workspace_dir).expect("temp workspace should be removed");
    }
}
