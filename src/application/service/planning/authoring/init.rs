use std::collections::HashMap;
use std::sync::Arc;

use anyhow::{Result, anyhow};
use chrono::Utc;

use crate::application::port::outbound::planning_task_repository_port::{
    PlanningDirectionAuthorityCommit, PlanningTaskAuthorityCommit, PlanningTaskRepositoryPort,
};
use crate::application::port::outbound::planning_workspace_port::{
    PlanningDraftFileRecord, PlanningDraftLoadRecord, PlanningStagedFileRecord,
    PlanningWorkspacePort,
};
use crate::application::service::planning::shared::contract::RESULT_OUTPUT_FILE_PATH;
use crate::domain::planning::{
    DirectionCatalogDocument, PLANNING_FORMAT_VERSION, PlanningValidationReport,
    TaskAuthorityDocument,
};

use crate::application::service::planning::authoring::bootstrap::{
    PlanningBootstrapMode, PlanningBootstrapService,
};
use crate::application::service::planning::runtime::validation::PlanningValidationService;
use crate::domain::planning::PriorityQueueService;

#[derive(Clone)]
pub struct PlanningInitService {
    planning_workspace_port: Arc<dyn PlanningWorkspacePort>,
    planning_bootstrap_service: PlanningBootstrapService,
    planning_validation_service: PlanningValidationService,
    planning_task_repository_port: Arc<dyn PlanningTaskRepositoryPort>,
    priority_queue_service: PriorityQueueService,
}

#[derive(Debug, Clone)]
pub struct PlanningInitStageResult {
    pub mode: PlanningBootstrapMode,
    pub draft_name: String,
    pub draft_directory: String,
    pub staged_files: Vec<PlanningStagedFileRecord>,
    pub staged_file_count: usize,
    pub validation_report: PlanningValidationReport,
}

impl PlanningInitStageResult {
    pub fn is_valid(&self) -> bool {
        self.validation_report.is_valid()
    }

    pub fn status_text(&self) -> String {
        format!(
            "planning init staged / mode: {} / draft: {} / files: {} / validation: {}",
            match self.mode {
                PlanningBootstrapMode::Detail => "detail",
                PlanningBootstrapMode::Simple => "simple",
            },
            self.draft_name,
            self.staged_file_count,
            if self.is_valid() {
                "ok"
            } else {
                "needs attention"
            }
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanningDraftEditorFile {
    pub active_path: String,
    pub staged_path: String,
    pub body: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanningDraftEditorSession {
    pub draft_name: String,
    pub draft_directory: String,
    pub editable_files: Vec<PlanningDraftEditorFile>,
    pub validation_report: PlanningValidationReport,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanningDraftSaveResult {
    pub draft_name: String,
    pub validation_report: PlanningValidationReport,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanningDraftPromoteResult {
    pub draft_name: String,
    pub promoted_file_count: usize,
    pub validation_report: PlanningValidationReport,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanningWorkspaceInitResult {
    pub mode: PlanningBootstrapMode,
    pub created_file_count: usize,
    pub created_paths: Vec<String>,
}

impl PlanningInitService {
    #[cfg(test)]
    #[allow(dead_code)]
    pub fn new(
        planning_workspace_port: Arc<dyn PlanningWorkspacePort>,
        planning_bootstrap_service: PlanningBootstrapService,
        planning_validation_service: PlanningValidationService,
    ) -> Self {
        Self::with_task_repository(
            planning_workspace_port,
            planning_bootstrap_service,
            planning_validation_service,
            Arc::new(crate::application::port::outbound::planning_task_repository_port::NoopPlanningTaskRepositoryPort),
            PriorityQueueService::new(),
        )
    }

    pub fn with_task_repository(
        planning_workspace_port: Arc<dyn PlanningWorkspacePort>,
        planning_bootstrap_service: PlanningBootstrapService,
        planning_validation_service: PlanningValidationService,
        planning_task_repository_port: Arc<dyn PlanningTaskRepositoryPort>,
        priority_queue_service: PriorityQueueService,
    ) -> Self {
        Self {
            planning_workspace_port,
            planning_bootstrap_service,
            planning_validation_service,
            planning_task_repository_port,
            priority_queue_service,
        }
    }

    pub fn stage_simple_mode_draft(&self, workspace_dir: &str) -> Result<PlanningInitStageResult> {
        self.stage_draft(workspace_dir, PlanningBootstrapMode::Simple)
    }

    pub fn stage_manual_editor_session(
        &self,
        workspace_dir: &str,
    ) -> Result<PlanningDraftEditorSession> {
        self.stage_editor_session(workspace_dir, PlanningBootstrapMode::Detail)
    }

    pub fn load_manual_editor_session(
        &self,
        workspace_dir: &str,
        draft_name: &str,
    ) -> Result<PlanningDraftEditorSession> {
        let loaded = self
            .planning_workspace_port
            .load_planning_draft_files(workspace_dir, draft_name)?;
        let validation_report = self.validate_loaded_draft(workspace_dir, &loaded)?;

        Ok(PlanningDraftEditorSession {
            draft_name: loaded.draft_name,
            draft_directory: loaded.draft_directory,
            editable_files: loaded
                .staged_files
                .into_iter()
                .filter(|file| is_operator_editable_draft_path(file.active_path.as_str()))
                .map(|file| PlanningDraftEditorFile {
                    active_path: file.active_path,
                    staged_path: file.staged_path,
                    body: file.body,
                })
                .collect(),
            validation_report,
        })
    }

    pub fn has_planning_workspace(&self, workspace_dir: &str) -> Result<bool> {
        Ok(self
            .planning_workspace_port
            .load_planning_workspace_files(workspace_dir)?
            .has_any_files())
    }

    pub fn has_planning_candidate_workspace(&self, workspace_dir: &str) -> Result<bool> {
        Ok(self
            .planning_workspace_port
            .load_planning_workspace_candidate_files(workspace_dir)?
            .has_any_files())
    }

    pub fn initialize_simple_workspace(
        &self,
        workspace_dir: &str,
    ) -> Result<PlanningWorkspaceInitResult> {
        self.initialize_workspace(workspace_dir, PlanningBootstrapMode::Simple)
    }

    pub fn save_draft_editor_files(
        &self,
        workspace_dir: &str,
        draft_name: &str,
        files: &[PlanningDraftEditorFile],
    ) -> Result<PlanningDraftSaveResult> {
        let loaded = self.replace_and_load_draft_editor_files(workspace_dir, draft_name, files)?;
        Ok(PlanningDraftSaveResult {
            draft_name: draft_name.to_string(),
            validation_report: self.validate_loaded_draft(workspace_dir, &loaded)?,
        })
    }

    pub fn promote_draft_editor_files(
        &self,
        workspace_dir: &str,
        draft_name: &str,
        files: &[PlanningDraftEditorFile],
    ) -> Result<PlanningDraftPromoteResult> {
        let loaded = self.replace_and_load_draft_editor_files(workspace_dir, draft_name, files)?;
        self.promote_loaded_draft(workspace_dir, draft_name, loaded)
    }

    pub fn promote_staged_draft(
        &self,
        workspace_dir: &str,
        draft_name: &str,
    ) -> Result<PlanningDraftPromoteResult> {
        let loaded = self
            .planning_workspace_port
            .load_planning_draft_files(workspace_dir, draft_name)?;
        self.promote_loaded_draft(workspace_dir, draft_name, loaded)
    }

    fn replace_and_load_draft_editor_files(
        &self,
        workspace_dir: &str,
        draft_name: &str,
        files: &[PlanningDraftEditorFile],
    ) -> Result<PlanningDraftLoadRecord> {
        for file in files {
            self.planning_workspace_port.replace_planning_draft_file(
                workspace_dir,
                draft_name,
                &file.active_path,
                &file.body,
            )?;
        }

        self.planning_workspace_port
            .load_planning_draft_files(workspace_dir, draft_name)
    }

    fn promote_loaded_draft(
        &self,
        workspace_dir: &str,
        draft_name: &str,
        loaded: PlanningDraftLoadRecord,
    ) -> Result<PlanningDraftPromoteResult> {
        let validation_result = self.validate_loaded_draft_result(workspace_dir, &loaded)?;
        let validation_report = validation_result.report.clone();
        if !validation_report.is_valid() {
            return Ok(PlanningDraftPromoteResult {
                draft_name: draft_name.to_string(),
                promoted_file_count: 0,
                validation_report,
            });
        }
        let directions = validation_result
            .directions
            .as_ref()
            .ok_or_else(|| anyhow!("valid staged draft did not include directions"))?;
        let task_authority = validation_result
            .task_authority
            .as_ref()
            .ok_or_else(|| anyhow!("valid staged draft did not include task-authority"))?;
        let queue_projection = self
            .priority_queue_service
            .build_projection(directions, task_authority)
            .map_err(|error| anyhow!("valid staged draft queue build failed: {error}"))?;
        let mut previous_active_files = HashMap::new();
        for file in &loaded.staged_files {
            previous_active_files.insert(
                file.active_path.clone(),
                self.planning_workspace_port
                    .load_optional_planning_file(workspace_dir, &file.active_path)?,
            );
        }
        let mut applied_paths = Vec::with_capacity(loaded.staged_files.len());
        let promote_result = (|| -> Result<()> {
            for file in &loaded.staged_files {
                self.planning_workspace_port
                    .replace_planning_workspace_file(
                        workspace_dir,
                        &file.active_path,
                        Some(file.body.as_str()),
                    )?;
                applied_paths.push(file.active_path.clone());
            }
            self.commit_direction_authority_from_bootstrap(workspace_dir, directions)?;
            // Draft promotion is an operator authority rewrite: it replaces an accepted
            // planning snapshot after validation instead of applying incremental task commands.
            self.planning_task_repository_port
                .commit_task_authority_snapshot(
                    workspace_dir,
                    PlanningTaskAuthorityCommit {
                        observed_planning_revision: None,
                        task_authority,
                        queue_projection: &queue_projection,
                    },
                )?;
            Ok(())
        })();
        if let Err(error) = promote_result {
            if let Err(rollback_error) = self.restore_promoted_active_state(
                workspace_dir,
                &applied_paths,
                &previous_active_files,
            ) {
                let mut manual_recovery_paths = applied_paths.clone();
                manual_recovery_paths.sort();
                manual_recovery_paths.dedup();
                return Err(anyhow!(
                    "failed to promote staged draft `{draft_name}`: {error}; rollback failed: {rollback_error}; manual recovery may be required for: {}",
                    manual_recovery_paths.join(", ")
                ));
            }
            return Err(error);
        }

        Ok(PlanningDraftPromoteResult {
            draft_name: draft_name.to_string(),
            promoted_file_count: loaded.staged_files.len(),
            validation_report,
        })
    }

    fn restore_promoted_active_state(
        &self,
        workspace_dir: &str,
        applied_paths: &[String],
        previous_active_files: &HashMap<String, Option<String>>,
    ) -> Result<()> {
        for active_path in applied_paths.iter().rev() {
            self.planning_workspace_port
                .replace_planning_workspace_file(
                    workspace_dir,
                    active_path,
                    previous_active_files
                        .get(active_path)
                        .and_then(|body| body.as_deref()),
                )?;
        }
        Ok(())
    }

    fn stage_draft(
        &self,
        workspace_dir: &str,
        mode: PlanningBootstrapMode,
    ) -> Result<PlanningInitStageResult> {
        let bootstrap = self.prepare_bootstrap_workspace(mode);
        let draft_name = build_bootstrap_draft_name(Utc::now());
        let stage_record = self.planning_workspace_port.stage_planning_draft_files(
            workspace_dir,
            &draft_name,
            &bootstrap.files,
        )?;

        Ok(PlanningInitStageResult {
            mode,
            draft_name: stage_record.draft_name,
            draft_directory: stage_record.draft_directory,
            staged_files: stage_record.staged_files.clone(),
            staged_file_count: stage_record.staged_files.len(),
            validation_report: bootstrap.validation_report,
        })
    }

    fn stage_editor_session(
        &self,
        workspace_dir: &str,
        mode: PlanningBootstrapMode,
    ) -> Result<PlanningDraftEditorSession> {
        let staged = self.stage_draft(workspace_dir, mode)?;
        self.load_manual_editor_session(workspace_dir, &staged.draft_name)
    }

    fn validate_loaded_draft(
        &self,
        workspace_dir: &str,
        loaded: &PlanningDraftLoadRecord,
    ) -> Result<PlanningValidationReport> {
        Ok(self
            .validate_loaded_draft_result(workspace_dir, loaded)?
            .report)
    }

    fn validate_loaded_draft_result(
        &self,
        workspace_dir: &str,
        loaded: &PlanningDraftLoadRecord,
    ) -> Result<crate::domain::planning::PlanningValidationResult> {
        let directions = self
            .planning_task_repository_port
            .load_direction_authority_snapshot(workspace_dir)?
            .map(|snapshot| snapshot.directions)
            .unwrap_or_else(|| {
                self.planning_bootstrap_service
                    .build_artifacts_for_mode(PlanningBootstrapMode::Detail)
                    .directions
            });
        let staged_file_map = loaded
            .staged_files
            .iter()
            .map(|file| (file.active_path.as_str(), file.body.as_str()))
            .collect::<HashMap<_, _>>();
        let task_authority_json = default_empty_task_authority_json();
        let mut result = self.planning_validation_service.validate_workspace_files(
            crate::domain::planning::PlanningWorkspaceFiles {
                directions: &directions,
                task_authority_json: &task_authority_json,
                result_output_markdown: staged_file_map
                    .get(RESULT_OUTPUT_FILE_PATH)
                    .copied()
                    .unwrap_or_default(),
            },
        );
        if let Some(directions) = result.directions.as_ref() {
            self.planning_validation_service
                .validate_direction_supporting_files(
                    directions,
                    |path| staged_file_map.contains_key(path),
                    &mut result.report,
                );
        }

        Ok(result)
    }

    fn initialize_workspace(
        &self,
        workspace_dir: &str,
        mode: PlanningBootstrapMode,
    ) -> Result<PlanningWorkspaceInitResult> {
        if self.has_planning_workspace(workspace_dir)? {
            anyhow::bail!(
                "planning workspace already exists; reset or reuse the existing workspace instead"
            );
        }

        let bootstrap = self.prepare_bootstrap_workspace(mode);
        if !bootstrap.validation_report.is_valid() {
            let first_error = bootstrap
                .validation_report
                .errors()
                .first()
                .map(|issue| issue.message.clone())
                .unwrap_or_else(|| "planning bootstrap validation failed".to_string());
            anyhow::bail!("planning bootstrap validation failed: {first_error}");
        }

        for file in &bootstrap.files {
            self.planning_workspace_port
                .replace_planning_workspace_file(
                    workspace_dir,
                    &file.active_path,
                    Some(&file.body),
                )?;
        }
        self.commit_direction_authority_from_bootstrap(workspace_dir, &bootstrap.directions)?;
        self.commit_task_authority_from_bootstrap(
            workspace_dir,
            &bootstrap.directions,
            &bootstrap.task_authority,
        )?;

        Ok(PlanningWorkspaceInitResult {
            mode,
            created_file_count: bootstrap.files.len(),
            created_paths: bootstrap
                .files
                .iter()
                .map(|file| file.active_path.clone())
                .collect(),
        })
    }

    fn prepare_bootstrap_workspace(&self, mode: PlanningBootstrapMode) -> BootstrapWorkspacePlan {
        let artifacts = self
            .planning_bootstrap_service
            .build_artifacts_for_mode(mode);
        let task_authority_json = serde_json::to_string(&artifacts.task_authority)
            .expect("bootstrap task authority should serialize");
        let mut validation_result = self.planning_validation_service.validate_workspace_files(
            crate::domain::planning::PlanningWorkspaceFiles {
                directions: &artifacts.directions,
                task_authority_json: &task_authority_json,
                result_output_markdown: &artifacts.result_output_markdown,
            },
        );
        if let Some(directions) = validation_result.directions.as_ref() {
            let staged_supporting_paths = artifacts
                .supplemental_files
                .iter()
                .map(|file| file.active_path.as_str())
                .collect::<Vec<_>>();
            self.planning_validation_service
                .validate_direction_supporting_files(
                    directions,
                    |path| staged_supporting_paths.contains(&path),
                    &mut validation_result.report,
                );
        }

        let mut files = vec![PlanningDraftFileRecord {
            active_path: artifacts.result_output_path,
            body: artifacts.result_output_markdown,
        }];
        files.extend(artifacts.supplemental_files.into_iter().map(Into::into));

        BootstrapWorkspacePlan {
            files,
            directions: artifacts.directions,
            task_authority: artifacts.task_authority,
            validation_report: validation_result.report,
        }
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

    fn commit_task_authority_from_bootstrap(
        &self,
        workspace_dir: &str,
        directions: &DirectionCatalogDocument,
        task_authority: &TaskAuthorityDocument,
    ) -> Result<()> {
        let queue_projection = self
            .priority_queue_service
            .build_projection(directions, task_authority)
            .map_err(|error| anyhow!("valid bootstrap queue build failed: {error}"))?;
        // Bootstrap seeds a complete system-owned authority snapshot. It intentionally
        // bypasses task-level mutation commands, which only handle incremental changes.
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
}

struct BootstrapWorkspacePlan {
    files: Vec<PlanningDraftFileRecord>,
    directions: DirectionCatalogDocument,
    task_authority: TaskAuthorityDocument,
    validation_report: PlanningValidationReport,
}

fn is_operator_editable_draft_path(active_path: &str) -> bool {
    matches!(active_path, RESULT_OUTPUT_FILE_PATH)
}

fn default_empty_task_authority_json() -> String {
    serde_json::to_string(&TaskAuthorityDocument {
        version: PLANNING_FORMAT_VERSION,
        tasks: Vec::new(),
    })
    .expect("empty task authority should serialize")
}

fn build_bootstrap_draft_name(now: chrono::DateTime<Utc>) -> String {
    format!(
        "bootstrap-{}Z-{:09}",
        now.format("%Y%m%dT%H%M%S"),
        now.timestamp_subsec_nanos()
    )
}

#[cfg(test)]
mod tests {
    use super::is_operator_editable_draft_path;
    use crate::application::service::planning::RESULT_OUTPUT_FILE_PATH;

    #[test]
    fn operator_editable_draft_paths_exclude_task_authority_artifacts() {
        assert!(is_operator_editable_draft_path(RESULT_OUTPUT_FILE_PATH));
        assert!(!is_operator_editable_draft_path(
            ".codex-exec-loop/planning/direction-authority"
        ));
        assert!(!is_operator_editable_draft_path("DB task authority"));
        assert!(!is_operator_editable_draft_path(
            ".codex-exec-loop/planning/legacy-queue-snapshot.json"
        ));
    }
}
