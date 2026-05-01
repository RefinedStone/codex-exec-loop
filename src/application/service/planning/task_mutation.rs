use std::sync::Arc;

use anyhow::{Result, anyhow, bail};
use chrono::{DateTime, Utc};

use crate::application::port::outbound::planning_task_repository_port::{
    PlanningTaskAuthorityCommit, PlanningTaskAuthorityCommitResult, PlanningTaskRepositoryPort,
};
use crate::domain::planning::{
    DirectionCatalogDocument, PLANNING_FORMAT_VERSION, PriorityQueueProjection,
    PriorityQueueService, PriorityQueueTask, TaskActor, TaskAuthorityDocument, TaskDefinition,
    TaskStatus,
};

const DEFAULT_TASK_PRIORITY: i32 = 80;
const MAX_REVISION_CONFLICT_RETRIES: usize = 3;
const MAX_COLLISION_SUFFIX_ATTEMPTS: u32 = 20;
const TASK_ID_HASH_CHARS: usize = 12;
const MAX_TASK_MUTATION_COMMANDS: usize = 16;

mod commands;
mod helpers;
mod validation;

pub use self::commands::{
    PlanningTaskCommandExtraction, PlanningTaskCreateInput, PlanningTaskMutationCommand,
    PlanningTaskUpdateInput, extract_planning_task_commands,
};
use self::helpers::{
    build_task_id, default_relation_note, direction_title, find_direction, format_timestamp,
    increment_suffix, normalize_references, required_id, required_text, select_direction,
    task_id_exists, terminal_status,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlanningTaskMutationSource {
    User,
    Llm,
    System,
}

impl PlanningTaskMutationSource {
    fn actor(self) -> TaskActor {
        match self {
            Self::User => TaskActor::User,
            Self::Llm => TaskActor::Llm,
            Self::System => TaskActor::System,
        }
    }

    fn id_slug(self) -> &'static str {
        match self {
            Self::User => "user",
            Self::Llm => "llm",
            Self::System => "system",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanningTaskMutationRequest {
    pub workspace_directory: String,
    pub source: PlanningTaskMutationSource,
    pub source_turn_id: Option<String>,
    pub commands: Vec<PlanningTaskMutationCommand>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanningTaskCreatePreviewRequest {
    pub workspace_directory: String,
    pub source: PlanningTaskMutationSource,
    pub source_turn_id: Option<String>,
    pub input: PlanningTaskCreateInput,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanningTaskCreatePreview {
    pub request: PlanningTaskCreatePreviewRequest,
    pub task: TaskDefinition,
    pub direction_title: String,
    pub generated_at: DateTime<Utc>,
    pub collision_suffix: Option<u32>,
    pub observed_planning_revision: i64,
    pub queue_head: Option<PriorityQueueTask>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanningTaskMutationCommitResult {
    pub committed_planning_revision: i64,
    pub queue_head: Option<PriorityQueueTask>,
    pub task_authority_changed: bool,
    pub applied_command_count: usize,
    pub committed_task_ids: Vec<String>,
}

#[derive(Clone)]
pub struct PlanningTaskMutationService {
    planning_task_repository_port: Arc<dyn PlanningTaskRepositoryPort>,
    priority_queue_service: PriorityQueueService,
}

impl PlanningTaskMutationService {
    pub fn new(
        planning_task_repository_port: Arc<dyn PlanningTaskRepositoryPort>,
        priority_queue_service: PriorityQueueService,
    ) -> Self {
        Self {
            planning_task_repository_port,
            priority_queue_service,
        }
    }

    pub fn preview_create_task(
        &self,
        request: PlanningTaskCreatePreviewRequest,
    ) -> Result<PlanningTaskCreatePreview> {
        let context = self.load_context(&request.workspace_directory)?;
        self.preview_create_task_with_authority(
            request,
            &context.directions,
            &context.task_authority,
            context.task_planning_revision,
        )
    }

    pub(crate) fn preview_create_task_with_authority(
        &self,
        request: PlanningTaskCreatePreviewRequest,
        directions: &DirectionCatalogDocument,
        task_authority: &TaskAuthorityDocument,
        task_planning_revision: i64,
    ) -> Result<PlanningTaskCreatePreview> {
        let generated_at = Utc::now();
        let task = self.build_unique_task(
            &request.input,
            request.source,
            request.source_turn_id.as_deref(),
            PlanningTaskAuthorityView {
                directions,
                task_authority,
            },
            generated_at,
            None,
        )?;
        let direction_title = direction_title(directions, &task.direction_id)
            .unwrap_or_else(|| task.direction_id.clone());
        let mut next_task_authority = task_authority.clone();
        next_task_authority.tasks.push(task.clone());
        let queue_projection = self.validate_and_project(directions, &next_task_authority)?;

        Ok(PlanningTaskCreatePreview {
            request,
            task,
            direction_title,
            generated_at,
            collision_suffix: None,
            observed_planning_revision: task_planning_revision,
            queue_head: queue_projection.next_task,
        })
    }

    pub fn commit_create_preview(
        &self,
        preview: &PlanningTaskCreatePreview,
    ) -> Result<PlanningTaskMutationCommitResult> {
        let mut observed_revision = preview.observed_planning_revision;
        let mut next_suffix = preview.collision_suffix;

        for _ in 0..=MAX_REVISION_CONFLICT_RETRIES {
            let context = self.load_context(&preview.request.workspace_directory)?;
            let task = if context.task_planning_revision == preview.observed_planning_revision
                && next_suffix == preview.collision_suffix
            {
                preview.task.clone()
            } else {
                self.build_unique_task(
                    &preview.request.input,
                    preview.request.source,
                    preview.request.source_turn_id.as_deref(),
                    PlanningTaskAuthorityView {
                        directions: &context.directions,
                        task_authority: &context.task_authority,
                    },
                    preview.generated_at,
                    next_suffix,
                )?
            };
            let committed_task_id = task.id.clone();
            let mut next_task_authority = context.task_authority.clone();
            next_task_authority.tasks.push(task);
            let queue_projection =
                self.validate_and_project(&context.directions, &next_task_authority)?;

            match self.commit_authority(
                &preview.request.workspace_directory,
                Some(observed_revision),
                &next_task_authority,
                &queue_projection,
            )? {
                PlanningTaskAuthorityCommitResult::Committed { planning_revision } => {
                    return Ok(PlanningTaskMutationCommitResult {
                        committed_planning_revision: planning_revision,
                        queue_head: queue_projection.next_task,
                        task_authority_changed: true,
                        applied_command_count: 1,
                        committed_task_ids: vec![committed_task_id],
                    });
                }
                PlanningTaskAuthorityCommitResult::Conflict {
                    current_planning_revision,
                    ..
                } => {
                    observed_revision = current_planning_revision;
                    next_suffix = increment_suffix(next_suffix);
                }
            }
        }

        bail!("planning task mutation could not commit because planning state kept changing")
    }

    pub fn apply_commands(
        &self,
        request: PlanningTaskMutationRequest,
    ) -> Result<PlanningTaskMutationCommitResult> {
        if request.commands.len() > MAX_TASK_MUTATION_COMMANDS {
            bail!(
                "planning task mutation accepts at most {MAX_TASK_MUTATION_COMMANDS} command(s) per worker response"
            );
        }
        if request.commands.is_empty() {
            let context = self.load_context(&request.workspace_directory)?;
            let queue_projection =
                self.validate_and_project(&context.directions, &context.task_authority)?;
            return Ok(PlanningTaskMutationCommitResult {
                committed_planning_revision: context.task_planning_revision,
                queue_head: queue_projection.next_task,
                task_authority_changed: false,
                applied_command_count: 0,
                committed_task_ids: Vec::new(),
            });
        }

        let mut observed_revision = None;
        for _ in 0..=MAX_REVISION_CONFLICT_RETRIES {
            let context = self.load_context(&request.workspace_directory)?;
            observed_revision = Some(context.task_planning_revision);
            let mut next_task_authority = context.task_authority.clone();
            let application = self.apply_commands_to_authority(
                &request,
                &context.directions,
                &mut next_task_authority,
                Utc::now(),
            )?;
            let queue_projection =
                self.validate_and_project(&context.directions, &next_task_authority)?;
            if !application.changed {
                return Ok(PlanningTaskMutationCommitResult {
                    committed_planning_revision: context.task_planning_revision,
                    queue_head: queue_projection.next_task,
                    task_authority_changed: false,
                    applied_command_count: 0,
                    committed_task_ids: application.committed_task_ids,
                });
            }

            match self.commit_authority(
                &request.workspace_directory,
                observed_revision,
                &next_task_authority,
                &queue_projection,
            )? {
                PlanningTaskAuthorityCommitResult::Committed { planning_revision } => {
                    return Ok(PlanningTaskMutationCommitResult {
                        committed_planning_revision: planning_revision,
                        queue_head: queue_projection.next_task,
                        task_authority_changed: true,
                        applied_command_count: request.commands.len(),
                        committed_task_ids: application.committed_task_ids,
                    });
                }
                PlanningTaskAuthorityCommitResult::Conflict { .. } => continue,
            }
        }

        let observed_revision = observed_revision.unwrap_or_default();
        bail!(
            "planning task mutation could not commit because planning state kept changing after observed revision {observed_revision}"
        )
    }

    fn load_context(&self, workspace_directory: &str) -> Result<PlanningTaskMutationContext> {
        let direction_snapshot = self
            .planning_task_repository_port
            .load_direction_authority_snapshot(workspace_directory)?
            .ok_or_else(|| anyhow!("planning direction authority is unavailable"))?;
        let task_snapshot = self
            .planning_task_repository_port
            .load_task_authority_snapshot(workspace_directory)?
            .ok_or_else(|| anyhow!("planning task authority is unavailable"))?;
        if direction_snapshot.directions.version != PLANNING_FORMAT_VERSION {
            bail!(
                "unsupported direction authority version {}; expected {}",
                direction_snapshot.directions.version,
                PLANNING_FORMAT_VERSION
            );
        }
        if task_snapshot.task_authority.version != PLANNING_FORMAT_VERSION {
            bail!(
                "unsupported task authority version {}; expected {}",
                task_snapshot.task_authority.version,
                PLANNING_FORMAT_VERSION
            );
        }

        Ok(PlanningTaskMutationContext {
            directions: direction_snapshot.directions,
            task_authority: task_snapshot.task_authority,
            task_planning_revision: task_snapshot.planning_revision,
        })
    }

    fn apply_commands_to_authority(
        &self,
        request: &PlanningTaskMutationRequest,
        directions: &DirectionCatalogDocument,
        task_authority: &mut TaskAuthorityDocument,
        updated_at: DateTime<Utc>,
    ) -> Result<PlanningTaskMutationApplication> {
        let mut committed_task_ids = Vec::new();
        let mut changed = false;
        for command in &request.commands {
            match command {
                PlanningTaskMutationCommand::CreateTask(input) => {
                    let task = self.build_unique_task(
                        input,
                        request.source,
                        request.source_turn_id.as_deref(),
                        PlanningTaskAuthorityView {
                            directions,
                            task_authority,
                        },
                        updated_at,
                        None,
                    )?;
                    committed_task_ids.push(task.id.clone());
                    task_authority.tasks.push(task);
                    changed = true;
                }
                PlanningTaskMutationCommand::UpdateTask(input) => {
                    let updated = self.apply_update(
                        input,
                        request.source,
                        request.source_turn_id.as_deref(),
                        directions,
                        task_authority,
                        updated_at,
                    )?;
                    committed_task_ids.push(input.task_id.trim().to_string());
                    changed |= updated;
                }
            }
        }
        Ok(PlanningTaskMutationApplication {
            committed_task_ids,
            changed,
        })
    }

    fn build_unique_task(
        &self,
        input: &PlanningTaskCreateInput,
        source: PlanningTaskMutationSource,
        source_turn_id: Option<&str>,
        authority: PlanningTaskAuthorityView<'_>,
        generated_at: DateTime<Utc>,
        starting_suffix: Option<u32>,
    ) -> Result<TaskDefinition> {
        let mut suffix = starting_suffix;
        for _ in 0..MAX_COLLISION_SUFFIX_ATTEMPTS {
            let task = self.build_task(
                input,
                source,
                source_turn_id,
                authority.directions,
                generated_at,
                suffix,
            )?;
            if !task_id_exists(authority.task_authority, &task.id) {
                return Ok(task);
            }
            suffix = increment_suffix(suffix);
        }
        bail!("planning task mutation could not allocate a unique task id")
    }

    fn build_task(
        &self,
        input: &PlanningTaskCreateInput,
        source: PlanningTaskMutationSource,
        source_turn_id: Option<&str>,
        directions: &DirectionCatalogDocument,
        generated_at: DateTime<Utc>,
        collision_suffix: Option<u32>,
    ) -> Result<TaskDefinition> {
        let title = required_text(&input.title, "task title")?.to_string();
        let description = input
            .description
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or(title.as_str())
            .to_string();
        let direction = select_direction(input.direction_id.as_deref(), directions)?;
        let actor = source.actor();
        let dynamic_priority_delta = input.dynamic_priority_delta.unwrap_or(0);
        let priority_reason = input
            .priority_reason
            .as_deref()
            .map(str::trim)
            .unwrap_or_default()
            .to_string();
        if dynamic_priority_delta != 0 && priority_reason.is_empty() {
            bail!(
                "task `{title}` must include priority_reason when dynamic_priority_delta is non-zero"
            );
        }

        Ok(TaskDefinition {
            id: build_task_id(source, generated_at, &title, collision_suffix),
            direction_id: direction.id.trim().to_string(),
            direction_relation_note: default_relation_note(
                input.direction_relation_note.as_deref(),
                direction,
            ),
            title,
            description,
            status: input.status.unwrap_or(TaskStatus::Ready),
            base_priority: input.base_priority.unwrap_or(DEFAULT_TASK_PRIORITY),
            dynamic_priority_delta,
            priority_reason,
            depends_on: normalize_references(&input.depends_on),
            blocked_by: normalize_references(&input.blocked_by),
            created_by: actor,
            last_updated_by: actor,
            source_turn_id: source_turn_id
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string),
            updated_at: format_timestamp(generated_at),
        })
    }

    fn apply_update(
        &self,
        input: &PlanningTaskUpdateInput,
        source: PlanningTaskMutationSource,
        source_turn_id: Option<&str>,
        directions: &DirectionCatalogDocument,
        task_authority: &mut TaskAuthorityDocument,
        updated_at: DateTime<Utc>,
    ) -> Result<bool> {
        let task_id = required_id(&input.task_id, "task id")?.to_string();
        let task = task_authority
            .tasks
            .iter_mut()
            .find(|task| task.id.trim() == task_id)
            .ok_or_else(|| anyhow!("task `{task_id}` does not exist"))?;
        let previous_task = task.clone();

        if let Some(direction_id) = input.direction_id.as_deref() {
            let direction = find_direction(direction_id, directions)?;
            task.direction_id = direction.id.trim().to_string();
            if input.direction_relation_note.is_none()
                && task.direction_relation_note.trim().is_empty()
            {
                task.direction_relation_note = default_relation_note(None, direction);
            }
        }
        if let Some(direction_relation_note) = input.direction_relation_note.as_deref() {
            task.direction_relation_note = direction_relation_note.trim().to_string();
        }
        if let Some(title) = input.title.as_deref() {
            task.title = required_text(title, "task title")?.to_string();
        }
        if let Some(description) = input.description.as_deref() {
            task.description = required_text(description, "task description")?.to_string();
        }
        if let Some(status) = input.status {
            if terminal_status(task.status) && task.status != status {
                bail!(
                    "task `{}` cannot change from terminal status `{}` to `{}`",
                    task.id.trim(),
                    task.status.label(),
                    status.label()
                );
            }
            task.status = status;
        }
        if let Some(base_priority) = input.base_priority {
            task.base_priority = base_priority;
        }
        if let Some(dynamic_priority_delta) = input.dynamic_priority_delta {
            task.dynamic_priority_delta = dynamic_priority_delta;
        }
        if let Some(priority_reason) = input.priority_reason.as_deref() {
            task.priority_reason = priority_reason.trim().to_string();
        }
        if let Some(depends_on) = input.depends_on.as_ref() {
            task.depends_on = normalize_references(depends_on);
        }
        if let Some(blocked_by) = input.blocked_by.as_ref() {
            task.blocked_by = normalize_references(blocked_by);
        }
        if task.dynamic_priority_delta != 0 && task.priority_reason.trim().is_empty() {
            bail!(
                "task `{}` must include priority_reason when dynamic_priority_delta is non-zero",
                task.id.trim()
            );
        }
        if *task == previous_task {
            return Ok(false);
        }
        task.last_updated_by = source.actor();
        if let Some(source_turn_id) = source_turn_id
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            task.source_turn_id = Some(source_turn_id.to_string());
        }
        task.updated_at = format_timestamp(updated_at);
        Ok(true)
    }

    fn commit_authority(
        &self,
        workspace_directory: &str,
        observed_planning_revision: Option<i64>,
        task_authority: &TaskAuthorityDocument,
        queue_projection: &PriorityQueueProjection,
    ) -> Result<PlanningTaskAuthorityCommitResult> {
        self.planning_task_repository_port
            .commit_task_authority_snapshot(
                workspace_directory,
                PlanningTaskAuthorityCommit {
                    observed_planning_revision,
                    task_authority,
                    queue_projection,
                },
            )
    }
}

#[derive(Debug, Clone)]
struct PlanningTaskMutationContext {
    directions: DirectionCatalogDocument,
    task_authority: TaskAuthorityDocument,
    task_planning_revision: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PlanningTaskMutationApplication {
    committed_task_ids: Vec<String>,
    changed: bool,
}

#[derive(Debug, Clone, Copy)]
struct PlanningTaskAuthorityView<'a> {
    directions: &'a DirectionCatalogDocument,
    task_authority: &'a TaskAuthorityDocument,
}

#[cfg(test)]
mod tests;
