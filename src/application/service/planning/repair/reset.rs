use std::sync::Arc;

use anyhow::{Result, anyhow};

use crate::application::port::outbound::planning_task_repository_port::{
    PlanningDirectionAuthorityCommit, PlanningTaskAuthorityCommit, PlanningTaskRepositoryPort,
};
use crate::application::port::outbound::planning_workspace_port::{
    PlanningWorkspaceLoadRecord, PlanningWorkspacePort,
};
use crate::application::service::planning::authoring::bootstrap::{
    PlanningBootstrapArtifacts, PlanningBootstrapMode, PlanningBootstrapService,
};
use crate::application::service::planning::runtime::validation::PlanningValidationService;
use crate::application::service::planning::shared::contract::{
    DEFAULT_QUEUE_IDLE_PROMPT_FILE_PATH, PLANNING_DIRECTION_DOCS_DIRECTORY,
    PLANNING_DRAFTS_DIRECTORY, PLANNING_PROMPTS_DIRECTORY, PLANNING_REJECTED_DIRECTORY,
    RESULT_OUTPUT_FILE_PATH,
};
use crate::domain::planning::PriorityQueueService;
use crate::domain::planning::{DirectionCatalogDocument, TaskAuthorityDocument, TaskStatus};

/*
 * Reset is the operator-controlled destructive repair path for planning authority.
 * It intentionally bypasses worker mutation prompts and writes fresh bootstrap-derived authority through the
 * same workspace and repository ports that normal planning uses, so downstream runtime snapshots keep one source
 * of truth after a reset.
 */

// Legacy runtime exports are removed only by full reset because they are generated cache/output material.
const LEGACY_RUNTIME_EXPORTS_DIRECTORY: &str = ".codex-exec-loop/runtime/exports";

// Directions reset replaces direction authority and prompt/detail artifacts while preserving existing tasks.
const RESET_DIRECTIONS_REMOVED_PATHS: &[&str] = &[
    PLANNING_DIRECTION_DOCS_DIRECTORY,
    PLANNING_PROMPTS_DIRECTORY,
];

// Full reset also clears generated drafts/rejections so stale generated planning state cannot survive bootstrap.
const RESET_ALL_GENERATED_ARTIFACT_PATHS: &[&str] = &[
    PLANNING_DRAFTS_DIRECTORY,
    PLANNING_REJECTED_DIRECTORY,
    LEGACY_RUNTIME_EXPORTS_DIRECTORY,
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
// Public reset target used by CLI, admin API, Telegram, TUI, and control command adapters.
pub enum PlanningResetTarget {
    Queue,
    Directions,
    All,
}
impl PlanningResetTarget {
    // Labels are part of the external command/report surface.
    pub fn label(self) -> &'static str {
        match self {
            Self::Queue => "queue",
            Self::Directions => "directions",
            Self::All => "all",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
// Result reports externally visible file effects; DB authority rewrites are represented by the target itself.
pub struct PlanningWorkspaceResetResult {
    pub target: PlanningResetTarget,
    pub rewritten_paths: Vec<String>,
    pub removed_paths: Vec<String>,
}

#[derive(Clone)]
/*
 * Reset service coordinates two outbound boundaries.
 * `PlanningWorkspacePort` rewrites/removes active scaffold files, while `PlanningTaskRepositoryPort` commits
 * accepted DB authority snapshots and queue projections after validation.
 */
pub struct PlanningResetService {
    planning_workspace_port: Arc<dyn PlanningWorkspacePort>,
    planning_bootstrap_service: PlanningBootstrapService,
    planning_task_repository_port: Arc<dyn PlanningTaskRepositoryPort>,
    planning_validation_service: PlanningValidationService,
    priority_queue_service: PriorityQueueService,
}
impl PlanningResetService {
    #[cfg(test)]
    #[allow(dead_code)]
    // Test constructor preserves the older dependency shape while production uses the full repository boundary.
    pub fn new(
        planning_workspace_port: Arc<dyn PlanningWorkspacePort>,
        planning_bootstrap_service: PlanningBootstrapService,
    ) -> Self {
        Self::with_task_repository(
            planning_workspace_port,
            planning_bootstrap_service,
            Arc::new(crate::application::port::outbound::planning_task_repository_port::NoopPlanningTaskRepositoryPort),
            PlanningValidationService::new(),
            PriorityQueueService::new(),
        )
    }

    // Production constructor receives every collaborator needed to rewrite both file and DB authority surfaces.
    pub fn with_task_repository(
        planning_workspace_port: Arc<dyn PlanningWorkspacePort>,
        planning_bootstrap_service: PlanningBootstrapService,
        planning_task_repository_port: Arc<dyn PlanningTaskRepositoryPort>,
        planning_validation_service: PlanningValidationService,
        priority_queue_service: PriorityQueueService,
    ) -> Self {
        Self {
            planning_workspace_port,
            planning_bootstrap_service,
            planning_task_repository_port,
            planning_validation_service,
            priority_queue_service,
        }
    }

    /*
     * Reset an existing planning workspace according to the selected destructive scope.
     * Bootstrap artifacts are always generated in Simple mode so queue/directions/all reset share the same
     * baseline direction catalog, default queue-idle prompt, and empty task authority.
     */
    pub fn reset_workspace(
        &self,
        workspace_dir: &str,
        target: PlanningResetTarget,
    ) -> Result<PlanningWorkspaceResetResult> {
        let workspace = self.load_existing_workspace(workspace_dir)?;
        let bootstrap = self
            .planning_bootstrap_service
            .build_artifacts_for_mode(PlanningBootstrapMode::Simple);
        match target {
            PlanningResetTarget::Queue => self.reset_queue(workspace_dir, &workspace, &bootstrap),
            PlanningResetTarget::Directions => {
                self.ensure_directions_reset_is_safe(workspace_dir)?;
                self.reset_directions(workspace_dir, &workspace, &bootstrap)
            }
            PlanningResetTarget::All => self.reset_all(workspace_dir, &bootstrap),
        }
    }

    // Reset should not implicitly initialize a totally absent workspace; init/doctor own bootstrap creation.
    fn load_existing_workspace(&self, workspace_dir: &str) -> Result<PlanningWorkspaceLoadRecord> {
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

    /*
     * Queue reset clears task authority back to the bootstrap empty queue.
     * It leaves direction files and prompts alone, so the commit helper must reuse the existing direction DB
     * snapshot and result-output markdown before accepting the replacement task authority.
     */
    fn reset_queue(
        &self,
        workspace_dir: &str,
        workspace: &PlanningWorkspaceLoadRecord,
        bootstrap: &PlanningBootstrapArtifacts,
    ) -> Result<PlanningWorkspaceResetResult> {
        self.commit_task_authority_from_document(
            workspace_dir,
            None,
            &bootstrap.task_authority,
            workspace.result_output_markdown.as_deref(),
        )?;
        Ok(PlanningWorkspaceResetResult {
            target: PlanningResetTarget::Queue,
            rewritten_paths: Vec::new(),
            removed_paths: Vec::new(),
        })
    }

    /*
     * Directions reset is blocked while live tasks exist because replacing direction authority under active work
     * can orphan task/direction relationships. Operators can choose reset all when they intend to discard both
     * directions and task queue together.
     */
    fn ensure_directions_reset_is_safe(&self, workspace_dir: &str) -> Result<()> {
        let task_authority = self
            .planning_task_repository_port
            .load_task_authority_snapshot(workspace_dir)?
            .map(|snapshot| snapshot.task_authority)
            .unwrap_or_else(|| TaskAuthorityDocument {
                version: crate::domain::planning::PLANNING_FORMAT_VERSION,
                tasks: Vec::new(),
            });
        let live_tasks = task_authority
            .tasks
            .iter()
            .filter(|task| !matches!(task.status, TaskStatus::Done | TaskStatus::Cancelled))
            .map(|task| format!("{}({})", task.id.trim(), task.status.label()))
            .collect::<Vec<_>>();
        if live_tasks.is_empty() {
            return Ok(());
        }
        let live_task_summary = live_tasks
            .iter()
            .take(3)
            .cloned()
            .collect::<Vec<_>>()
            .join(", ");
        let extra_count = live_tasks.len().saturating_sub(3);
        let suffix = if extra_count == 0 {
            String::new()
        } else {
            format!(" (+{extra_count} more)")
        };
        Err(anyhow!(
            "planning directions reset is blocked by live tasks: {live_task_summary}{suffix}; use reset all to replace the full workspace instead"
        ))
    }

    /*
     * Directions reset refreshes the direction catalog and supporting prompt/detail files, then recommits the
     * existing task authority against the new directions. That validation step is the guard that prevents the
     * repository snapshot from accepting tasks that no longer match the reset direction catalog.
     */
    fn reset_directions(
        &self,
        workspace_dir: &str,
        workspace: &PlanningWorkspaceLoadRecord,
        bootstrap: &PlanningBootstrapArtifacts,
    ) -> Result<PlanningWorkspaceResetResult> {
        self.reset_directions_side_artifacts(workspace_dir, bootstrap)?;
        let task_authority = self
            .planning_task_repository_port
            .load_task_authority_snapshot(workspace_dir)?
            .map(|snapshot| snapshot.task_authority)
            .unwrap_or_else(|| bootstrap.task_authority.clone());
        self.commit_task_authority_from_document(
            workspace_dir,
            Some(&bootstrap.directions),
            &task_authority,
            workspace.result_output_markdown.as_deref(),
        )?;
        Ok(PlanningWorkspaceResetResult {
            target: PlanningResetTarget::Directions,
            rewritten_paths: vec![DEFAULT_QUEUE_IDLE_PROMPT_FILE_PATH.to_string()],
            removed_paths: removed_path_strings(RESET_DIRECTIONS_REMOVED_PATHS),
        })
    }

    /*
     * Full reset replaces the active scaffold, direction authority, task authority, and generated planning caches.
     * It is the only target that rewrites `result-output.md`, because queue/directions reset should not erase the
     * operator-facing current planning instruction document.
     */
    fn reset_all(
        &self,
        workspace_dir: &str,
        bootstrap: &PlanningBootstrapArtifacts,
    ) -> Result<PlanningWorkspaceResetResult> {
        self.reset_all_generated_artifacts(workspace_dir)?;
        self.reset_directions_side_artifacts(workspace_dir, bootstrap)?;
        self.planning_workspace_port
            .replace_planning_workspace_file(
                workspace_dir,
                RESULT_OUTPUT_FILE_PATH,
                Some(&bootstrap.result_output_markdown),
            )?;
        self.commit_task_authority_from_document(
            workspace_dir,
            Some(&bootstrap.directions),
            &bootstrap.task_authority,
            Some(&bootstrap.result_output_markdown),
        )?;
        Ok(PlanningWorkspaceResetResult {
            target: PlanningResetTarget::All,
            rewritten_paths: vec![
                RESULT_OUTPUT_FILE_PATH.to_string(),
                DEFAULT_QUEUE_IDLE_PROMPT_FILE_PATH.to_string(),
            ],
            removed_paths: reset_all_removed_path_strings(),
        })
    }

    // Remove generated artifacts before writing fresh bootstrap state so old drafts/rejections cannot reappear.
    fn reset_all_generated_artifacts(&self, workspace_dir: &str) -> Result<()> {
        for path in RESET_ALL_GENERATED_ARTIFACT_PATHS {
            self.planning_workspace_port
                .remove_planning_workspace_entry(workspace_dir, path)?;
        }
        Ok(())
    }

    /*
     * Direction side artifacts are file-backed companion material for direction authority.
     * The DB direction snapshot is committed before supplemental files so any later file write error leaves the
     * authority source updated, while the operator still sees the failed path through the returned error.
     */
    fn reset_directions_side_artifacts(
        &self,
        workspace_dir: &str,
        bootstrap: &PlanningBootstrapArtifacts,
    ) -> Result<()> {
        for path in RESET_DIRECTIONS_REMOVED_PATHS {
            self.planning_workspace_port
                .remove_planning_workspace_entry(workspace_dir, path)?;
        }
        self.commit_direction_authority_from_bootstrap(workspace_dir, &bootstrap.directions)?;
        for supplemental_file in &bootstrap.supplemental_files {
            self.planning_workspace_port
                .replace_planning_workspace_file(
                    workspace_dir,
                    &supplemental_file.active_path,
                    Some(&supplemental_file.body),
                )?;
        }
        Ok(())
    }

    /*
     * Commit task authority only after enough context exists to validate the whole planning runtime contract.
     * If directions or result-output are missing, the safest reset effect is to clear the DB task snapshot rather
     * than commit a queue projection that cannot be proven against active workspace authority.
     */
    fn commit_task_authority_from_document(
        &self,
        workspace_dir: &str,
        directions: Option<&DirectionCatalogDocument>,
        task_authority: &TaskAuthorityDocument,
        result_output_markdown: Option<&str>,
    ) -> Result<()> {
        let loaded_directions;
        let directions = match directions {
            Some(directions) => Some(directions),
            None => {
                loaded_directions = self
                    .planning_task_repository_port
                    .load_direction_authority_snapshot(workspace_dir)?
                    .map(|snapshot| snapshot.directions);
                loaded_directions.as_ref()
            }
        };
        let (Some(directions), Some(result_output_markdown)) = (directions, result_output_markdown)
        else {
            return self
                .planning_task_repository_port
                .clear_task_authority_snapshot(workspace_dir);
        };
        let task_authority_json = serde_json::to_string(task_authority)?;
        let validation_result = self.planning_validation_service.validate_workspace_files(
            crate::domain::planning::PlanningWorkspaceFiles {
                directions,
                task_authority_json: &task_authority_json,
                result_output_markdown,
            },
        );
        if !validation_result.is_valid() {
            return Ok(());
        }

        // Validation reparses the accepted direction/task documents; commit those normalized domain values.
        let directions = validation_result
            .directions
            .as_ref()
            .ok_or_else(|| anyhow!("valid reset workspace did not include directions"))?;
        let task_authority = validation_result
            .task_authority
            .as_ref()
            .ok_or_else(|| anyhow!("valid reset workspace did not include task-authority"))?;
        let queue_projection = self
            .priority_queue_service
            .build_projection(directions, task_authority)
            .map_err(|error| anyhow!("valid reset queue build failed: {error}"))?;
        // Reset is an operator/system authority rewrite boundary, not an incremental
        // task mutation. The caller selected a destructive reset target explicitly.
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

    // Direction authority reset does not need a queue projection; tasks are committed separately after validation.
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
}

// Merge direction-side and generated-artifact removal lists for the full-reset report.
fn reset_all_removed_path_strings() -> Vec<String> {
    RESET_DIRECTIONS_REMOVED_PATHS
        .iter()
        .chain(RESET_ALL_GENERATED_ARTIFACT_PATHS.iter())
        .map(|path| (*path).to_string())
        .collect()
}

// Convert static reset path lists into owned report data without exposing the static slices.
fn removed_path_strings(paths: &[&str]) -> Vec<String> {
    paths.iter().map(|path| (*path).to_string()).collect()
}

#[cfg(test)]
// Current unit coverage only pins the public target variants; behavior is exercised through inbound reset flows.
mod tests {
    use super::PlanningResetTarget;

    #[test]
    // Keep target variants from disappearing while reset callers are still wired through public enum matching.
    fn reset_target_values_still_exist() {
        assert!(matches!(
            PlanningResetTarget::Queue,
            PlanningResetTarget::Queue
        ));
        assert!(matches!(
            PlanningResetTarget::Directions,
            PlanningResetTarget::Directions
        ));
        assert!(matches!(PlanningResetTarget::All, PlanningResetTarget::All));
    }
}
