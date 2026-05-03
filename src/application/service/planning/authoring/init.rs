use crate::application::port::outbound::planning_task_repository_port::{
    PlanningDirectionAuthorityCommit, PlanningTaskAuthorityCommit, PlanningTaskRepositoryPort,
};
use crate::application::port::outbound::planning_workspace_port::{
    PlanningDraftFileRecord, PlanningDraftLoadRecord, PlanningStagedFileRecord,
    PlanningWorkspacePort,
};
use crate::application::service::planning::authoring::bootstrap::{
    PlanningBootstrapMode, PlanningBootstrapService,
};
use crate::application::service::planning::runtime::validation::PlanningValidationService;
use crate::application::service::planning::shared::contract::RESULT_OUTPUT_FILE_PATH;
use crate::domain::planning::PriorityQueueService;
use crate::domain::planning::{
    DirectionCatalogDocument, PLANNING_FORMAT_VERSION, PlanningValidationReport,
    TaskAuthorityDocument,
};
use anyhow::{Result, anyhow};
use chrono::Utc;
use std::collections::HashMap;
use std::sync::Arc;

/*
 * PlanningInitService owns the transition from bootstrap artifacts to an
 * operator-visible draft or an active planning workspace. It is deliberately
 * the place where workspace markdown files, DB direction authority, DB task
 * authority, and queue projection are written together after validation.
 */
#[derive(Clone)]
pub struct PlanningInitService {
    // Workspace files store editable markdown, while repository authority stores
    // accepted JSON state. The service coordinates both ports so init/promotion
    // do not leave one side updated without the other.
    planning_workspace_port: Arc<dyn PlanningWorkspacePort>,
    planning_bootstrap_service: PlanningBootstrapService,
    planning_validation_service: PlanningValidationService,
    planning_task_repository_port: Arc<dyn PlanningTaskRepositoryPort>,
    priority_queue_service: PriorityQueueService,
}

#[derive(Debug, Clone)]
pub struct PlanningInitStageResult {
    // Staging returns both location and validation state because the operator
    // can inspect/fix a bootstrap draft before any active files are overwritten.
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
        // This compact status is used by command/TUI feedback, so it avoids
        // embedding the full validation report in a one-line notification.
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
    // active_path is the eventual workspace target; staged_path is the draft
    // copy. Keeping both visible prevents the editor from confusing isolation
    // with the final active file.
    pub active_path: String,
    pub staged_path: String,
    pub body: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanningDraftEditorSession {
    // The manual editor sees only operator-editable files, but validation is
    // computed against the full staged draft directory.
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
    // promoted_file_count is zero when validation fails; callers can show that
    // the draft was checked but no active workspace state was changed.
    pub draft_name: String,
    pub promoted_file_count: usize,
    pub validation_report: PlanningValidationReport,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanningWorkspaceInitResult {
    // Direct init writes bootstrap files immediately, unlike staging. The paths
    // list gives operators an exact record of what was created.
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
        // Test construction uses a noop authority repository so unit tests can
        // focus on workspace draft behavior without a DB-backed planning store.
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
        // Production composition injects all boundaries explicitly. That keeps
        // bootstrap, validation, repository commits, and queue projection
        // replaceable in adapter tests.
        Self {
            planning_workspace_port,
            planning_bootstrap_service,
            planning_validation_service,
            planning_task_repository_port,
            priority_queue_service,
        }
    }

    pub fn stage_simple_mode_draft(&self, workspace_dir: &str) -> Result<PlanningInitStageResult> {
        // Simple mode stages a queue-idle-ready bootstrap without touching
        // active planning files or accepted DB authority.
        self.stage_draft(workspace_dir, PlanningBootstrapMode::Simple)
    }

    pub fn stage_manual_editor_session(
        &self,
        workspace_dir: &str,
    ) -> Result<PlanningDraftEditorSession> {
        // The manual editor starts from Detail bootstrap so the operator can
        // replace placeholder direction taxonomy before promotion.
        self.stage_editor_session(workspace_dir, PlanningBootstrapMode::Detail)
    }

    pub fn load_manual_editor_session(
        &self,
        workspace_dir: &str,
        draft_name: &str,
    ) -> Result<PlanningDraftEditorSession> {
        // Loading a draft recomputes validation from staged files rather than
        // trusting stale validation from the original stage result.
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
                // Manual init only exposes result-output today; authority JSON
                // is committed through validated bootstrap structs, not edited
                // as arbitrary draft text.
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
        // Active workspace detection is file-based because older workspaces may
        // predate DB authority snapshots.
        Ok(self
            .planning_workspace_port
            .load_planning_workspace_files(workspace_dir)?
            .has_any_files())
    }

    pub fn has_planning_candidate_workspace(&self, workspace_dir: &str) -> Result<bool> {
        // Candidate detection checks staged/generated planning files used by
        // the init overlay before a full active workspace exists.
        Ok(self
            .planning_workspace_port
            .load_planning_workspace_candidate_files(workspace_dir)?
            .has_any_files())
    }
    pub fn initialize_simple_workspace(
        &self,
        workspace_dir: &str,
    ) -> Result<PlanningWorkspaceInitResult> {
        // Direct simple init is the non-editor path: validate bootstrap, write
        // files, then seed accepted authority and queue projection.
        self.initialize_workspace(workspace_dir, PlanningBootstrapMode::Simple)
    }

    pub fn save_draft_editor_files(
        &self,
        workspace_dir: &str,
        draft_name: &str,
        files: &[PlanningDraftEditorFile],
    ) -> Result<PlanningDraftSaveResult> {
        // Save replaces only files posted by the editor, reloads the draft, and
        // reports validation without promoting anything.
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
        // Promotion from the editor first persists the latest edited bodies into
        // the draft directory, then runs the same promotion path as staged drafts.
        let loaded = self.replace_and_load_draft_editor_files(workspace_dir, draft_name, files)?;
        self.promote_loaded_draft(workspace_dir, draft_name, loaded)
    }
    pub fn promote_staged_draft(
        &self,
        workspace_dir: &str,
        draft_name: &str,
    ) -> Result<PlanningDraftPromoteResult> {
        // Non-editor promotion is used by admin flows that have already staged
        // a complete draft and only need the validated active-state transition.
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
        // Editor save operates on staged copies only. The active path tells the
        // workspace adapter which draft file to replace without touching the
        // corresponding active workspace file.
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
        // Promotion is intentionally validation-gated. Invalid drafts return a
        // normal result with zero promoted files so the UI can show validation
        // details without treating the attempt as an infrastructure failure.
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
        // Keep a pre-promotion snapshot for every active file that will be
        // replaced. If a later workspace or authority write fails, these bodies
        // are the rollback source of truth.
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
            // Workspace files are written before DB authority so the committed
            // authority never points to missing active markdown in the success
            // path. Rollback handles partial workspace writes below.
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
            // Draft promotion is an operator authority rewrite: it replaces an
            // accepted planning snapshot after validation instead of applying
            // incremental task commands.
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
            // Only workspace file writes are rolled back here; if DB authority
            // write failed after workspace replacement, restoring active files
            // returns the file layer to the last known state and surfaces the
            // original authority error.
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
        // Roll back in reverse write order so repeated paths behave like a stack
        // of replacements, even though normal drafts should contain unique
        // active paths.
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
        // Staging materializes bootstrap files into an isolated draft directory.
        // It is the reversible path used before an operator commits to making
        // those files active.
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
        // Editor sessions are a thin composition: stage the bootstrap draft,
        // then reload it through the common draft-view projection.
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
        // Draft validation uses accepted direction authority when available,
        // with Detail bootstrap as a fallback for first-time manual drafts.
        let directions = self
            .planning_task_repository_port
            .load_direction_authority_snapshot(workspace_dir)?
            .map(|snapshot| snapshot.directions)
            .unwrap_or_else(|| {
                self.planning_bootstrap_service
                    .build_artifacts_for_mode(PlanningBootstrapMode::Detail)
                    .directions
            });
        // The staged map is the only source for editable/supporting file bodies.
        // Active workspace files are intentionally ignored so a draft must be
        // internally complete before promotion.
        let staged_file_map = loaded
            .staged_files
            .iter()
            .map(|file| (file.active_path.as_str(), file.body.as_str()))
            .collect::<HashMap<_, _>>();
        let task_authority_json = default_empty_task_authority_json();
        // Manual bootstrap drafts do not expose task authority editing; use an
        // empty valid authority document to validate direction/result-output and
        // supporting-file references.
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
                    // Supporting docs are considered present only when staged
                    // with the draft, which catches incomplete bootstrap plans.
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
        // Direct init refuses to run over an existing active workspace. Reset or
        // draft promotion should handle intentional replacement.
        if self.has_planning_workspace(workspace_dir)? {
            anyhow::bail!(
                "planning workspace already exists; reset or reuse the existing workspace instead"
            );
        }
        let bootstrap = self.prepare_bootstrap_workspace(mode);
        if !bootstrap.validation_report.is_valid() {
            // Fail fast before writing any file or authority state. Bootstrap
            // validation errors are operator-actionable configuration problems.
            let first_error = bootstrap
                .validation_report
                .errors()
                .first()
                .map(|issue| issue.message.clone())
                .unwrap_or_else(|| "planning bootstrap validation failed".to_string());
            anyhow::bail!("planning bootstrap validation failed: {first_error}");
        }
        // File writes come before authority commits so accepted authority never
        // references missing bootstrap markdown when initialization succeeds.
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
        // Convert the pure bootstrap artifact set into the concrete workspace
        // plan that both staging and direct init consume.
        let artifacts = self
            .planning_bootstrap_service
            .build_artifacts_for_mode(mode);
        let task_authority_json = serde_json::to_string(&artifacts.task_authority)
            .expect("bootstrap task authority should serialize");
        // Validate the exact documents that would be committed to accepted
        // authority, before adding any draft-specific path metadata.
        let mut validation_result = self.planning_validation_service.validate_workspace_files(
            crate::domain::planning::PlanningWorkspaceFiles {
                directions: &artifacts.directions,
                task_authority_json: &task_authority_json,
                result_output_markdown: &artifacts.result_output_markdown,
            },
        );
        if let Some(directions) = validation_result.directions.as_ref() {
            // Supplemental files are the only supporting files available during
            // bootstrap validation; this catches direction catalogs that point
            // to detail docs or prompt files not included in the seed plan.
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
        // Only workspace-backed files enter the draft/active file list. The DB
        // authority documents stay in structured fields below.
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
        // Bootstrap and draft promotion both replace accepted direction
        // authority after validation, so they do not use optimistic revision
        // checks from an editor session.
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
        // Queue projection is derived at the same boundary as task authority so
        // accepted task state and scheduler-facing projection remain consistent.
        let queue_projection = self
            .priority_queue_service
            .build_projection(directions, task_authority)
            .map_err(|error| anyhow!("valid bootstrap queue build failed: {error}"))?;
        // Bootstrap seeds a complete system-owned authority snapshot. It
        // intentionally bypasses task-level mutation commands, which only handle
        // incremental changes.
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
    // Internal plan keeps workspace files and DB authority documents together
    // after validation but before either staging or direct initialization.
    files: Vec<PlanningDraftFileRecord>,
    directions: DirectionCatalogDocument,
    task_authority: TaskAuthorityDocument,
    validation_report: PlanningValidationReport,
}

fn is_operator_editable_draft_path(active_path: &str) -> bool {
    // The manual init editor is intentionally narrow. Authority JSON is derived
    // from bootstrap structs and validation, not edited as free-form text.
    matches!(active_path, RESULT_OUTPUT_FILE_PATH)
}

fn default_empty_task_authority_json() -> String {
    // Validation needs a task-authority document even when this surface is only
    // editing directions/result-output. Empty versioned authority is the neutral
    // document for that check.
    serde_json::to_string(&TaskAuthorityDocument {
        version: PLANNING_FORMAT_VERSION,
        tasks: Vec::new(),
    })
    .expect("empty task authority should serialize")
}

fn build_bootstrap_draft_name(now: chrono::DateTime<Utc>) -> String {
    // Timestamp plus nanoseconds keeps concurrently staged bootstrap drafts
    // distinct while still making their creation time visible to operators.
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
        // Guard the UI boundary: only result-output belongs in the manual
        // bootstrap editor until structured authority editing exists here.
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
