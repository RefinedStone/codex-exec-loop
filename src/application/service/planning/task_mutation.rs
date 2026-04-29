use std::collections::{BTreeSet, HashSet};
use std::sync::Arc;

use anyhow::{Context, Result, anyhow, bail};
use chrono::{DateTime, SecondsFormat, Utc};
use serde::Deserialize;

use crate::application::port::outbound::planning_task_repository_port::{
    PlanningTaskAuthorityCommit, PlanningTaskAuthorityCommitResult, PlanningTaskRepositoryPort,
};
use crate::domain::planning::{
    DirectionCatalogDocument, DirectionDefinition, DirectionState, PLANNING_FORMAT_VERSION,
    PlanningFileKind, PlanningSemanticValidationService, PlanningValidationReport,
    PriorityQueueProjection, PriorityQueueService, PriorityQueueTask, TaskActor,
    TaskAuthorityDocument, TaskDefinition, TaskStatus,
};

const DEFAULT_TASK_PRIORITY: i32 = 80;
const MAX_REVISION_CONFLICT_RETRIES: usize = 3;
const MAX_COLLISION_SUFFIX_ATTEMPTS: u32 = 20;
const TASK_ID_HASH_CHARS: usize = 12;

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
pub struct PlanningTaskCreateInput {
    pub direction_id: Option<String>,
    pub direction_relation_note: Option<String>,
    pub title: String,
    pub description: Option<String>,
    pub status: Option<TaskStatus>,
    pub base_priority: Option<i32>,
    pub dynamic_priority_delta: Option<i32>,
    pub priority_reason: Option<String>,
    pub depends_on: Vec<String>,
    pub blocked_by: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanningTaskUpdateInput {
    pub task_id: String,
    pub direction_id: Option<String>,
    pub direction_relation_note: Option<String>,
    pub title: Option<String>,
    pub description: Option<String>,
    pub status: Option<TaskStatus>,
    pub base_priority: Option<i32>,
    pub dynamic_priority_delta: Option<i32>,
    pub priority_reason: Option<String>,
    pub depends_on: Option<Vec<String>>,
    pub blocked_by: Option<Vec<String>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlanningTaskMutationCommand {
    CreateTask(PlanningTaskCreateInput),
    UpdateTask(PlanningTaskUpdateInput),
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlanningTaskCommandExtraction {
    Commands(Vec<PlanningTaskMutationCommand>),
    LegacyTaskAuthorityRejected(String),
    InvalidCommands(String),
    None,
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
        let generated_at = Utc::now();
        let task = self.build_unique_task(
            &request.input,
            request.source,
            request.source_turn_id.as_deref(),
            &context,
            generated_at,
            None,
        )?;
        let direction_title = direction_title(&context.directions, &task.direction_id)
            .unwrap_or_else(|| task.direction_id.clone());
        let mut next_task_authority = context.task_authority.clone();
        next_task_authority.tasks.push(task.clone());
        let queue_projection =
            self.validate_and_project(&context.directions, &next_task_authority)?;

        Ok(PlanningTaskCreatePreview {
            request,
            task,
            direction_title,
            generated_at,
            collision_suffix: None,
            observed_planning_revision: context.task_planning_revision,
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
                    &context,
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
            let committed_task_ids = self.apply_commands_to_authority(
                &request,
                &context.directions,
                &mut next_task_authority,
                Utc::now(),
            )?;
            let queue_projection =
                self.validate_and_project(&context.directions, &next_task_authority)?;

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
                        committed_task_ids,
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
    ) -> Result<Vec<String>> {
        let mut committed_task_ids = Vec::new();
        for command in &request.commands {
            match command {
                PlanningTaskMutationCommand::CreateTask(input) => {
                    let task = self.build_unique_task(
                        input,
                        request.source,
                        request.source_turn_id.as_deref(),
                        &PlanningTaskMutationContext {
                            directions: directions.clone(),
                            task_authority: task_authority.clone(),
                            task_planning_revision: 0,
                        },
                        updated_at,
                        None,
                    )?;
                    committed_task_ids.push(task.id.clone());
                    task_authority.tasks.push(task);
                }
                PlanningTaskMutationCommand::UpdateTask(input) => {
                    self.apply_update(
                        input,
                        request.source,
                        request.source_turn_id.as_deref(),
                        directions,
                        task_authority,
                        updated_at,
                    )?;
                    committed_task_ids.push(input.task_id.trim().to_string());
                }
            }
        }
        Ok(committed_task_ids)
    }

    fn build_unique_task(
        &self,
        input: &PlanningTaskCreateInput,
        source: PlanningTaskMutationSource,
        source_turn_id: Option<&str>,
        context: &PlanningTaskMutationContext,
        generated_at: DateTime<Utc>,
        starting_suffix: Option<u32>,
    ) -> Result<TaskDefinition> {
        let mut suffix = starting_suffix;
        for _ in 0..MAX_COLLISION_SUFFIX_ATTEMPTS {
            let task =
                self.build_task(input, source, source_turn_id, context, generated_at, suffix)?;
            if !task_id_exists(&context.task_authority, &task.id) {
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
        context: &PlanningTaskMutationContext,
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
        let direction = select_direction(input.direction_id.as_deref(), &context.directions)?;
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
    ) -> Result<()> {
        let task_id = required_id(&input.task_id, "task id")?.to_string();
        let task = task_authority
            .tasks
            .iter_mut()
            .find(|task| task.id.trim() == task_id)
            .ok_or_else(|| anyhow!("task `{task_id}` does not exist"))?;

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
            if terminal_status(task.status) && task.status != status && !terminal_status(status) {
                bail!(
                    "task `{}` cannot regress from terminal status `{}` to `{}`",
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
        task.last_updated_by = source.actor();
        if let Some(source_turn_id) = source_turn_id
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            task.source_turn_id = Some(source_turn_id.to_string());
        }
        task.updated_at = format_timestamp(updated_at);
        Ok(())
    }

    fn validate_and_project(
        &self,
        directions: &DirectionCatalogDocument,
        task_authority: &TaskAuthorityDocument,
    ) -> Result<PriorityQueueProjection> {
        let mut report = PlanningValidationReport::new();
        PlanningSemanticValidationService::new().validate(
            Some(directions),
            Some(task_authority),
            &mut report,
        );
        reject_task_validation_errors(&report)?;
        self.validate_task_links(task_authority)?;
        validate_priorities(task_authority)?;
        self.priority_queue_service
            .build_projection(directions, task_authority)
            .context("failed to rebuild planning queue projection")
    }

    fn validate_task_links(&self, task_authority: &TaskAuthorityDocument) -> Result<()> {
        let task_ids = task_authority
            .tasks
            .iter()
            .map(|task| task.id.trim().to_string())
            .collect::<HashSet<_>>();
        for task in &task_authority.tasks {
            let task_id = task.id.trim();
            for dependency_id in &task.depends_on {
                validate_task_reference("dependency", task_id, dependency_id, &task_ids)?;
            }
            for blocker_id in &task.blocked_by {
                validate_task_reference("blocker", task_id, blocker_id, &task_ids)?;
            }
        }
        Ok(())
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

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct PlanningTaskCommandsDocument {
    planning_task_commands: PlanningTaskCommandsEnvelope,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct PlanningTaskCommandsEnvelope {
    version: u32,
    #[serde(default)]
    commands: Vec<PlanningTaskCommandEnvelope>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
enum PlanningTaskCommandEnvelope {
    CreateTask(PlanningTaskCreateCommand),
    UpdateTask(PlanningTaskUpdateCommand),
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct PlanningTaskCreateCommand {
    direction_id: Option<String>,
    direction_relation_note: Option<String>,
    title: String,
    description: Option<String>,
    status: Option<TaskStatus>,
    base_priority: Option<i32>,
    dynamic_priority_delta: Option<i32>,
    priority_reason: Option<String>,
    #[serde(default)]
    depends_on: Vec<String>,
    #[serde(default)]
    blocked_by: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct PlanningTaskUpdateCommand {
    task_id: String,
    direction_id: Option<String>,
    direction_relation_note: Option<String>,
    title: Option<String>,
    description: Option<String>,
    status: Option<TaskStatus>,
    base_priority: Option<i32>,
    dynamic_priority_delta: Option<i32>,
    priority_reason: Option<String>,
    depends_on: Option<Vec<String>>,
    blocked_by: Option<Vec<String>>,
}

pub fn extract_planning_task_commands(message: &str) -> PlanningTaskCommandExtraction {
    let mut first_invalid = None;
    for candidate in candidate_json_sections(message) {
        if candidate.trim().is_empty() {
            continue;
        }
        let Ok(value) = serde_json::from_str::<serde_json::Value>(candidate) else {
            continue;
        };
        if value.get("task_authority").is_some()
            || (value.get("version").is_some() && value.get("tasks").is_some())
        {
            let rejected =
                serde_json::to_string_pretty(&value).unwrap_or_else(|_| candidate.to_string());
            return PlanningTaskCommandExtraction::LegacyTaskAuthorityRejected(rejected);
        }
        if value.get("planning_task_commands").is_some() {
            match serde_json::from_value::<PlanningTaskCommandsDocument>(value) {
                Ok(document) => {
                    if document.planning_task_commands.version != 1 {
                        return PlanningTaskCommandExtraction::InvalidCommands(format!(
                            "planning_task_commands version {} is not supported",
                            document.planning_task_commands.version
                        ));
                    }
                    return PlanningTaskCommandExtraction::Commands(
                        document
                            .planning_task_commands
                            .commands
                            .into_iter()
                            .map(PlanningTaskMutationCommand::from)
                            .collect(),
                    );
                }
                Err(error) => first_invalid = Some(error.to_string()),
            }
        }
    }

    first_invalid
        .map(PlanningTaskCommandExtraction::InvalidCommands)
        .unwrap_or(PlanningTaskCommandExtraction::None)
}

impl From<PlanningTaskCommandEnvelope> for PlanningTaskMutationCommand {
    fn from(command: PlanningTaskCommandEnvelope) -> Self {
        match command {
            PlanningTaskCommandEnvelope::CreateTask(command) => {
                Self::CreateTask(PlanningTaskCreateInput {
                    direction_id: command.direction_id,
                    direction_relation_note: command.direction_relation_note,
                    title: command.title,
                    description: command.description,
                    status: command.status,
                    base_priority: command.base_priority,
                    dynamic_priority_delta: command.dynamic_priority_delta,
                    priority_reason: command.priority_reason,
                    depends_on: command.depends_on,
                    blocked_by: command.blocked_by,
                })
            }
            PlanningTaskCommandEnvelope::UpdateTask(command) => {
                Self::UpdateTask(PlanningTaskUpdateInput {
                    task_id: command.task_id,
                    direction_id: command.direction_id,
                    direction_relation_note: command.direction_relation_note,
                    title: command.title,
                    description: command.description,
                    status: command.status,
                    base_priority: command.base_priority,
                    dynamic_priority_delta: command.dynamic_priority_delta,
                    priority_reason: command.priority_reason,
                    depends_on: command.depends_on,
                    blocked_by: command.blocked_by,
                })
            }
        }
    }
}

fn select_direction<'a>(
    requested_direction_id: Option<&str>,
    directions: &'a DirectionCatalogDocument,
) -> Result<&'a DirectionDefinition> {
    if let Some(requested_direction_id) = requested_direction_id
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        let direction = find_direction(requested_direction_id, directions)?;
        if direction.state != DirectionState::Active {
            bail!(
                "direction `{}` is not active; task mutations can only create tasks for active directions",
                direction.id.trim()
            );
        }
        return Ok(direction);
    }

    if let Some(direction) = directions.directions.iter().find(|direction| {
        direction.id.trim() == "general-workstream" && direction.state == DirectionState::Active
    }) {
        return Ok(direction);
    }

    directions
        .directions
        .iter()
        .find(|direction| direction.state == DirectionState::Active)
        .ok_or_else(|| anyhow!("task mutation requires an active planning direction"))
}

fn find_direction<'a>(
    direction_id: &str,
    directions: &'a DirectionCatalogDocument,
) -> Result<&'a DirectionDefinition> {
    let direction_id = required_id(direction_id, "direction id")?;
    directions
        .directions
        .iter()
        .find(|direction| direction.id.trim() == direction_id)
        .ok_or_else(|| anyhow!("direction `{direction_id}` does not exist"))
}

fn direction_title(directions: &DirectionCatalogDocument, direction_id: &str) -> Option<String> {
    directions
        .directions
        .iter()
        .find(|direction| direction.id.trim() == direction_id.trim())
        .map(|direction| direction.title.trim().to_string())
}

fn default_relation_note(raw_note: Option<&str>, direction: &DirectionDefinition) -> String {
    raw_note
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| {
            format!(
                "Task supports direction `{}`: {}",
                direction.id.trim(),
                direction.summary.trim()
            )
        })
}

fn validate_task_reference(
    link_kind: &'static str,
    task_id: &str,
    target_task_id: &str,
    task_ids: &HashSet<String>,
) -> Result<()> {
    let normalized = target_task_id.trim();
    if normalized.is_empty() {
        bail!("task `{task_id}` contains a blank {link_kind}");
    }
    if normalized == task_id {
        bail!("task `{task_id}` cannot reference itself as a {link_kind}");
    }
    if !task_ids.contains(normalized) {
        bail!("task `{task_id}` references unknown {link_kind} `{normalized}`");
    }
    Ok(())
}

fn validate_priorities(task_authority: &TaskAuthorityDocument) -> Result<()> {
    for task in &task_authority.tasks {
        if !(0..=100).contains(&task.base_priority) {
            bail!(
                "task `{}` base_priority must be within 0..100",
                task.id.trim()
            );
        }
        if !(-100..=100).contains(&task.dynamic_priority_delta) {
            bail!(
                "task `{}` dynamic_priority_delta must be within -100..100",
                task.id.trim()
            );
        }
        if !(0..=100).contains(&task.combined_priority()) {
            bail!(
                "task `{}` combined priority must stay within 0..100",
                task.id.trim()
            );
        }
    }
    Ok(())
}

fn reject_task_validation_errors(report: &PlanningValidationReport) -> Result<()> {
    let errors = report
        .errors()
        .into_iter()
        .filter(|issue| issue.file_kind == PlanningFileKind::TaskAuthority)
        .map(|issue| issue.message.as_str())
        .collect::<Vec<_>>();
    if errors.is_empty() {
        return Ok(());
    }
    bail!(
        "planning task mutation failed validation: {}",
        errors.join("; ")
    )
}

fn build_task_id(
    source: PlanningTaskMutationSource,
    generated_at: DateTime<Utc>,
    title: &str,
    collision_suffix: Option<u32>,
) -> String {
    let timestamp = generated_at.format("%Y%m%dT%H%M%SZ");
    let base = format!(
        "task-{}-{timestamp}-{}",
        source.id_slug(),
        stable_short_hash(title)
    );
    match collision_suffix {
        Some(suffix) => format!("{base}-{suffix}"),
        None => base,
    }
}

fn stable_short_hash(value: &str) -> String {
    const FNV_OFFSET: u64 = 0xcbf29ce484222325;
    const FNV_PRIME: u64 = 0x100000001b3;

    let mut hash = FNV_OFFSET;
    for byte in value.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(FNV_PRIME);
    }

    format!("{hash:016x}")[..TASK_ID_HASH_CHARS].to_string()
}

fn increment_suffix(suffix: Option<u32>) -> Option<u32> {
    Some(suffix.unwrap_or(0) + 1)
}

fn task_id_exists(task_authority: &TaskAuthorityDocument, task_id: &str) -> bool {
    task_authority
        .tasks
        .iter()
        .any(|task| task.id.trim() == task_id.trim())
}

fn required_id<'a>(value: &'a str, label: &str) -> Result<&'a str> {
    let value = value.trim();
    if value.is_empty() {
        bail!("{label} is required");
    }
    if value.contains(char::is_whitespace) || value.contains('/') || value.contains('\\') {
        bail!("{label} `{value}` must not contain whitespace or path separators");
    }
    Ok(value)
}

fn required_text<'a>(value: &'a str, label: &str) -> Result<&'a str> {
    let value = value.trim();
    if value.is_empty() {
        bail!("{label} is required");
    }
    Ok(value)
}

fn normalize_references(values: &[String]) -> Vec<String> {
    values
        .iter()
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

fn terminal_status(status: TaskStatus) -> bool {
    matches!(status, TaskStatus::Done | TaskStatus::Cancelled)
}

fn format_timestamp(timestamp: DateTime<Utc>) -> String {
    timestamp.to_rfc3339_opts(SecondsFormat::Secs, true)
}

fn candidate_json_sections(message: &str) -> Vec<&str> {
    let mut sections = Vec::new();
    let mut remainder = message;
    while let Some(start) = remainder.find("```") {
        remainder = &remainder[start + 3..];
        let body_start = remainder.find('\n').map(|index| index + 1).unwrap_or(0);
        let after_header = &remainder[body_start..];
        let Some(end) = after_header.find("```") else {
            break;
        };
        sections.push(after_header[..end].trim());
        remainder = &after_header[end + 3..];
    }
    sections.push(message.trim());
    sections
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::{
        PlanningTaskCommandExtraction, PlanningTaskCreateInput, PlanningTaskCreatePreviewRequest,
        PlanningTaskMutationCommand, PlanningTaskMutationRequest, PlanningTaskMutationService,
        PlanningTaskMutationSource, PlanningTaskUpdateInput, extract_planning_task_commands,
    };
    use crate::application::port::outbound::planning_task_repository_port::{
        NoopPlanningTaskRepositoryPort, PlanningDirectionAuthorityCommit,
        PlanningTaskAuthorityCommit, PlanningTaskRepositoryPort,
    };
    use crate::domain::planning::{
        DirectionCatalogDocument, DirectionDefinition, DirectionState, PLANNING_FORMAT_VERSION,
        PriorityQueueProjection, QueueIdleConfig, TaskActor, TaskAuthorityDocument, TaskDefinition,
        TaskStatus,
    };

    fn workspace(label: &str) -> String {
        format!(
            "/tmp/akra-planning-task-mutation-test-{label}-{}",
            std::process::id()
        )
    }

    fn repo() -> Arc<NoopPlanningTaskRepositoryPort> {
        Arc::new(NoopPlanningTaskRepositoryPort)
    }

    fn directions() -> DirectionCatalogDocument {
        DirectionCatalogDocument {
            version: PLANNING_FORMAT_VERSION,
            queue_idle: QueueIdleConfig::default(),
            directions: vec![DirectionDefinition {
                id: "general-workstream".to_string(),
                title: "General".to_string(),
                summary: "Handle general planning work.".to_string(),
                success_criteria: vec!["done".to_string()],
                scope_hints: Vec::new(),
                detail_doc_path: String::new(),
                state: DirectionState::Active,
            }],
        }
    }

    fn task(id: &str, status: TaskStatus) -> TaskDefinition {
        TaskDefinition {
            id: id.to_string(),
            direction_id: "general-workstream".to_string(),
            direction_relation_note: "supports direction".to_string(),
            title: "Existing task".to_string(),
            description: "Existing task".to_string(),
            status,
            base_priority: 10,
            dynamic_priority_delta: 0,
            priority_reason: String::new(),
            depends_on: Vec::new(),
            blocked_by: Vec::new(),
            created_by: TaskActor::User,
            last_updated_by: TaskActor::User,
            source_turn_id: None,
            updated_at: "2026-04-29T00:00:00Z".to_string(),
        }
    }

    fn seed(
        repo: &NoopPlanningTaskRepositoryPort,
        workspace: &str,
        task_authority: TaskAuthorityDocument,
    ) {
        repo.clear_direction_authority_snapshot(workspace).unwrap();
        repo.clear_task_authority_snapshot(workspace).unwrap();
        repo.commit_direction_authority_snapshot(
            workspace,
            PlanningDirectionAuthorityCommit {
                observed_planning_revision: None,
                directions: &directions(),
            },
        )
        .unwrap();
        repo.commit_task_authority_snapshot(
            workspace,
            PlanningTaskAuthorityCommit {
                observed_planning_revision: None,
                task_authority: &task_authority,
                queue_projection: &PriorityQueueProjection {
                    next_task: None,
                    active_tasks: Vec::new(),
                    proposed_tasks: Vec::new(),
                    skipped_tasks: Vec::new(),
                },
            },
        )
        .unwrap();
    }

    #[test]
    fn user_preview_and_llm_create_share_defaults_and_audit() {
        let repo = repo();
        let workspace = workspace("shared-defaults");
        seed(
            repo.as_ref(),
            &workspace,
            TaskAuthorityDocument {
                version: PLANNING_FORMAT_VERSION,
                tasks: Vec::new(),
            },
        );
        let service = PlanningTaskMutationService::new(
            repo.clone(),
            crate::domain::planning::PriorityQueueService::new(),
        );

        let preview = service
            .preview_create_task(PlanningTaskCreatePreviewRequest {
                workspace_directory: workspace.clone(),
                source: PlanningTaskMutationSource::User,
                source_turn_id: Some("turn-user".to_string()),
                input: PlanningTaskCreateInput {
                    direction_id: None,
                    direction_relation_note: None,
                    title: "Create from task command".to_string(),
                    description: Some("Create from task command".to_string()),
                    status: None,
                    base_priority: None,
                    dynamic_priority_delta: None,
                    priority_reason: None,
                    depends_on: Vec::new(),
                    blocked_by: Vec::new(),
                },
            })
            .unwrap();
        assert_eq!(preview.task.status, TaskStatus::Ready);
        assert_eq!(preview.task.base_priority, 80);
        assert_eq!(preview.task.created_by, TaskActor::User);
        assert_eq!(preview.task.last_updated_by, TaskActor::User);

        let result = service
            .apply_commands(PlanningTaskMutationRequest {
                workspace_directory: workspace.clone(),
                source: PlanningTaskMutationSource::Llm,
                source_turn_id: Some("turn-llm".to_string()),
                commands: vec![PlanningTaskMutationCommand::CreateTask(
                    PlanningTaskCreateInput {
                        direction_id: None,
                        direction_relation_note: None,
                        title: "Create from worker command".to_string(),
                        description: None,
                        status: None,
                        base_priority: None,
                        dynamic_priority_delta: None,
                        priority_reason: None,
                        depends_on: Vec::new(),
                        blocked_by: Vec::new(),
                    },
                )],
            })
            .unwrap();
        assert!(result.task_authority_changed);
        let snapshot = repo
            .load_task_authority_snapshot(&workspace)
            .unwrap()
            .unwrap();
        let llm_task = snapshot
            .task_authority
            .tasks
            .iter()
            .find(|task| task.title == "Create from worker command")
            .unwrap();
        assert_eq!(llm_task.status, TaskStatus::Ready);
        assert_eq!(llm_task.base_priority, 80);
        assert_eq!(llm_task.created_by, TaskActor::Llm);
        assert_eq!(llm_task.last_updated_by, TaskActor::Llm);
        assert_eq!(llm_task.source_turn_id.as_deref(), Some("turn-llm"));
    }

    #[test]
    fn update_preserves_unspecified_fields() {
        let repo = repo();
        let workspace = workspace("preserve-update");
        seed(
            repo.as_ref(),
            &workspace,
            TaskAuthorityDocument {
                version: PLANNING_FORMAT_VERSION,
                tasks: vec![task("task-1", TaskStatus::Ready)],
            },
        );
        let service = PlanningTaskMutationService::new(
            repo.clone(),
            crate::domain::planning::PriorityQueueService::new(),
        );

        service
            .apply_commands(PlanningTaskMutationRequest {
                workspace_directory: workspace.clone(),
                source: PlanningTaskMutationSource::Llm,
                source_turn_id: Some("turn-2".to_string()),
                commands: vec![PlanningTaskMutationCommand::UpdateTask(
                    PlanningTaskUpdateInput {
                        task_id: "task-1".to_string(),
                        direction_id: None,
                        direction_relation_note: None,
                        title: Some("Updated title".to_string()),
                        description: None,
                        status: Some(TaskStatus::Blocked),
                        base_priority: None,
                        dynamic_priority_delta: None,
                        priority_reason: None,
                        depends_on: None,
                        blocked_by: None,
                    },
                )],
            })
            .unwrap();

        let snapshot = repo
            .load_task_authority_snapshot(&workspace)
            .unwrap()
            .unwrap();
        let updated = &snapshot.task_authority.tasks[0];
        assert_eq!(updated.title, "Updated title");
        assert_eq!(updated.description, "Existing task");
        assert_eq!(updated.status, TaskStatus::Blocked);
        assert_eq!(updated.created_by, TaskActor::User);
        assert_eq!(updated.last_updated_by, TaskActor::Llm);
        assert_eq!(updated.source_turn_id.as_deref(), Some("turn-2"));
    }

    #[test]
    fn legacy_task_authority_is_rejected_by_extractor() {
        let message = r#"```json
{"task_authority":{"version":1,"tasks":[]}}
```"#;

        assert!(matches!(
            extract_planning_task_commands(message),
            PlanningTaskCommandExtraction::LegacyTaskAuthorityRejected(_)
        ));
    }

    #[test]
    fn unknown_command_fields_and_delete_ops_are_invalid() {
        let unknown_field = r#"{"planning_task_commands":{"version":1,"commands":[{"op":"create_task","title":"x","id":"forbidden"}]}}"#;
        let delete_op = r#"{"planning_task_commands":{"version":1,"commands":[{"op":"delete_task","task_id":"task-1"}]}}"#;

        assert!(matches!(
            extract_planning_task_commands(unknown_field),
            PlanningTaskCommandExtraction::InvalidCommands(_)
        ));
        assert!(matches!(
            extract_planning_task_commands(delete_op),
            PlanningTaskCommandExtraction::InvalidCommands(_)
        ));
    }

    #[test]
    fn terminal_status_regression_is_rejected() {
        let repo = repo();
        let workspace = workspace("terminal-regression");
        seed(
            repo.as_ref(),
            &workspace,
            TaskAuthorityDocument {
                version: PLANNING_FORMAT_VERSION,
                tasks: vec![task("task-1", TaskStatus::Done)],
            },
        );
        let service = PlanningTaskMutationService::new(
            repo,
            crate::domain::planning::PriorityQueueService::new(),
        );

        let error = service
            .apply_commands(PlanningTaskMutationRequest {
                workspace_directory: workspace,
                source: PlanningTaskMutationSource::Llm,
                source_turn_id: None,
                commands: vec![PlanningTaskMutationCommand::UpdateTask(
                    PlanningTaskUpdateInput {
                        task_id: "task-1".to_string(),
                        direction_id: None,
                        direction_relation_note: None,
                        title: None,
                        description: None,
                        status: Some(TaskStatus::Ready),
                        base_priority: None,
                        dynamic_priority_delta: None,
                        priority_reason: None,
                        depends_on: None,
                        blocked_by: None,
                    },
                )],
            })
            .unwrap_err();

        assert!(error.to_string().contains("cannot regress"));
    }
}
