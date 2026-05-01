use anyhow::{Result, anyhow};

use super::supporting_files::{
    build_default_detail_doc_markdown, default_validated_direction_detail_doc_path,
};
use super::{PlanningDirectionsService, PlanningDoctorOutcome, trimmed_non_empty};
use crate::application::service::planning::shared::auto_follow_copy::DEFAULT_QUEUE_IDLE_REVIEW_PROMPT_MARKDOWN;
use crate::application::service::planning::shared::contract::{
    DEFAULT_QUEUE_IDLE_PROMPT_FILE_PATH, PLANNING_DIRECTION_DOCS_DIRECTORY,
    PLANNING_PROMPTS_DIRECTORY,
};
use crate::application::service::planning::shared::planning_paths::is_valid_planning_markdown_path;
use crate::domain::planning::{PlanningValidationReport, PlanningWorkspaceFiles, QueueIdlePolicy};

impl PlanningDirectionsService {
    #[allow(dead_code)]
    pub fn doctor_workspace(&self, workspace_dir: &str) -> Result<PlanningDoctorOutcome> {
        let workspace = self.load_complete_workspace(workspace_dir)?;
        let mut directions = workspace.directions.clone();
        let mut repaired_detail_doc_mappings = 0;
        let mut created_detail_doc_files = 0;
        let mut repaired_queue_idle_prompt_mapping = false;
        let mut created_queue_idle_prompt_file = false;
        let mut pending_supporting_files = std::collections::HashMap::<String, String>::new();

        for direction in directions.directions.clone() {
            let configured_path = trimmed_non_empty(direction.detail_doc_path.as_str());
            let target_path = if configured_path.is_some_and(|path| {
                is_valid_planning_markdown_path(path, PLANNING_DIRECTION_DOCS_DIRECTORY)
            }) {
                configured_path.expect("checked above").to_string()
            } else {
                default_validated_direction_detail_doc_path(&direction.id)?
            };

            if configured_path != Some(target_path.as_str()) {
                super::set_direction_detail_doc_path(&mut directions, &direction.id, &target_path)?;
                repaired_detail_doc_mappings += 1;
            }

            if self
                .load_supporting_file_best_effort(workspace_dir, &target_path)
                .is_none()
                && pending_supporting_files
                    .insert(
                        target_path.clone(),
                        build_default_detail_doc_markdown(&direction),
                    )
                    .is_none()
            {
                created_detail_doc_files += 1;
            }
        }

        let configured_prompt_path = trimmed_non_empty(directions.queue_idle.prompt_path.as_str());
        let should_repair_queue_idle_prompt = directions.queue_idle.policy
            == QueueIdlePolicy::ReviewAndEnqueue
            || configured_prompt_path.is_some();
        if should_repair_queue_idle_prompt {
            let target_prompt_path = if configured_prompt_path.is_some_and(|path| {
                is_valid_planning_markdown_path(path, PLANNING_PROMPTS_DIRECTORY)
            }) {
                configured_prompt_path.expect("checked above").to_string()
            } else {
                DEFAULT_QUEUE_IDLE_PROMPT_FILE_PATH.to_string()
            };

            if configured_prompt_path != Some(target_prompt_path.as_str()) {
                super::set_queue_idle_prompt_path(&mut directions, &target_prompt_path);
                repaired_queue_idle_prompt_mapping = true;
            }

            if self
                .load_supporting_file_best_effort(workspace_dir, &target_prompt_path)
                .is_none()
                && pending_supporting_files
                    .insert(
                        target_prompt_path,
                        DEFAULT_QUEUE_IDLE_REVIEW_PROMPT_MARKDOWN.to_string(),
                    )
                    .is_none()
            {
                created_queue_idle_prompt_file = true;
            }
        }

        if directions != workspace.directions {
            self.commit_direction_catalog(workspace_dir, &directions)?;
        }
        for (relative_path, body) in pending_supporting_files {
            self.planning_workspace_port
                .replace_planning_workspace_file(workspace_dir, &relative_path, Some(&body))?;
        }

        let validation_report = self.validate_active_workspace(workspace_dir)?;

        Ok(PlanningDoctorOutcome {
            repaired_detail_doc_mappings,
            created_detail_doc_files,
            repaired_queue_idle_prompt_mapping,
            created_queue_idle_prompt_file,
            validation_report,
        })
    }

    #[allow(dead_code)]
    fn validate_active_workspace(&self, workspace_dir: &str) -> Result<PlanningValidationReport> {
        self.authority_seed_service
            .ensure_default_authority(workspace_dir)?;
        let workspace = self
            .planning_workspace_port
            .load_planning_workspace_files(workspace_dir)?;
        let directions = self.load_direction_catalog(workspace_dir)?;
        let mut result =
            self.planning_validation_service
                .validate_workspace_files(PlanningWorkspaceFiles {
                    directions: &directions,
                    task_authority_json: "{\"version\":1,\"tasks\":[]}",
                    result_output_markdown: workspace
                        .result_output_markdown
                        .as_deref()
                        .ok_or_else(|| {
                            anyhow!("default planning authority seed did not provide result output")
                        })?,
                });
        if let Some(directions) = result.directions.as_ref() {
            self.planning_validation_service
                .validate_direction_supporting_files(
                    directions,
                    |path| {
                        self.load_supporting_file_best_effort(workspace_dir, path)
                            .is_some()
                    },
                    &mut result.report,
                );
        }

        Ok(result.report)
    }
}
