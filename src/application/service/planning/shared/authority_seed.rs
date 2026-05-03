use crate::application::port::outbound::planning_task_repository_port::{
    PlanningDirectionAuthorityCommit, PlanningTaskAuthorityCommit,
    PlanningTaskAuthorityCommitResult, PlanningTaskRepositoryPort,
};
use crate::application::port::outbound::planning_workspace_port::{
    PlanningWorkspaceLoadRecord, PlanningWorkspacePort,
};
use crate::application::service::planning::authoring::bootstrap::{
    PlanningBootstrapMode, PlanningBootstrapService,
};
use crate::application::service::planning::runtime::validation::PlanningValidationService;
use crate::domain::planning::{
    DirectionCatalogDocument, PlanningWorkspaceFiles, PriorityQueueService, TaskAuthorityDocument,
};
use anyhow::{Context, Result};
use std::sync::Arc;

/*
 * PlanningAuthoritySeedService is the bridge between file-backed planning
 * workspace artifacts and repository-backed authority snapshots.  It exists for
 * startup/repair paths that need a usable Simple-mode workspace without
 * overwriting operator-authored files or already committed authority state.
 */
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) struct PlanningAuthoritySeedOutcome {
    // The three flags are intentionally separate because callers may repair only
    // one missing layer: markdown files, direction authority, or task authority.
    pub(crate) workspace_files_seeded: bool,
    pub(crate) direction_authority_seeded: bool,
    pub(crate) task_authority_seeded: bool,
}
#[derive(Clone)]
pub(crate) struct PlanningAuthoritySeedService {
    planning_workspace_port: Arc<dyn PlanningWorkspacePort>,
    planning_task_repository_port: Arc<dyn PlanningTaskRepositoryPort>,
    // Bootstrap supplies canonical Simple-mode artifacts; validation and queue
    // services prove those artifacts are safe before task authority is committed.
    planning_bootstrap_service: PlanningBootstrapService,
    planning_validation_service: PlanningValidationService,
    priority_queue_service: PriorityQueueService,
}
impl PlanningAuthoritySeedService {
    pub(crate) fn new(
        planning_workspace_port: Arc<dyn PlanningWorkspacePort>,
        planning_task_repository_port: Arc<dyn PlanningTaskRepositoryPort>,
        planning_validation_service: PlanningValidationService,
        priority_queue_service: PriorityQueueService,
    ) -> Self {
        Self {
            planning_workspace_port,
            planning_task_repository_port,
            planning_bootstrap_service: PlanningBootstrapService::new(),
            planning_validation_service,
            priority_queue_service,
        }
    }
    pub(crate) fn ensure_default_authority(
        &self,
        workspace_dir: &str,
    ) -> Result<PlanningAuthoritySeedOutcome> {
        /*
         * Seeding is ordered from files to authorities.  Task authority
         * validation needs result output markdown, and queue projection needs a
         * direction catalog, so later layers depend on earlier ones being
         * present even when they were not changed in this call.
         */
        let bootstrap = self
            .planning_bootstrap_service
            .build_artifacts_for_mode(PlanningBootstrapMode::Simple);
        let mut workspace = self
            .planning_workspace_port
            .load_planning_workspace_files(workspace_dir)?;
        let workspace_files_seeded = self.ensure_workspace_files(workspace_dir, &mut workspace)?;
        let direction_authority_seeded =
            self.ensure_direction_authority(workspace_dir, &bootstrap.directions)?;
        let task_authority_seeded = self.ensure_task_authority(
            workspace_dir,
            &workspace,
            &bootstrap.directions,
            &bootstrap.task_authority,
        )?;
        Ok(PlanningAuthoritySeedOutcome {
            workspace_files_seeded,
            direction_authority_seeded,
            task_authority_seeded,
        })
    }
    fn ensure_workspace_files(
        &self,
        workspace_dir: &str,
        workspace: &mut PlanningWorkspaceLoadRecord,
    ) -> Result<bool> {
        let bootstrap = self
            .planning_bootstrap_service
            .build_artifacts_for_mode(PlanningBootstrapMode::Simple);
        let mut changed = false;
        // Result output is stored inside the aggregate workspace record; missing
        // supplemental files are written individually so existing operator files
        // keep their exact content.
        if workspace.result_output_markdown.is_none() {
            workspace.result_output_markdown = Some(bootstrap.result_output_markdown);
            changed = true;
        }
        for supplemental_file in bootstrap.supplemental_files {
            // Supplemental artifacts are default prompts/contracts.  Presence is
            // enough; this service does not attempt to reconcile local edits.
            if self
                .planning_workspace_port
                .load_optional_planning_file(workspace_dir, &supplemental_file.active_path)?
                .is_none()
            {
                self.planning_workspace_port
                    .replace_planning_workspace_file(
                        workspace_dir,
                        &supplemental_file.active_path,
                        Some(&supplemental_file.body),
                    )?;
                changed = true;
            }
        }
        if changed {
            // Commit the aggregate record only when a field inside it changed.
            // Supplemental file writes have already been persisted through the
            // workspace port.
            self.planning_workspace_port
                .commit_planning_workspace_files(workspace_dir, workspace)?;
        }
        Ok(changed)
    }
    fn ensure_direction_authority(
        &self,
        workspace_dir: &str,
        default_directions: &DirectionCatalogDocument,
    ) -> Result<bool> {
        // Existing authority snapshots win.  This protects repositories that
        // have already moved beyond the Simple bootstrap direction set.
        if self
            .planning_task_repository_port
            .load_direction_authority_snapshot(workspace_dir)?
            .is_some()
        {
            return Ok(false);
        }
        match self
            .planning_task_repository_port
            .commit_direction_authority_snapshot(
                workspace_dir,
                PlanningDirectionAuthorityCommit {
                    observed_planning_revision: None,
                    directions: default_directions,
                },
            )? {
            // A conflict means another seeder or admin flow committed first; the
            // caller can treat that as "nothing to seed" for idempotent startup.
            PlanningTaskAuthorityCommitResult::Committed { .. } => Ok(true),
            PlanningTaskAuthorityCommitResult::Conflict { .. } => Ok(false),
        }
    }
    fn ensure_task_authority(
        &self,
        workspace_dir: &str,
        workspace: &PlanningWorkspaceLoadRecord,
        default_directions: &DirectionCatalogDocument,
        default_task_authority: &TaskAuthorityDocument,
    ) -> Result<bool> {
        // Task authority is the last layer because it references directions and
        // is paired with a queue projection in the same repository commit.
        if self
            .planning_task_repository_port
            .load_task_authority_snapshot(workspace_dir)?
            .is_some()
        {
            return Ok(false);
        }
        let result_output_markdown = workspace
            .result_output_markdown
            .as_deref()
            .context("default planning authority seed did not provide result output")?;
        let task_authority_json = serde_json::to_string(default_task_authority)
            .context("failed to serialize default task authority")?;
        let validation_result =
            self.planning_validation_service
                .validate_workspace_files(PlanningWorkspaceFiles {
                    directions: default_directions,
                    task_authority_json: &task_authority_json,
                    result_output_markdown,
                });
        if !validation_result.is_valid() {
            // Fail before writing authority if bootstrap artifacts no longer
            // satisfy the same validation used by operator-managed workspaces.
            let first_failure = validation_result
                .report
                .errors()
                .first()
                .map(|issue| issue.message.as_str())
                .unwrap_or("planning validation failed");
            anyhow::bail!("default planning authority seed failed validation: {first_failure}");
        }
        let queue_projection = self
            .priority_queue_service
            .build_projection(default_directions, default_task_authority)
            .context("failed to build default planning queue projection")?;
        // Commit task authority and its derived queue projection atomically so
        // runtime readers never observe an authority without matching queue
        // metadata.
        match self
            .planning_task_repository_port
            .commit_task_authority_snapshot(
                workspace_dir,
                PlanningTaskAuthorityCommit {
                    observed_planning_revision: None,
                    task_authority: default_task_authority,
                    queue_projection: &queue_projection,
                },
            )? {
            PlanningTaskAuthorityCommitResult::Committed { .. } => Ok(true),
            PlanningTaskAuthorityCommitResult::Conflict { .. } => Ok(false),
        }
    }
}
#[cfg(test)]
mod tests {
    use super::PlanningAuthoritySeedService;
    use crate::adapter::outbound::filesystem::planning_workspace::FilesystemPlanningWorkspaceAdapter;
    use crate::application::port::outbound::planning_task_repository_port::{
        NoopPlanningTaskRepositoryPort, PlanningTaskRepositoryPort,
    };
    use crate::application::port::outbound::planning_workspace_port::PlanningWorkspacePort;
    use crate::application::service::planning::PlanningValidationService;
    use crate::application::service::planning::shared::contract::{
        DEFAULT_QUEUE_IDLE_PROMPT_FILE_PATH, RESULT_OUTPUT_FILE_PATH,
    };
    use crate::domain::planning::PriorityQueueService;
    use std::sync::Arc;
    use std::time::{SystemTime, UNIX_EPOCH};

    /*
     * Tests use the real filesystem workspace adapter and the noop authority
     * repository.  That combination exercises file preservation and repository
     * snapshot commits without needing a full app-server runtime.
     */
    fn unique_workspace_dir(label: &str) -> String {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock should be after epoch")
            .as_nanos();
        std::env::temp_dir()
            .join(format!("akra-planning-seed-{label}-{nanos}"))
            .display()
            .to_string()
    }
    fn seed_service(
        workspace_port: Arc<dyn PlanningWorkspacePort>,
    ) -> PlanningAuthoritySeedService {
        PlanningAuthoritySeedService::new(
            workspace_port,
            Arc::new(NoopPlanningTaskRepositoryPort),
            PlanningValidationService::new(),
            PriorityQueueService::new(),
        )
    }
    #[test]
    fn seeds_default_workspace_files_and_empty_task_authority() {
        // Empty workspaces should receive all three planning layers and become
        // immediately readable by runtime services.
        let workspace_dir = unique_workspace_dir("empty");
        let workspace_port = Arc::new(FilesystemPlanningWorkspaceAdapter::new());
        let service = seed_service(workspace_port.clone());
        let outcome = service
            .ensure_default_authority(&workspace_dir)
            .expect("default seed should succeed");

        assert!(outcome.workspace_files_seeded);
        assert!(outcome.direction_authority_seeded);
        assert!(outcome.task_authority_seeded);
        let workspace = workspace_port
            .load_planning_workspace_files(&workspace_dir)
            .expect("seeded workspace should load");
        assert!(workspace.result_output_markdown.is_some());
        let directions = NoopPlanningTaskRepositoryPort
            .load_direction_authority_snapshot(&workspace_dir)
            .expect("seeded directions should load")
            .expect("seeded directions should exist")
            .directions;
        assert_eq!(directions.directions[0].id, "general-workstream");
        assert!(
            workspace_port
                .load_optional_planning_file(&workspace_dir, DEFAULT_QUEUE_IDLE_PROMPT_FILE_PATH)
                .expect("seeded prompt should load")
                .is_some()
        );
        assert!(
            workspace_port
                .load_optional_planning_file(&workspace_dir, RESULT_OUTPUT_FILE_PATH)
                .expect("seeded result output should load")
                .is_some()
        );
    }
    #[test]
    fn preserves_existing_operator_files_while_seeding_missing_authority() {
        // Seeding may fill missing authorities in a partially initialized
        // workspace, but it must not rewrite operator-authored markdown.
        let workspace_dir = unique_workspace_dir("partial");
        let workspace_port = Arc::new(FilesystemPlanningWorkspaceAdapter::new());
        workspace_port
            .replace_planning_workspace_file(
                &workspace_dir,
                RESULT_OUTPUT_FILE_PATH,
                Some("# Custom Result\n\n- Preserve this operator-authored instruction."),
            )
            .expect("custom result should write");
        let service = seed_service(workspace_port.clone());
        let outcome = service
            .ensure_default_authority(&workspace_dir)
            .expect("default seed should succeed");

        assert!(outcome.workspace_files_seeded);
        assert!(outcome.direction_authority_seeded);
        assert!(outcome.task_authority_seeded);
        let workspace = workspace_port
            .load_planning_workspace_files(&workspace_dir)
            .expect("seeded workspace should load");
        assert_eq!(
            workspace.result_output_markdown.as_deref(),
            Some("# Custom Result\n\n- Preserve this operator-authored instruction.")
        );
        let directions = NoopPlanningTaskRepositoryPort
            .load_direction_authority_snapshot(&workspace_dir)
            .expect("seeded directions should load")
            .expect("seeded directions should exist")
            .directions;
        assert_eq!(directions.directions[0].id, "general-workstream");
    }
}
