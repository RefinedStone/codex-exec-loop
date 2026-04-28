use std::sync::Arc;

use anyhow::{Context, Result};

use crate::application::port::outbound::planning_task_repository_port::{
    PlanningTaskAuthorityCommit, PlanningTaskAuthorityCommitResult, PlanningTaskRepositoryPort,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) struct PlanningAuthoritySeedOutcome {
    pub(crate) workspace_files_seeded: bool,
    pub(crate) task_authority_seeded: bool,
}

#[derive(Clone)]
pub(crate) struct PlanningAuthoritySeedService {
    planning_workspace_port: Arc<dyn PlanningWorkspacePort>,
    planning_task_repository_port: Arc<dyn PlanningTaskRepositoryPort>,
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
        let bootstrap = self
            .planning_bootstrap_service
            .build_artifacts_for_mode(PlanningBootstrapMode::Simple);
        let mut workspace = self
            .planning_workspace_port
            .load_planning_workspace_files(workspace_dir)?;
        let workspace_files_seeded = self.ensure_workspace_files(workspace_dir, &mut workspace)?;
        let task_authority_seeded =
            self.ensure_task_authority(workspace_dir, &workspace, &bootstrap.task_authority)?;

        Ok(PlanningAuthoritySeedOutcome {
            workspace_files_seeded,
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
        if workspace.directions_toml.is_none() {
            workspace.directions_toml = Some(bootstrap.directions_toml);
            changed = true;
        }
        if workspace.result_output_markdown.is_none() {
            workspace.result_output_markdown = Some(bootstrap.result_output_markdown);
            changed = true;
        }
        for supplemental_file in bootstrap.supplemental_files {
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
            self.planning_workspace_port
                .commit_planning_workspace_files(workspace_dir, workspace)?;
        }
        Ok(changed)
    }

    fn ensure_task_authority(
        &self,
        workspace_dir: &str,
        workspace: &PlanningWorkspaceLoadRecord,
        default_task_authority: &TaskAuthorityDocument,
    ) -> Result<bool> {
        if self
            .planning_task_repository_port
            .load_task_authority_snapshot(workspace_dir)?
            .is_some()
        {
            return Ok(false);
        }

        let directions_toml = workspace
            .directions_toml
            .as_deref()
            .context("default planning authority seed did not provide directions")?;
        let result_output_markdown = workspace
            .result_output_markdown
            .as_deref()
            .context("default planning authority seed did not provide result output")?;
        let task_authority_json = serde_json::to_string(default_task_authority)
            .context("failed to serialize default task authority")?;
        let validation_result =
            self.planning_validation_service
                .validate_workspace_files(PlanningWorkspaceFiles {
                    directions_toml,
                    task_authority_json: &task_authority_json,
                    result_output_markdown,
                });
        if !validation_result.is_valid() {
            let first_failure = validation_result
                .report
                .errors()
                .first()
                .map(|issue| issue.message.as_str())
                .unwrap_or("planning validation failed");
            anyhow::bail!("default planning authority seed failed validation: {first_failure}");
        }

        let directions = toml::from_str::<DirectionCatalogDocument>(directions_toml)
            .context("failed to parse planning directions during default authority seed")?;
        let queue_projection = self
            .priority_queue_service
            .build_projection(&directions, default_task_authority)
            .context("failed to build default planning queue projection")?;
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
    use std::sync::Arc;
    use std::time::{SystemTime, UNIX_EPOCH};

    use crate::adapter::outbound::filesystem::planning_workspace::FilesystemPlanningWorkspaceAdapter;
    use crate::application::port::outbound::planning_task_repository_port::NoopPlanningTaskRepositoryPort;
    use crate::application::port::outbound::planning_workspace_port::PlanningWorkspacePort;
    use crate::application::service::planning::PlanningValidationService;
    use crate::application::service::planning::shared::contract::{
        DEFAULT_QUEUE_IDLE_PROMPT_FILE_PATH, DIRECTIONS_FILE_PATH, RESULT_OUTPUT_FILE_PATH,
    };
    use crate::domain::planning::{DirectionCatalogDocument, PriorityQueueService};

    use super::PlanningAuthoritySeedService;

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
        let workspace_dir = unique_workspace_dir("empty");
        let workspace_port = Arc::new(FilesystemPlanningWorkspaceAdapter::new());
        let service = seed_service(workspace_port.clone());

        let outcome = service
            .ensure_default_authority(&workspace_dir)
            .expect("default seed should succeed");

        assert!(outcome.workspace_files_seeded);
        assert!(outcome.task_authority_seeded);
        let workspace = workspace_port
            .load_planning_workspace_files(&workspace_dir)
            .expect("seeded workspace should load");
        assert!(workspace.directions_toml.is_some());
        assert!(workspace.result_output_markdown.is_some());
        let directions: DirectionCatalogDocument =
            toml::from_str(workspace.directions_toml.as_deref().unwrap())
                .expect("seeded directions should parse");
        assert_eq!(directions.directions[0].id, "general-workstream");
        assert!(
            workspace_port
                .load_optional_planning_file(&workspace_dir, DEFAULT_QUEUE_IDLE_PROMPT_FILE_PATH)
                .expect("seeded prompt should load")
                .is_some()
        );
        assert!(
            workspace_port
                .load_optional_planning_file(&workspace_dir, DIRECTIONS_FILE_PATH)
                .expect("seeded directions should load")
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
        assert!(outcome.task_authority_seeded);
        let workspace = workspace_port
            .load_planning_workspace_files(&workspace_dir)
            .expect("seeded workspace should load");
        assert_eq!(
            workspace.result_output_markdown.as_deref(),
            Some("# Custom Result\n\n- Preserve this operator-authored instruction.")
        );
        assert!(workspace.directions_toml.is_some());
    }
}
