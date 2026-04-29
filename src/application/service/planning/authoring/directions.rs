use std::collections::HashSet;
use std::path::{Component, Path};
use std::sync::Arc;

use anyhow::{Result, anyhow};
use chrono::Utc;

use crate::application::port::outbound::planning_task_repository_port::{
    PlanningDirectionAuthorityCommit, PlanningTaskAuthorityCommitResult, PlanningTaskRepositoryPort,
};
use crate::application::port::outbound::planning_workspace_port::{
    PlanningDraftFileRecord, PlanningDraftLoadRecord, PlanningWorkspacePort,
};
use crate::application::service::planning::authoring::init::{
    PlanningDraftEditorFile, PlanningDraftEditorSession,
};
use crate::application::service::planning::runtime::validation::PlanningValidationService;
use crate::application::service::planning::shared::authority_seed::PlanningAuthoritySeedService;
use crate::application::service::planning::shared::auto_follow_copy::DEFAULT_QUEUE_IDLE_REVIEW_PROMPT_MARKDOWN;
use crate::application::service::planning::shared::contract::{
    DEFAULT_QUEUE_IDLE_PROMPT_FILE_PATH, PLANNING_DIRECTION_DOCS_DIRECTORY,
    PLANNING_PROMPTS_DIRECTORY, RESULT_OUTPUT_FILE_PATH, default_direction_detail_doc_path,
};
use crate::domain::planning::PriorityQueueService;
use crate::domain::planning::{
    DirectionCatalogDocument, PlanningValidationReport, QueueIdlePolicy,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DirectionsSupportingFileStatus {
    MissingMapping,
    Ready,
    BrokenMapping,
}

impl DirectionsSupportingFileStatus {
    pub fn label(self) -> &'static str {
        match self {
            Self::MissingMapping => "unset",
            Self::Ready => "ready",
            Self::BrokenMapping => "broken",
        }
    }

    pub fn needs_attention(self) -> bool {
        self != Self::Ready
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirectionsMaintenanceDirectionSummary {
    pub id: String,
    pub title: String,
    pub detail_doc_path: Option<String>,
    pub detail_doc_status: DirectionsSupportingFileStatus,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirectionsMaintenanceSummary {
    pub directions: Vec<DirectionsMaintenanceDirectionSummary>,
    pub missing_detail_doc_count: usize,
    pub broken_detail_doc_count: usize,
    pub queue_idle_policy: QueueIdlePolicy,
    pub queue_idle_prompt_path: Option<String>,
    pub queue_idle_prompt_status: DirectionsSupportingFileStatus,
    pub parse_error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueueIdleReviewContext {
    pub policy: QueueIdlePolicy,
    pub prompt_path: Option<String>,
    pub prompt_markdown: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanningDoctorOutcome {
    pub repaired_detail_doc_mappings: usize,
    pub created_detail_doc_files: usize,
    pub repaired_queue_idle_prompt_mapping: bool,
    pub created_queue_idle_prompt_file: bool,
    pub validation_report: PlanningValidationReport,
}

impl PlanningDoctorOutcome {
    pub fn applied_fix_count(&self) -> usize {
        self.repaired_detail_doc_mappings
            + self.created_detail_doc_files
            + usize::from(self.repaired_queue_idle_prompt_mapping)
            + usize::from(self.created_queue_idle_prompt_file)
    }
}

#[derive(Clone)]
pub struct PlanningDirectionsService {
    planning_workspace_port: Arc<dyn PlanningWorkspacePort>,
    planning_task_repository_port: Arc<dyn PlanningTaskRepositoryPort>,
    planning_validation_service: PlanningValidationService,
    authority_seed_service: PlanningAuthoritySeedService,
}

impl PlanningDirectionsService {
    pub fn new(
        planning_workspace_port: Arc<dyn PlanningWorkspacePort>,
        planning_task_repository_port: Arc<dyn PlanningTaskRepositoryPort>,
        planning_validation_service: PlanningValidationService,
        priority_queue_service: PriorityQueueService,
    ) -> Self {
        Self {
            authority_seed_service: PlanningAuthoritySeedService::new(
                planning_workspace_port.clone(),
                planning_task_repository_port.clone(),
                planning_validation_service.clone(),
                priority_queue_service,
            ),
            planning_workspace_port,
            planning_task_repository_port,
            planning_validation_service,
        }
    }

    fn load_direction_catalog(&self, workspace_dir: &str) -> Result<DirectionCatalogDocument> {
        self.authority_seed_service
            .ensure_default_authority(workspace_dir)?;
        self.planning_task_repository_port
            .load_direction_authority_snapshot(workspace_dir)?
            .map(|snapshot| snapshot.directions)
            .ok_or_else(|| anyhow!("default planning authority seed did not provide directions"))
    }

    fn commit_direction_catalog(
        &self,
        workspace_dir: &str,
        directions: &DirectionCatalogDocument,
    ) -> Result<()> {
        match self
            .planning_task_repository_port
            .commit_direction_authority_snapshot(
                workspace_dir,
                PlanningDirectionAuthorityCommit {
                    observed_planning_revision: None,
                    directions,
                },
            )? {
            PlanningTaskAuthorityCommitResult::Committed { .. } => Ok(()),
            PlanningTaskAuthorityCommitResult::Conflict { .. } => Err(anyhow!(
                "planning direction authority changed while editing; retry"
            )),
        }
    }

    pub fn load_summary(&self, workspace_dir: &str) -> Result<DirectionsMaintenanceSummary> {
        let catalog = self.load_direction_catalog(workspace_dir)?;
        let queue_idle_prompt_path =
            trimmed_non_empty(catalog.queue_idle.prompt_path.as_str()).map(str::to_string);
        let queue_idle_prompt_status = self.supporting_file_status(
            workspace_dir,
            queue_idle_prompt_path.as_deref(),
            PLANNING_PROMPTS_DIRECTORY,
        );

        let directions = catalog
            .directions
            .into_iter()
            .map(|direction| {
                let detail_doc_path =
                    trimmed_non_empty(direction.detail_doc_path.as_str()).map(str::to_string);
                let detail_doc_status = self.supporting_file_status(
                    workspace_dir,
                    detail_doc_path.as_deref(),
                    PLANNING_DIRECTION_DOCS_DIRECTORY,
                );

                Ok(DirectionsMaintenanceDirectionSummary {
                    id: direction.id.trim().to_string(),
                    title: direction.title.trim().to_string(),
                    detail_doc_path,
                    detail_doc_status,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        let missing_detail_doc_count = directions
            .iter()
            .filter(|direction| {
                direction.detail_doc_status == DirectionsSupportingFileStatus::MissingMapping
            })
            .count();
        let broken_detail_doc_count = directions
            .iter()
            .filter(|direction| {
                direction.detail_doc_status == DirectionsSupportingFileStatus::BrokenMapping
            })
            .count();

        Ok(DirectionsMaintenanceSummary {
            directions,
            missing_detail_doc_count,
            broken_detail_doc_count,
            queue_idle_policy: catalog.queue_idle.policy,
            queue_idle_prompt_path,
            queue_idle_prompt_status,
            parse_error: None,
        })
    }

    pub fn load_queue_idle_review_context(
        &self,
        workspace_dir: &str,
    ) -> Result<QueueIdleReviewContext> {
        let directions = self.load_direction_catalog(workspace_dir)?;
        let prompt_path =
            trimmed_non_empty(directions.queue_idle.prompt_path.as_str()).map(str::to_string);
        let prompt_markdown = prompt_path
            .as_deref()
            .and_then(|path| self.load_supporting_file_best_effort(workspace_dir, path))
            .map(|prompt| normalize_queue_idle_review_prompt_markdown(&prompt));

        Ok(QueueIdleReviewContext {
            policy: directions.queue_idle.policy,
            prompt_path,
            prompt_markdown,
        })
    }

    pub fn stage_detail_doc_editor_session(
        &self,
        workspace_dir: &str,
        direction_id: &str,
    ) -> Result<PlanningDraftEditorSession> {
        let mut workspace = self.load_complete_workspace(workspace_dir)?;
        let selected_direction = workspace
            .directions
            .directions
            .iter()
            .find(|direction| direction.id.trim() == direction_id.trim())
            .ok_or_else(|| anyhow!("unknown direction id: {}", direction_id.trim()))?;
        let (detail_doc_path, detail_doc_body) = self.resolve_detail_doc_editor_target(
            workspace_dir,
            direction_id,
            trimmed_non_empty(selected_direction.detail_doc_path.as_str()),
        )?;
        set_direction_detail_doc_path(&mut workspace.directions, direction_id, &detail_doc_path)?;
        self.commit_direction_catalog(workspace_dir, &workspace.directions)?;
        workspace
            .extra_files
            .retain(|file| file.active_path != detail_doc_path);
        workspace.extra_files.push(PlanningDraftFileRecord {
            active_path: detail_doc_path.clone(),
            body: detail_doc_body,
        });

        self.stage_session_from_source(workspace_dir, workspace, &[detail_doc_path])
    }

    pub fn stage_queue_idle_prompt_editor_session(
        &self,
        workspace_dir: &str,
    ) -> Result<PlanningDraftEditorSession> {
        let mut workspace = self.load_complete_workspace(workspace_dir)?;
        let (prompt_path, prompt_body) = self.resolve_queue_idle_prompt_editor_target(
            workspace_dir,
            trimmed_non_empty(workspace.directions.queue_idle.prompt_path.as_str()),
        )?;
        set_queue_idle_prompt_path(&mut workspace.directions, &prompt_path);
        self.commit_direction_catalog(workspace_dir, &workspace.directions)?;
        workspace
            .extra_files
            .retain(|file| file.active_path != prompt_path);
        workspace.extra_files.push(PlanningDraftFileRecord {
            active_path: prompt_path.clone(),
            body: prompt_body,
        });

        self.stage_session_from_source(workspace_dir, workspace, &[prompt_path])
    }

    #[cfg(test)]
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
                set_direction_detail_doc_path(&mut directions, &direction.id, &target_path)?;
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
                set_queue_idle_prompt_path(&mut directions, &target_prompt_path);
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

    fn stage_session_from_source(
        &self,
        workspace_dir: &str,
        source: ActiveDirectionsWorkspace,
        editable_paths: &[String],
    ) -> Result<PlanningDraftEditorSession> {
        let draft_name = build_maintenance_draft_name();
        let mut files = vec![PlanningDraftFileRecord {
            active_path: RESULT_OUTPUT_FILE_PATH.to_string(),
            body: source.result_output_markdown,
        }];
        files.extend(source.extra_files);
        self.planning_workspace_port.stage_planning_draft_files(
            workspace_dir,
            &draft_name,
            &files,
        )?;
        let loaded = self
            .planning_workspace_port
            .load_planning_draft_files(workspace_dir, &draft_name)?;
        let validation_report =
            self.validate_loaded_draft(workspace_dir, &source.directions, &loaded)?;

        let editable_path_set = editable_paths.iter().cloned().collect::<HashSet<_>>();
        Ok(PlanningDraftEditorSession {
            draft_name: loaded.draft_name.clone(),
            draft_directory: loaded.draft_directory.clone(),
            editable_files: loaded
                .staged_files
                .into_iter()
                .filter(|file| editable_path_set.contains(file.active_path.as_str()))
                .map(|file| PlanningDraftEditorFile {
                    active_path: file.active_path,
                    staged_path: file.staged_path,
                    body: file.body,
                })
                .collect(),
            validation_report,
        })
    }

    fn load_complete_workspace(&self, workspace_dir: &str) -> Result<ActiveDirectionsWorkspace> {
        self.authority_seed_service
            .ensure_default_authority(workspace_dir)?;
        let workspace = self
            .planning_workspace_port
            .load_planning_workspace_files(workspace_dir)?;
        let directions = self.load_direction_catalog(workspace_dir)?;
        let mut active_workspace = ActiveDirectionsWorkspace {
            directions,
            result_output_markdown: workspace.result_output_markdown.ok_or_else(|| {
                anyhow!("default planning authority seed did not provide result output")
            })?,
            extra_files: Vec::new(),
        };
        let mut supporting_paths = HashSet::new();
        if let Some(prompt_path) =
            trimmed_non_empty(active_workspace.directions.queue_idle.prompt_path.as_str())
        {
            supporting_paths.insert(prompt_path.to_string());
        }
        supporting_paths.extend(
            active_workspace
                .directions
                .directions
                .iter()
                .filter_map(|direction| trimmed_non_empty(direction.detail_doc_path.as_str()))
                .map(str::to_string),
        );
        for supporting_path in supporting_paths {
            if let Some(body) =
                self.load_supporting_file_best_effort(workspace_dir, &supporting_path)
            {
                active_workspace.extra_files.push(PlanningDraftFileRecord {
                    active_path: supporting_path,
                    body,
                });
            }
        }
        Ok(active_workspace)
    }

    #[cfg(test)]
    #[allow(dead_code)]
    fn validate_active_workspace(&self, workspace_dir: &str) -> Result<PlanningValidationReport> {
        self.authority_seed_service
            .ensure_default_authority(workspace_dir)?;
        let workspace = self
            .planning_workspace_port
            .load_planning_workspace_files(workspace_dir)?;
        let directions = self.load_direction_catalog(workspace_dir)?;
        let mut result = self.planning_validation_service.validate_workspace_files(
            crate::domain::planning::PlanningWorkspaceFiles {
                directions: &directions,
                task_authority_json: "{\"version\":1,\"tasks\":[]}",
                result_output_markdown: workspace.result_output_markdown.as_deref().ok_or_else(
                    || anyhow!("default planning authority seed did not provide result output"),
                )?,
            },
        );
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

    fn validate_loaded_draft(
        &self,
        workspace_dir: &str,
        directions: &DirectionCatalogDocument,
        loaded: &PlanningDraftLoadRecord,
    ) -> Result<PlanningValidationReport> {
        let staged_file_map = loaded
            .staged_files
            .iter()
            .map(|file| (file.active_path.as_str(), file.body.as_str()))
            .collect::<std::collections::HashMap<_, _>>();
        let result_output_markdown =
            if let Some(body) = staged_file_map.get(RESULT_OUTPUT_FILE_PATH).copied() {
                body.to_string()
            } else {
                self.planning_workspace_port
                    .load_optional_planning_file(workspace_dir, RESULT_OUTPUT_FILE_PATH)?
                    .unwrap_or_default()
            };
        let mut result = self.planning_validation_service.validate_workspace_files(
            crate::domain::planning::PlanningWorkspaceFiles {
                directions,
                task_authority_json: "{\"version\":1,\"tasks\":[]}",
                result_output_markdown: &result_output_markdown,
            },
        );
        self.planning_validation_service
            .validate_direction_supporting_files(
                directions,
                |path| {
                    staged_file_map.contains_key(path)
                        || self
                            .load_supporting_file_best_effort(workspace_dir, path)
                            .is_some()
                },
                &mut result.report,
            );
        Ok(result.report)
    }

    fn load_supporting_file_best_effort(
        &self,
        workspace_dir: &str,
        relative_path: &str,
    ) -> Option<String> {
        self.planning_workspace_port
            .load_optional_planning_file(workspace_dir, relative_path)
            .ok()
            .flatten()
    }

    fn supporting_file_status(
        &self,
        workspace_dir: &str,
        configured_path: Option<&str>,
        required_prefix: &str,
    ) -> DirectionsSupportingFileStatus {
        let Some(path) = configured_path else {
            return DirectionsSupportingFileStatus::MissingMapping;
        };
        if !is_valid_planning_markdown_path(path, required_prefix) {
            return DirectionsSupportingFileStatus::BrokenMapping;
        }
        if self
            .load_supporting_file_best_effort(workspace_dir, path)
            .is_some()
        {
            DirectionsSupportingFileStatus::Ready
        } else {
            DirectionsSupportingFileStatus::BrokenMapping
        }
    }

    fn resolve_detail_doc_editor_target(
        &self,
        workspace_dir: &str,
        direction_id: &str,
        configured_path: Option<&str>,
    ) -> Result<(String, String)> {
        if let Some(path) = configured_path
            .filter(|path| is_valid_planning_markdown_path(path, PLANNING_DIRECTION_DOCS_DIRECTORY))
        {
            match self
                .planning_workspace_port
                .load_optional_planning_file(workspace_dir, path)
            {
                Ok(Some(body)) => return Ok((path.to_string(), body)),
                Ok(None) => return Ok((path.to_string(), String::new())),
                Err(_) => {}
            }
        }

        let fallback_path = default_direction_detail_doc_path(direction_id);
        let fallback_body = self
            .load_supporting_file_best_effort(workspace_dir, &fallback_path)
            .unwrap_or_default();
        Ok((fallback_path, fallback_body))
    }

    fn resolve_queue_idle_prompt_editor_target(
        &self,
        workspace_dir: &str,
        configured_path: Option<&str>,
    ) -> Result<(String, String)> {
        if let Some(path) = configured_path
            .filter(|path| is_valid_planning_markdown_path(path, PLANNING_PROMPTS_DIRECTORY))
        {
            match self
                .planning_workspace_port
                .load_optional_planning_file(workspace_dir, path)
            {
                Ok(Some(body)) => return Ok((path.to_string(), body)),
                Ok(None) => {
                    return Ok((
                        path.to_string(),
                        DEFAULT_QUEUE_IDLE_REVIEW_PROMPT_MARKDOWN.to_string(),
                    ));
                }
                Err(_) => {}
            }
        }

        let fallback_path = DEFAULT_QUEUE_IDLE_PROMPT_FILE_PATH.to_string();
        let fallback_body = self
            .load_supporting_file_best_effort(workspace_dir, &fallback_path)
            .unwrap_or_else(|| DEFAULT_QUEUE_IDLE_REVIEW_PROMPT_MARKDOWN.to_string());
        Ok((fallback_path, fallback_body))
    }
}

struct ActiveDirectionsWorkspace {
    directions: DirectionCatalogDocument,
    result_output_markdown: String,
    extra_files: Vec<PlanningDraftFileRecord>,
}

fn trimmed_non_empty(value: &str) -> Option<&str> {
    let value = value.trim();
    (!value.is_empty()).then_some(value)
}

fn normalize_queue_idle_review_prompt_markdown(prompt_markdown: &str) -> String {
    if is_legacy_queue_idle_review_prompt(prompt_markdown) {
        DEFAULT_QUEUE_IDLE_REVIEW_PROMPT_MARKDOWN.to_string()
    } else {
        prompt_markdown.to_string()
    }
}

fn is_legacy_queue_idle_review_prompt(prompt_markdown: &str) -> bool {
    let normalized = prompt_markdown.to_lowercase();
    [
        "directions.toml",
        "task-ledger",
        "task catalog compatibility",
        "latest answer clearly implies",
        "latest accepted answer",
    ]
    .iter()
    .any(|legacy_marker| normalized.contains(legacy_marker))
}

fn build_maintenance_draft_name() -> String {
    let now = Utc::now();
    format!(
        "directions-{}Z-{:09}",
        now.format("%Y%m%dT%H%M%S"),
        now.timestamp_subsec_nanos()
    )
}

#[cfg(test)]
#[allow(dead_code)]
fn build_default_detail_doc_markdown(
    direction: &crate::domain::planning::DirectionDefinition,
) -> String {
    let mut lines = vec![
        format!("# {}", direction.title.trim()),
        String::new(),
        format!("- Direction id: `{}`", direction.id.trim()),
        String::new(),
        "## Goal".to_string(),
        String::new(),
        direction.summary.trim().to_string(),
    ];
    if !direction.success_criteria.is_empty() {
        lines.push(String::new());
        lines.push("## Success criteria".to_string());
        lines.push(String::new());
        lines.extend(
            direction
                .success_criteria
                .iter()
                .map(|criterion| format!("- {}", criterion.trim())),
        );
    }
    if !direction.scope_hints.is_empty() {
        lines.push(String::new());
        lines.push("## Scope hints".to_string());
        lines.push(String::new());
        lines.extend(
            direction
                .scope_hints
                .iter()
                .map(|hint| format!("- {}", hint.trim())),
        );
    }
    lines.join("\n")
}

#[cfg(test)]
#[allow(dead_code)]
fn default_validated_direction_detail_doc_path(direction_id: &str) -> Result<String> {
    let fallback_path = default_direction_detail_doc_path(direction_id);
    if is_valid_planning_markdown_path(&fallback_path, PLANNING_DIRECTION_DOCS_DIRECTORY) {
        Ok(fallback_path)
    } else {
        Err(anyhow!(
            "direction {} does not produce a safe default detail_doc_path",
            direction_id.trim()
        ))
    }
}

fn is_valid_planning_markdown_path(path: &str, required_prefix: &str) -> bool {
    let normalized = path.trim().replace('\\', "/");
    if normalized.is_empty()
        || normalized.starts_with('/')
        || normalized.contains("../")
        || normalized.contains("/..")
        || Path::new(&normalized)
            .components()
            .any(|component| matches!(component, Component::ParentDir))
    {
        return false;
    }

    let Some(suffix) = normalized.strip_prefix(required_prefix) else {
        return false;
    };

    suffix.starts_with('/') && suffix.len() > 1 && normalized.ends_with(".md")
}

fn set_direction_detail_doc_path(
    directions: &mut DirectionCatalogDocument,
    direction_id: &str,
    detail_doc_path: &str,
) -> Result<()> {
    let Some(direction) = directions
        .directions
        .iter_mut()
        .find(|direction| direction.id.trim() == direction_id.trim())
    else {
        return Err(anyhow!("unknown direction id: {}", direction_id.trim()));
    };
    direction.detail_doc_path = detail_doc_path.to_string();
    Ok(())
}

fn set_queue_idle_prompt_path(directions: &mut DirectionCatalogDocument, prompt_path: &str) {
    directions.queue_idle.prompt_path = prompt_path.to_string();
}

#[cfg(test)]
mod tests {
    use super::normalize_queue_idle_review_prompt_markdown;
    use crate::application::service::planning::shared::auto_follow_copy::DEFAULT_QUEUE_IDLE_REVIEW_PROMPT_MARKDOWN;

    #[test]
    fn queue_idle_review_prompt_normalizes_legacy_file_authority_copy() {
        let legacy_prompt = r#"# Queue Idle Review Prompt

- `directions.toml`의 direction 목표, success criteria, detail doc를 기준으로 현재 task-ledger work list를 다시 점검하세요.
- When the latest answer clearly implies a next step, derive it.
"#;

        let normalized = normalize_queue_idle_review_prompt_markdown(legacy_prompt);

        assert_eq!(normalized, DEFAULT_QUEUE_IDLE_REVIEW_PROMPT_MARKDOWN);
        assert!(normalized.contains("[accepted-db-direction-authority]"));
        assert!(normalized.contains("[accepted-db-task-authority]"));
        assert!(normalized.contains("[db-queue-projection]"));
        assert!(!normalized.contains("directions.toml"));
        assert!(!normalized.contains("task-ledger"));
        assert!(!normalized.contains("latest answer clearly implies"));
    }

    #[test]
    fn queue_idle_review_prompt_keeps_db_authority_copy() {
        let prompt = "# Queue Idle Review Prompt\n\n- Use accepted DB authority.";

        assert_eq!(normalize_queue_idle_review_prompt_markdown(prompt), prompt);
    }
}
