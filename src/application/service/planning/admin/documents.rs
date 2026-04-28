use std::collections::BTreeSet;
use std::sync::OnceLock;

use anyhow::{Context, Result, anyhow, bail};
use chrono::Utc;

use crate::application::port::outbound::planning_task_repository_port::{
    PlanningTaskAuthorityCommit, PlanningTaskAuthorityCommitResult,
};
use crate::application::service::planning::authoring::bootstrap::{
    PlanningBootstrapMode, PlanningBootstrapService,
};
use crate::application::service::planning::shared::authority_seed::PlanningAuthoritySeedService;
use crate::application::service::planning::{DIRECTIONS_FILE_PATH, RESULT_OUTPUT_FILE_PATH};
use crate::domain::planning::{
    DirectionCatalogDocument, DirectionDefinition, DirectionState, PlanningWorkspaceFiles,
    TaskActor, TaskAuthorityDocument, TaskDefinition, TaskStatus,
};

use super::{
    PlanningAdminDirectionMutationRequest, PlanningAdminFacadeService,
    PlanningAdminTaskMutationRequest,
};

pub(super) const DEFAULT_DIRECTION_ID: &str = "general-workstream";
const GENERATED_DIRECTION_ID_PREFIX: &str = "dir";
const GENERATED_TASK_ID_PREFIX: &str = "task";
static DEFAULT_DIRECTION_DEFINITION: OnceLock<Result<DirectionDefinition, String>> =
    OnceLock::new();

impl PlanningAdminFacadeService {
    pub(super) fn ensure_default_authority(&self) -> Result<()> {
        PlanningAuthoritySeedService::new(
            self.planning_workspace_port.clone(),
            self.planning_task_repository_port.clone(),
            self.planning_validation_service.clone(),
            self.priority_queue_service.clone(),
        )
        .ensure_default_authority(self.workspace_dir.as_str())
        .map(|_| ())
    }

    pub(super) fn load_admin_documents(&self) -> Result<PlanningAdminDocuments> {
        self.ensure_default_authority()?;
        let workspace = self
            .planning_workspace_port
            .load_planning_workspace_files(self.workspace_dir.as_str())?;
        let directions_toml = workspace
            .directions_toml
            .ok_or_else(|| anyhow!("default planning authority seed did not provide directions"))?;
        let result_output_markdown = workspace.result_output_markdown.ok_or_else(|| {
            anyhow!("default planning authority seed did not provide result output")
        })?;
        let task_authority_snapshot = self
            .planning_task_repository_port
            .load_task_authority_snapshot(self.workspace_dir.as_str())?
            .ok_or_else(|| {
                anyhow!("default planning authority seed did not provide task authority")
            })?;
        let directions = toml::from_str::<DirectionCatalogDocument>(&directions_toml)
            .context("failed to parse active directions.toml")?;
        Ok(PlanningAdminDocuments {
            directions,
            task_authority: task_authority_snapshot.task_authority,
            result_output_markdown,
            observed_planning_revision: Some(task_authority_snapshot.planning_revision),
        })
    }

    pub(super) fn commit_admin_documents(
        &self,
        mut documents: PlanningAdminDocuments,
    ) -> Result<()> {
        ensure_default_direction(&mut documents.directions)?;
        remove_tasks_with_unresolved_directions(&mut documents);

        let directions_toml = toml::to_string_pretty(&documents.directions)?;
        let task_authority_json = serde_json::to_string_pretty(&documents.task_authority)?;
        let validation_result =
            self.planning_validation_service
                .validate_workspace_files(PlanningWorkspaceFiles {
                    directions_toml: &directions_toml,
                    task_authority_json: &task_authority_json,
                    result_output_markdown: &documents.result_output_markdown,
                });
        if !validation_result.report.is_valid() {
            bail!(
                "planning mutation failed validation: {}",
                validation_result
                    .report
                    .issues
                    .iter()
                    .map(|issue| issue.message.as_str())
                    .collect::<Vec<_>>()
                    .join("; ")
            );
        }
        let queue_projection = self
            .priority_queue_service
            .build_projection(&documents.directions, &documents.task_authority)
            .context("failed to rebuild planning queue")?;

        match self
            .planning_task_repository_port
            .commit_task_authority_snapshot(
                self.workspace_dir.as_str(),
                PlanningTaskAuthorityCommit {
                    observed_planning_revision: documents.observed_planning_revision,
                    task_authority: &documents.task_authority,
                    queue_projection: &queue_projection,
                },
            )? {
            PlanningTaskAuthorityCommitResult::Committed { .. } => {}
            PlanningTaskAuthorityCommitResult::Conflict {
                observed_planning_revision,
                current_planning_revision,
            } => {
                bail!(
                    "planning db changed while editing (observed revision {observed_planning_revision}, current revision {current_planning_revision}); reload and retry"
                );
            }
        }
        self.planning_workspace_port
            .replace_planning_workspace_file(
                self.workspace_dir.as_str(),
                DIRECTIONS_FILE_PATH,
                Some(&directions_toml),
            )?;
        self.planning_workspace_port
            .replace_planning_workspace_file(
                self.workspace_dir.as_str(),
                RESULT_OUTPUT_FILE_PATH,
                Some(&documents.result_output_markdown),
            )?;
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub(super) struct PlanningAdminDocuments {
    pub(super) directions: DirectionCatalogDocument,
    pub(super) task_authority: TaskAuthorityDocument,
    pub(super) result_output_markdown: String,
    observed_planning_revision: Option<i64>,
}

pub(super) fn direction_from_request(
    request: PlanningAdminDirectionMutationRequest,
    directions: &DirectionCatalogDocument,
) -> Result<DirectionDefinition> {
    let title = normalized_required_text(&request.title, "direction title")?;
    let id = if request.id.trim().is_empty() {
        generated_unique_id(
            GENERATED_DIRECTION_ID_PREFIX,
            title,
            directions
                .directions
                .iter()
                .map(|direction| direction.id.trim()),
        )
    } else {
        normalized_required_id(&request.id, "direction id")?.to_string()
    };
    let success_criteria = split_lines(&request.success_criteria_text);
    if success_criteria.is_empty() {
        bail!("direction `{id}` requires at least one success criterion");
    }
    Ok(DirectionDefinition {
        id,
        title: title.to_string(),
        summary: non_empty_or(&request.summary, title),
        success_criteria,
        scope_hints: split_lines(&request.scope_hints_text),
        detail_doc_path: request.detail_doc_path.trim().to_string(),
        state: parse_direction_state(&request.state)?,
    })
}

pub(super) fn task_from_request(
    request: PlanningAdminTaskMutationRequest,
    task_authority: &TaskAuthorityDocument,
    default_direction_id: &str,
) -> Result<TaskDefinition> {
    let title = normalized_required_text(&request.title, "task title")?;
    let id = if request.id.trim().is_empty() {
        generated_unique_id(
            GENERATED_TASK_ID_PREFIX,
            title,
            task_authority.tasks.iter().map(|task| task.id.trim()),
        )
    } else {
        normalized_required_id(&request.id, "task id")?.to_string()
    };
    let now = Utc::now().to_rfc3339();
    let existing = task_authority
        .tasks
        .iter()
        .find(|task| task.id.trim() == id.as_str())
        .cloned();
    let direction_id = if request.direction_id.trim().is_empty() {
        default_direction_id.to_string()
    } else {
        normalized_required_id(&request.direction_id, "direction id")?.to_string()
    };
    let base_priority = parse_i32_or_default(&request.base_priority, 10, "base priority")?;
    let dynamic_priority_delta =
        parse_i32_or_default(&request.dynamic_priority_delta, 0, "dynamic priority delta")?;
    Ok(TaskDefinition {
        id,
        direction_id,
        direction_relation_note: existing
            .as_ref()
            .map(|task| task.direction_relation_note.clone())
            .unwrap_or_default(),
        title: title.to_string(),
        description: non_empty_or(&request.description, title),
        status: parse_task_status(&request.status)?,
        base_priority,
        dynamic_priority_delta,
        priority_reason: request.priority_reason.trim().to_string(),
        depends_on: split_references(&request.depends_on_text),
        blocked_by: split_references(&request.blocked_by_text),
        created_by: existing
            .as_ref()
            .map(|task| task.created_by)
            .unwrap_or(TaskActor::User),
        last_updated_by: TaskActor::User,
        source_turn_id: existing.and_then(|task| task.source_turn_id),
        updated_at: now,
    })
}

pub(super) fn ensure_default_direction(directions: &mut DirectionCatalogDocument) -> Result<()> {
    if directions
        .directions
        .iter()
        .any(|direction| direction.id.trim() == DEFAULT_DIRECTION_ID)
    {
        return Ok(());
    }
    directions.directions.push(default_direction_definition()?);
    Ok(())
}

fn default_direction_definition() -> Result<DirectionDefinition> {
    DEFAULT_DIRECTION_DEFINITION
        .get_or_init(build_default_direction_definition)
        .clone()
        .map_err(|message| anyhow!(message))
}

fn build_default_direction_definition() -> Result<DirectionDefinition, String> {
    let artifacts =
        PlanningBootstrapService::new().build_artifacts_for_mode(PlanningBootstrapMode::Simple);
    let directions = toml::from_str::<DirectionCatalogDocument>(&artifacts.directions_toml)
        .map_err(|error| format!("failed to parse bootstrap default directions: {error}"))?;
    directions
        .directions
        .into_iter()
        .find(|direction| direction.id.trim() == DEFAULT_DIRECTION_ID)
        .ok_or_else(|| format!("bootstrap default direction `{DEFAULT_DIRECTION_ID}` is missing"))
}

fn remove_tasks_with_unresolved_directions(documents: &mut PlanningAdminDocuments) {
    let direction_ids = documents
        .directions
        .directions
        .iter()
        .map(|direction| direction.id.trim())
        .collect::<BTreeSet<_>>();
    let mut removed_task_ids = BTreeSet::new();
    documents.task_authority.tasks.retain(|task| {
        let should_keep = direction_ids.contains(task.direction_id.trim());
        if !should_keep {
            removed_task_ids.insert(task.id.trim().to_string());
        }
        should_keep
    });
    if removed_task_ids.is_empty() {
        return;
    }
    remove_task_references(&mut documents.task_authority, &removed_task_ids);
}

fn parse_direction_state(raw: &str) -> Result<DirectionState> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "" | "active" => Ok(DirectionState::Active),
        "paused" => Ok(DirectionState::Paused),
        "done" => Ok(DirectionState::Done),
        other => bail!("unknown direction state `{other}`"),
    }
}

fn parse_task_status(raw: &str) -> Result<TaskStatus> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "" | "ready" => Ok(TaskStatus::Ready),
        "blocked" => Ok(TaskStatus::Blocked),
        "in_progress" => Ok(TaskStatus::InProgress),
        "done" => Ok(TaskStatus::Done),
        "cancelled" => Ok(TaskStatus::Cancelled),
        "awaiting_user" => Ok(TaskStatus::AwaitingUser),
        "proposed" => Ok(TaskStatus::Proposed),
        other => bail!("unknown task status `{other}`"),
    }
}

pub(super) fn default_direction_id(directions: &DirectionCatalogDocument) -> Result<&str> {
    if let Some(direction) = directions
        .directions
        .iter()
        .find(|direction| direction.id.trim() == DEFAULT_DIRECTION_ID)
    {
        return Ok(direction.id.trim());
    }
    directions
        .directions
        .iter()
        .find(|direction| direction.state == DirectionState::Active)
        .or_else(|| directions.directions.first())
        .map(|direction| direction.id.trim())
        .filter(|id| !id.is_empty())
        .ok_or_else(|| anyhow!("at least one direction is required"))
}

pub(super) fn ensure_direction_exists(
    directions: &DirectionCatalogDocument,
    direction_id: &str,
) -> Result<()> {
    if directions
        .directions
        .iter()
        .any(|direction| direction.id.trim() == direction_id.trim())
    {
        return Ok(());
    }
    bail!("direction `{}` does not exist", direction_id.trim())
}

pub(super) fn normalized_required_id<'a>(value: &'a str, label: &str) -> Result<&'a str> {
    let value = value.trim();
    if value.is_empty() {
        bail!("{label} is required");
    }
    if value.contains(char::is_whitespace) || value.contains('/') || value.contains('\\') {
        bail!("{label} `{value}` must not contain whitespace or path separators");
    }
    Ok(value)
}

fn normalized_required_text<'a>(value: &'a str, label: &str) -> Result<&'a str> {
    let value = value.trim();
    if value.is_empty() {
        bail!("{label} is required");
    }
    Ok(value)
}

fn non_empty_or(value: &str, fallback: &str) -> String {
    let value = value.trim();
    if value.is_empty() {
        fallback.to_string()
    } else {
        value.to_string()
    }
}

fn generated_unique_id<'a>(
    prefix: &str,
    title: &str,
    existing_ids: impl IntoIterator<Item = &'a str>,
) -> String {
    let existing = existing_ids.into_iter().collect::<BTreeSet<_>>();
    let slug = slugify_title(title);
    let base = format!("{prefix}-{slug}");
    if !existing.contains(base.as_str()) {
        return base;
    }

    for suffix in 2.. {
        let candidate = format!("{base}-{suffix}");
        if !existing.contains(candidate.as_str()) {
            return candidate;
        }
    }
    unreachable!("numeric suffix search should eventually find an unused id")
}

fn slugify_title(title: &str) -> String {
    let mut slug = String::new();
    let mut previous_dash = false;
    for character in title.chars().flat_map(char::to_lowercase) {
        if character.is_alphanumeric() {
            slug.push(character);
            previous_dash = false;
        } else if !previous_dash && !slug.is_empty() {
            slug.push('-');
            previous_dash = true;
        }
    }
    while slug.ends_with('-') {
        slug.pop();
    }
    if slug.is_empty() {
        "item".to_string()
    } else {
        slug
    }
}

fn parse_i32_or_default(raw: &str, default: i32, label: &str) -> Result<i32> {
    let raw = raw.trim();
    if raw.is_empty() {
        return Ok(default);
    }
    raw.parse::<i32>()
        .with_context(|| format!("{label} must be an integer"))
}

fn split_lines(raw: &str) -> Vec<String> {
    raw.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_string)
        .collect()
}

fn split_references(raw: &str) -> Vec<String> {
    raw.split([',', '\n'])
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .map(str::to_string)
        .collect()
}

pub(super) fn remove_task_references(
    task_authority: &mut TaskAuthorityDocument,
    removed_task_ids: &BTreeSet<String>,
) {
    for task in &mut task_authority.tasks {
        task.depends_on
            .retain(|task_id| !removed_task_ids.contains(task_id.trim()));
        task.blocked_by
            .retain(|task_id| !removed_task_ids.contains(task_id.trim()));
    }
}

#[cfg(test)]
mod tests {
    use super::{generated_unique_id, slugify_title};

    #[test]
    fn slugify_title_preserves_unicode_alphanumerics() {
        assert_eq!(slugify_title("한글 작업 1"), "한글-작업-1");
    }

    #[test]
    fn generated_unique_id_keeps_unicode_title_identity() {
        let existing = ["task-한글-작업", "task-한글-작업-2"];

        assert_eq!(
            generated_unique_id("task", "한글 작업", existing),
            "task-한글-작업-3"
        );
    }
}
