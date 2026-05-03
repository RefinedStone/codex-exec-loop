use std::collections::BTreeSet;
use std::sync::OnceLock;

use anyhow::{Context, Result, anyhow, bail};

use super::{PlanningAdminDirectionMutationRequest, PlanningAdminFacadeService};
use crate::application::port::outbound::planning_task_repository_port::{
    PlanningDirectionAuthorityCommit, PlanningTaskAuthorityCommit,
    PlanningTaskAuthorityCommitResult,
};
use crate::application::service::planning::RESULT_OUTPUT_FILE_PATH;
use crate::application::service::planning::authoring::bootstrap::{
    PlanningBootstrapMode, PlanningBootstrapService,
};
use crate::application::service::planning::shared::authority_seed::PlanningAuthoritySeedService;
use crate::domain::planning::{
    DirectionCatalogDocument, DirectionDefinition, DirectionState, PlanningWorkspaceFiles,
    TaskAuthorityDocument,
};

/*
 * This module is the write boundary for admin-edited planning authority. Admin
 * forms and draft files are operator-friendly text, but committed authority is
 * split between DB-backed direction/task snapshots and the workspace result
 * markdown. The functions below keep those stores in revision order and repair
 * small consistency gaps before validation.
 */
pub(super) const DEFAULT_DIRECTION_ID: &str = "general-workstream";
const GENERATED_DIRECTION_ID_PREFIX: &str = "dir";

// The default direction is derived from bootstrap artifacts. Cache the parsed
// definition so repeated admin loads do not rebuild the bootstrap bundle.
static DEFAULT_DIRECTION_DEFINITION: OnceLock<Result<DirectionDefinition, String>> =
    OnceLock::new();

impl PlanningAdminFacadeService {
    pub(super) fn ensure_default_authority(&self) -> Result<()> {
        // Admin pages can be the first planning entry point in a workspace, so
        // they seed the same authority baseline as runtime startup.
        PlanningAuthoritySeedService::new(
            self.planning_workspace_port.clone(),
            self.planning_task_repository_port.clone(),
            self.planning_validation_service.clone(),
            self.priority_queue_service.clone(),
        )
        .ensure_default_authority(self.workspace_dir.as_str())
        .map(|_| ())
    }
    pub(super) fn load_operator_planning_documents(
        &self,
    ) -> Result<PlanningOperatorPlanningDocuments> {
        // Load uses the repository snapshots as authority and only reads
        // result_output from the workspace file system. The observed revision is
        // carried forward for optimistic concurrency during commit.
        self.ensure_default_authority()?;
        let workspace = self
            .planning_workspace_port
            .load_planning_workspace_files(self.workspace_dir.as_str())?;
        let result_output_markdown = workspace.result_output_markdown.ok_or_else(|| {
            anyhow!("default planning authority seed did not provide result output")
        })?;
        let direction_authority_snapshot = self
            .planning_task_repository_port
            .load_direction_authority_snapshot(self.workspace_dir.as_str())?
            .ok_or_else(|| {
                anyhow!("default planning authority seed did not provide direction authority")
            })?;
        let task_authority_snapshot = self
            .planning_task_repository_port
            .load_task_authority_snapshot(self.workspace_dir.as_str())?
            .ok_or_else(|| {
                anyhow!("default planning authority seed did not provide task authority")
            })?;
        Ok(PlanningOperatorPlanningDocuments {
            directions: direction_authority_snapshot.directions,
            task_authority: task_authority_snapshot.task_authority,
            result_output_markdown,
            observed_planning_revision: Some(task_authority_snapshot.planning_revision),
        })
    }
    pub(super) fn commit_operator_planning_documents(
        &self,
        mut documents: PlanningOperatorPlanningDocuments,
    ) -> Result<()> {
        // Admin edits may delete directions before cleaning dependent tasks.
        // Normalize that graph first, then validate the exact files that would
        // be persisted.
        ensure_default_direction(&mut documents.directions)?;
        remove_tasks_with_unresolved_directions(&mut documents);

        let task_authority_json = serde_json::to_string_pretty(&documents.task_authority)?;
        let validation_result =
            self.planning_validation_service
                .validate_workspace_files(PlanningWorkspaceFiles {
                    directions: &documents.directions,
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
        let task_observed_revision = match self
            .planning_task_repository_port
            .commit_direction_authority_snapshot(
                self.workspace_dir.as_str(),
                PlanningDirectionAuthorityCommit {
                    observed_planning_revision: documents.observed_planning_revision,
                    directions: &documents.directions,
                },
            )? {
            PlanningTaskAuthorityCommitResult::Committed { planning_revision } => planning_revision,
            PlanningTaskAuthorityCommitResult::Conflict {
                observed_planning_revision,
                current_planning_revision,
            } => {
                bail!(
                    "planning db changed while editing directions (observed revision {observed_planning_revision}, current revision {current_planning_revision}); reload and retry"
                );
            }
        };
        // Direction and task authority share the same planning DB revision. After the
        // direction snapshot commits, the task snapshot must observe that new revision.
        match self
            .planning_task_repository_port
            .commit_task_authority_snapshot(
                self.workspace_dir.as_str(),
                PlanningTaskAuthorityCommit {
                    observed_planning_revision: Some(task_observed_revision),
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
        // result_output is still file-backed, so it is written last after DB
        // authority and queue projection have accepted the mutation.
        self.planning_workspace_port
            .replace_planning_workspace_file(
                self.workspace_dir.as_str(),
                RESULT_OUTPUT_FILE_PATH,
                Some(&documents.result_output_markdown),
            )?;
        Ok(())
    }
}

/*
 * A loaded admin edit session spans all planning authority stores. The revision
 * tracks the DB snapshots only; result_output is committed after those snapshots
 * because it does not participate in repository conflict detection.
 */
#[derive(Debug, Clone)]
pub(super) struct PlanningOperatorPlanningDocuments {
    pub(super) directions: DirectionCatalogDocument,
    pub(super) task_authority: TaskAuthorityDocument,
    pub(super) result_output_markdown: String,
    observed_planning_revision: Option<i64>,
}

pub(super) fn direction_from_request(
    request: PlanningAdminDirectionMutationRequest,
    directions: &DirectionCatalogDocument,
) -> Result<DirectionDefinition> {
    // Direction forms can either update an existing id or create a stable id
    // from the title. Success criteria are mandatory because queue-idle review
    // treats them as completion authority.
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

pub(super) fn ensure_default_direction(directions: &mut DirectionCatalogDocument) -> Result<()> {
    // The default direction is a compatibility anchor for blank task forms and
    // older planning data. Missing it is repaired before committing admin edits.
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
    // Source the default from the same bootstrap path used to create a new
    // workspace so admin repair and first-run initialization cannot drift.
    let artifacts =
        PlanningBootstrapService::new().build_artifacts_for_mode(PlanningBootstrapMode::Simple);
    artifacts
        .directions
        .directions
        .into_iter()
        .find(|direction| direction.id.trim() == DEFAULT_DIRECTION_ID)
        .ok_or_else(|| format!("bootstrap default direction `{DEFAULT_DIRECTION_ID}` is missing"))
}

fn remove_tasks_with_unresolved_directions(documents: &mut PlanningOperatorPlanningDocuments) {
    // If a direction disappears, its tasks can no longer enter the queue. Remove
    // those tasks and then prune dependency/blocker edges that pointed at them.
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
    // Empty state defaults to active to keep simple creation forms concise.
    match raw.trim().to_ascii_lowercase().as_str() {
        "" | "active" => Ok(DirectionState::Active),
        "paused" => Ok(DirectionState::Paused),
        "done" => Ok(DirectionState::Done),
        other => bail!("unknown direction state `{other}`"),
    }
}

pub(super) fn default_direction_id(directions: &DirectionCatalogDocument) -> Result<&str> {
    // Prefer the compatibility default, then any active direction, then the
    // first non-empty id. This gives task creation a deterministic target even
    // while an operator is reshaping direction authority.
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

pub(super) fn normalized_required_id<'a>(value: &'a str, label: &str) -> Result<&'a str> {
    // IDs are used in references and generated paths; whitespace or separators
    // would make authority graph matching and route parameters ambiguous.
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
    // Generated ids are deterministic for the same title but still avoid
    // collisions inside the current authority document.
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
    // Keep Unicode alphanumerics so non-English direction titles retain meaning
    // in generated ids instead of collapsing to opaque counters.
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

fn split_lines(raw: &str) -> Vec<String> {
    // Admin forms edit list fields as text areas; blank lines are presentation
    // noise and should not become authority entries.
    raw.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_string)
        .collect()
}

pub(super) fn remove_task_references(
    task_authority: &mut TaskAuthorityDocument,
    removed_task_ids: &BTreeSet<String>,
) {
    // Reference cleanup trims both sides so legacy whitespace in authority files
    // does not keep a dangling dependency alive.
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
        // Generated ids should remain readable for non-English operator titles.
        assert_eq!(slugify_title("한글 작업 1"), "한글-작업-1");
    }

    #[test]
    fn generated_unique_id_keeps_unicode_title_identity() {
        // Collision suffixes append to the readable slug instead of replacing it.
        let existing = ["task-한글-작업", "task-한글-작업-2"];

        assert_eq!(
            generated_unique_id("task", "한글 작업", existing),
            "task-한글-작업-3"
        );
    }
}
