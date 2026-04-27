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
use crate::application::service::priority_queue_service::PriorityQueueService;
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
        let mut validation_result =
            self.planning_validation_service
                .validate_workspace_files(PlanningWorkspaceFiles {
                    directions_toml: &candidate_directions_toml,
                    task_ledger_json: required_workspace_body(
                        &active_workspace,
                        WorkspaceBody::TaskLedger,
                    )?,
                    task_ledger_schema_json: required_workspace_body(
                        &active_workspace,
                        WorkspaceBody::TaskLedgerSchema,
                    )?,
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
        let task_ledger = validation_result
            .task_ledger
            .as_ref()
            .expect("valid tracked directions should include task ledger");
        let queue_projection = self
            .priority_queue_service
            .build_projection(directions, task_ledger)
            .context("failed to rebuild planning queue after tracked directions apply")?;

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
                        "planning db changed while applying tracked directions (observed revision {observed_planning_revision}, current revision {current_planning_revision}); reload and retry"
                    ));
                }
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
    TaskLedger,
    TaskLedgerSchema,
    ResultOutput,
}

fn required_workspace_body(
    workspace: &PlanningWorkspaceLoadRecord,
    body: WorkspaceBody,
) -> Result<&str> {
    match body {
        WorkspaceBody::TaskLedger => workspace
            .task_ledger_json
            .as_deref()
            .ok_or_else(|| anyhow!("planning workspace is missing task-ledger.json")),
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
    use std::fs;
    use std::path::Path;

    use super::PlanningDirectionsApplyService;
    use crate::adapter::outbound::filesystem::FilesystemPlanningWorkspaceAdapter;
    use crate::application::port::outbound::planning_workspace_port::PlanningWorkspacePort;
    use crate::application::service::planning::authoring::bootstrap::{
        PlanningBootstrapMode, PlanningBootstrapService,
    };
    use crate::application::service::planning::runtime::validation::PlanningValidationService;
    use crate::application::service::planning::shared::contract::DIRECTIONS_FILE_PATH;
    use crate::application::service::planning::shared::contract::default_direction_detail_doc_path;

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
        fs::create_dir_all(planning_dir.join("prompts"))
            .expect("prompt directory should be created");
        fs::write(
            planning_dir.join("prompts/queue-idle-review.md"),
            "# Queue Idle Review Prompt\n",
        )
        .expect("prompt should write");
    }

    #[test]
    fn applies_tracked_directions_and_supporting_files_into_active_workspace() {
        let workspace_dir = create_temp_workspace("planning-directions-apply");
        write_bootstrap_candidate_workspace(&workspace_dir);
        let adapter = std::sync::Arc::new(FilesystemPlanningWorkspaceAdapter::new());
        let validation_service = PlanningValidationService::new();
        let service = PlanningDirectionsApplyService::new(adapter.clone(), validation_service);
        let bootstrap =
            PlanningBootstrapService::new().build_artifacts_for_mode(PlanningBootstrapMode::Detail);

        adapter
            .replace_planning_workspace_file(
                &workspace_dir,
                DIRECTIONS_FILE_PATH,
                Some(
                    "version = 1\n\n[queue_idle]\npolicy = \"review_and_enqueue\"\nprompt_path = \".codex-exec-loop/planning/prompts/queue-idle-review.md\"\n\n[[directions]]\nid = \"general-workstream\"\ntitle = \"General\"\nsummary = \"summary\"\nsuccess_criteria = [\"one\"]\nscope_hints = [\"two\"]\ndetail_doc_path = \"\"\nstate = \"active\"\n",
                ),
            )
            .expect("active directions should seed");
        adapter
            .replace_planning_workspace_file(
                &workspace_dir,
                ".codex-exec-loop/planning/task-ledger.json",
                Some(&bootstrap.task_ledger_json),
            )
            .expect("active task ledger should seed");
        adapter
            .replace_planning_workspace_file(
                &workspace_dir,
                ".codex-exec-loop/planning/task-ledger.schema.json",
                Some(&bootstrap.task_ledger_schema_json),
            )
            .expect("active schema should seed");
        adapter
            .replace_planning_workspace_file(
                &workspace_dir,
                ".codex-exec-loop/planning/result-output.md",
                Some(&bootstrap.result_output_markdown),
            )
            .expect("active result output should seed");

        let detail_doc_path = default_direction_detail_doc_path("general-workstream");
        fs::create_dir_all(
            Path::new(&workspace_dir)
                .join(&detail_doc_path)
                .parent()
                .expect("detail doc should have parent"),
        )
        .expect("detail doc parent should exist");
        fs::write(
            Path::new(&workspace_dir).join(&detail_doc_path),
            "# General workstream\n",
        )
        .expect("detail doc should write");
        let candidate_directions = format!(
            "version = 1\n\n[queue_idle]\npolicy = \"review_and_enqueue\"\nprompt_path = \".codex-exec-loop/planning/prompts/queue-idle-review.md\"\n\n[[directions]]\nid = \"general-workstream\"\ntitle = \"General\"\nsummary = \"summary\"\nsuccess_criteria = [\"one\"]\nscope_hints = [\"two\"]\ndetail_doc_path = \"{detail_doc_path}\"\nstate = \"active\"\n"
        );
        fs::write(
            Path::new(&workspace_dir).join(DIRECTIONS_FILE_PATH),
            candidate_directions,
        )
        .expect("candidate directions should write");

        let result = service
            .apply_tracked_directions(&workspace_dir)
            .expect("tracked directions should apply");

        assert!(result.applied());
        assert!(
            result
                .applied_paths
                .contains(&".codex-exec-loop/planning/prompts/queue-idle-review.md".to_string())
        );
        assert!(result.applied_paths.contains(&detail_doc_path));
        let active_directions = adapter
            .load_optional_planning_file(&workspace_dir, DIRECTIONS_FILE_PATH)
            .expect("active directions should load")
            .expect("active directions should exist");
        assert!(active_directions.contains("detail_doc_path"));
        let active_detail_doc = adapter
            .load_optional_planning_file(&workspace_dir, &detail_doc_path)
            .expect("detail doc should load")
            .expect("detail doc should exist");
        assert_eq!(active_detail_doc, "# General workstream\n");
    }

    #[test]
    fn blocks_tracked_directions_apply_when_candidate_supporting_file_is_missing() {
        let workspace_dir = create_temp_workspace("planning-directions-apply-invalid");
        write_bootstrap_candidate_workspace(&workspace_dir);
        let adapter = std::sync::Arc::new(FilesystemPlanningWorkspaceAdapter::new());
        let validation_service = PlanningValidationService::new();
        let service = PlanningDirectionsApplyService::new(adapter.clone(), validation_service);
        let bootstrap =
            PlanningBootstrapService::new().build_artifacts_for_mode(PlanningBootstrapMode::Detail);

        adapter
            .replace_planning_workspace_file(
                &workspace_dir,
                DIRECTIONS_FILE_PATH,
                Some("version = 1\n\n[queue_idle]\npolicy = \"review_and_enqueue\"\nprompt_path = \".codex-exec-loop/planning/prompts/queue-idle-review.md\"\n\n[[directions]]\nid = \"general-workstream\"\ntitle = \"General\"\nsummary = \"summary\"\nsuccess_criteria = [\"one\"]\nscope_hints = [\"two\"]\ndetail_doc_path = \"\"\nstate = \"active\"\n"),
            )
            .expect("active directions should seed");
        adapter
            .replace_planning_workspace_file(
                &workspace_dir,
                ".codex-exec-loop/planning/task-ledger.json",
                Some(&bootstrap.task_ledger_json),
            )
            .expect("active task ledger should seed");
        adapter
            .replace_planning_workspace_file(
                &workspace_dir,
                ".codex-exec-loop/planning/task-ledger.schema.json",
                Some(&bootstrap.task_ledger_schema_json),
            )
            .expect("active schema should seed");
        adapter
            .replace_planning_workspace_file(
                &workspace_dir,
                ".codex-exec-loop/planning/result-output.md",
                Some(&bootstrap.result_output_markdown),
            )
            .expect("active result output should seed");

        let candidate_directions = format!(
            "version = 1\n\n[queue_idle]\npolicy = \"review_and_enqueue\"\nprompt_path = \".codex-exec-loop/planning/prompts/queue-idle-review.md\"\n\n[[directions]]\nid = \"general-workstream\"\ntitle = \"General\"\nsummary = \"summary\"\nsuccess_criteria = [\"one\"]\nscope_hints = [\"two\"]\ndetail_doc_path = \"{}\"\nstate = \"active\"\n",
            default_direction_detail_doc_path("general-workstream")
        );
        fs::write(
            Path::new(&workspace_dir).join(DIRECTIONS_FILE_PATH),
            candidate_directions,
        )
        .expect("candidate directions should write");

        let result = service
            .apply_tracked_directions(&workspace_dir)
            .expect("invalid tracked directions should report validation");

        assert!(!result.applied());
        assert!(result.applied_paths.is_empty());
        assert!(
            result
                .validation_report
                .errors()
                .iter()
                .any(|issue| issue.code == "missing_detail_doc_file")
        );
    }
}
