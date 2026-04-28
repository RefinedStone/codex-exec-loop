use std::collections::HashSet;
use std::sync::Arc;

use anyhow::{Context, Result, anyhow};
use chrono::{DateTime, SecondsFormat, Utc};

use crate::application::port::outbound::planning_task_repository_port::{
    PlanningTaskAuthorityCommit, PlanningTaskAuthorityCommitResult, PlanningTaskRepositoryPort,
};
use crate::application::port::outbound::planning_workspace_port::{
    PlanningWorkspaceLoadRecord, PlanningWorkspacePort,
};
use crate::application::service::planning::runtime::validation::PlanningValidationService;
use crate::application::service::planning::shared::authority_seed::PlanningAuthoritySeedService;
use crate::application::service::planning::shared::contract::{
    DIRECTIONS_FILE_PATH, RESULT_OUTPUT_FILE_PATH,
};
use crate::domain::planning::PriorityQueueService;
use crate::domain::planning::{
    DirectionCatalogDocument, DirectionDefinition, DirectionState, PLANNING_FORMAT_VERSION,
    PlanningWorkspaceFiles, PriorityQueueProjection, PriorityQueueTask, TaskActor,
    TaskAuthorityDocument, TaskDefinition, TaskStatus,
};

const DEFAULT_RUNTIME_TASK_PRIORITY: i32 = 80;
const MAX_COLLISION_SUFFIX_ATTEMPTS: u32 = 20;
const MAX_REVISION_CONFLICT_RETRIES: usize = 3;
const TASK_TITLE_LIMIT: usize = 72;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanningTaskIntakeRequest {
    pub workspace_directory: String,
    pub raw_prompt: String,
    pub active_turn_id: Option<String>,
    pub requested_direction_id: Option<String>,
    pub observed_planning_revision: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanningTaskIntakeDraft {
    pub task: TaskDefinition,
    pub direction_title: String,
    pub normalized_prompt: String,
    pub generated_at: DateTime<Utc>,
    pub collision_suffix: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanningTaskIntakeProposal {
    pub request: PlanningTaskIntakeRequest,
    pub draft: PlanningTaskIntakeDraft,
    pub observed_planning_revision: i64,
    pub preview_lines: Vec<String>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanningTaskIntakeCommitResult {
    pub committed_task_id: String,
    pub committed_planning_revision: i64,
    pub queue_head: Option<PriorityQueueTask>,
    pub task_authority_committed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanningTaskIntakeValidationError {
    pub code: &'static str,
    pub message: String,
}

impl PlanningTaskIntakeValidationError {
    fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }

    fn into_anyhow(self) -> anyhow::Error {
        anyhow!("{}", self.message)
    }
}

pub trait PlanningTaskDraftGenerator: Send + Sync {
    fn generate(
        &self,
        request: &PlanningTaskIntakeGenerationRequest<'_>,
    ) -> Result<PlanningTaskIntakeDraft>;
}

#[derive(Debug, Clone, Copy)]
pub struct PlanningTaskIntakeGenerationRequest<'a> {
    pub request: &'a PlanningTaskIntakeRequest,
    pub directions: &'a DirectionCatalogDocument,
    pub generated_at: DateTime<Utc>,
    pub collision_suffix: Option<u32>,
}

#[derive(Debug, Clone, Default)]
pub struct LocalPromptTaskDraftGenerator;

impl LocalPromptTaskDraftGenerator {
    pub fn new() -> Self {
        Self
    }
}

impl PlanningTaskDraftGenerator for LocalPromptTaskDraftGenerator {
    fn generate(
        &self,
        request: &PlanningTaskIntakeGenerationRequest<'_>,
    ) -> Result<PlanningTaskIntakeDraft> {
        let normalized_prompt = normalize_prompt(&request.request.raw_prompt);
        let direction = select_direction(
            request.request.requested_direction_id.as_deref(),
            request.directions,
        )
        .map_err(PlanningTaskIntakeValidationError::into_anyhow)?;
        let task_id = build_task_id(
            request.generated_at,
            &normalized_prompt,
            request.collision_suffix,
        );
        let updated_at = request
            .generated_at
            .to_rfc3339_opts(SecondsFormat::Secs, true);

        Ok(PlanningTaskIntakeDraft {
            task: TaskDefinition {
                id: task_id,
                direction_id: direction.id.trim().to_string(),
                direction_relation_note: format!(
                    "User runtime intake task for direction {}.",
                    direction.id.trim()
                ),
                title: build_task_title(&normalized_prompt),
                description: format!("User prompt:\n\n{}", request.request.raw_prompt.trim()),
                status: TaskStatus::Ready,
                base_priority: DEFAULT_RUNTIME_TASK_PRIORITY,
                dynamic_priority_delta: 0,
                priority_reason: "User requested this task through runtime intake.".to_string(),
                depends_on: Vec::new(),
                blocked_by: Vec::new(),
                created_by: TaskActor::User,
                last_updated_by: TaskActor::User,
                source_turn_id: request.request.active_turn_id.clone(),
                updated_at,
            },
            direction_title: direction.title.trim().to_string(),
            normalized_prompt,
            generated_at: request.generated_at,
            collision_suffix: request.collision_suffix,
        })
    }
}

#[derive(Clone, Default)]
pub struct PlanningTaskIntakeValidationService;

impl PlanningTaskIntakeValidationService {
    pub fn new() -> Self {
        Self
    }

    pub fn validate_draft(
        &self,
        request: &PlanningTaskIntakeRequest,
        draft: &PlanningTaskIntakeDraft,
        directions: &DirectionCatalogDocument,
        task_authority: &TaskAuthorityDocument,
    ) -> std::result::Result<(), PlanningTaskIntakeValidationError> {
        if normalize_prompt(&request.raw_prompt).is_empty() {
            return Err(PlanningTaskIntakeValidationError::new(
                "blank_prompt",
                "Type a task prompt before previewing runtime intake.",
            ));
        }
        if draft.task.title.trim().is_empty() {
            return Err(PlanningTaskIntakeValidationError::new(
                "blank_title",
                "Generated task title is blank.",
            ));
        }
        if draft.task.description.trim().is_empty() {
            return Err(PlanningTaskIntakeValidationError::new(
                "blank_description",
                "Generated task description is blank.",
            ));
        }

        let direction = directions
            .directions
            .iter()
            .find(|direction| direction.id.trim() == draft.task.direction_id.trim())
            .ok_or_else(|| {
                PlanningTaskIntakeValidationError::new(
                    "unknown_direction",
                    format!(
                        "Task direction `{}` is not in directions.toml.",
                        draft.task.direction_id.trim()
                    ),
                )
            })?;
        if direction.state != DirectionState::Active {
            return Err(PlanningTaskIntakeValidationError::new(
                "inactive_direction",
                format!(
                    "Task direction `{}` is not active; use :directions or :planning first.",
                    direction.id.trim()
                ),
            ));
        }

        let effective_priority = draft.task.combined_priority();
        if !(0..=100).contains(&draft.task.base_priority)
            || !(-100..=100).contains(&draft.task.dynamic_priority_delta)
            || !(0..=100).contains(&effective_priority)
        {
            return Err(PlanningTaskIntakeValidationError::new(
                "invalid_priority",
                "Runtime intake priority must stay within 0..100 after delta.",
            ));
        }

        let existing_task_ids = task_authority
            .tasks
            .iter()
            .map(|task| task.id.trim().to_string())
            .collect::<HashSet<_>>();
        let task_id = draft.task.id.trim();
        if existing_task_ids.contains(task_id) {
            return Err(PlanningTaskIntakeValidationError::new(
                "duplicate_task_id",
                format!("Generated task id `{task_id}` already exists."),
            ));
        }

        for dependency_id in &draft.task.depends_on {
            validate_task_link("dependency", task_id, dependency_id, &existing_task_ids)?;
        }
        for blocker_id in &draft.task.blocked_by {
            validate_task_link("blocker", task_id, blocker_id, &existing_task_ids)?;
        }

        Ok(())
    }
}

#[derive(Clone)]
pub struct PlanningTaskIntakeService {
    planning_workspace_port: Arc<dyn PlanningWorkspacePort>,
    planning_task_repository_port: Arc<dyn PlanningTaskRepositoryPort>,
    planning_validation_service: PlanningValidationService,
    priority_queue_service: PriorityQueueService,
    authority_seed_service: PlanningAuthoritySeedService,
    intake_validation_service: PlanningTaskIntakeValidationService,
    draft_generator: Arc<dyn PlanningTaskDraftGenerator>,
}

impl PlanningTaskIntakeService {
    pub fn new(
        planning_workspace_port: Arc<dyn PlanningWorkspacePort>,
        planning_task_repository_port: Arc<dyn PlanningTaskRepositoryPort>,
        planning_validation_service: PlanningValidationService,
        priority_queue_service: PriorityQueueService,
    ) -> Self {
        Self::with_generator(
            planning_workspace_port,
            planning_task_repository_port,
            planning_validation_service,
            priority_queue_service,
            Arc::new(LocalPromptTaskDraftGenerator::new()),
        )
    }

    pub fn with_generator(
        planning_workspace_port: Arc<dyn PlanningWorkspacePort>,
        planning_task_repository_port: Arc<dyn PlanningTaskRepositoryPort>,
        planning_validation_service: PlanningValidationService,
        priority_queue_service: PriorityQueueService,
        draft_generator: Arc<dyn PlanningTaskDraftGenerator>,
    ) -> Self {
        Self {
            authority_seed_service: PlanningAuthoritySeedService::new(
                planning_workspace_port.clone(),
                planning_task_repository_port.clone(),
                planning_validation_service.clone(),
                priority_queue_service.clone(),
            ),
            planning_workspace_port,
            planning_task_repository_port,
            planning_validation_service,
            priority_queue_service,
            intake_validation_service: PlanningTaskIntakeValidationService::new(),
            draft_generator,
        }
    }

    pub fn prepare_task_intake(
        &self,
        request: PlanningTaskIntakeRequest,
    ) -> Result<PlanningTaskIntakeProposal> {
        let context = self.load_context(&request)?;
        let generated_at = Utc::now();
        let draft = self.generate_valid_draft(&request, &context, generated_at, None)?;
        Ok(PlanningTaskIntakeProposal {
            preview_lines: build_preview_lines(&draft),
            warnings: Vec::new(),
            observed_planning_revision: context.planning_revision,
            request,
            draft,
        })
    }

    pub fn commit_task_intake(
        &self,
        proposal: &PlanningTaskIntakeProposal,
    ) -> Result<PlanningTaskIntakeCommitResult> {
        let mut next_suffix = proposal.draft.collision_suffix;
        let generated_at = proposal.draft.generated_at;
        let mut observed_revision = proposal.observed_planning_revision;

        for _ in 0..=MAX_REVISION_CONFLICT_RETRIES {
            let context = self.load_context(&proposal.request)?;
            let draft = if context.planning_revision == proposal.observed_planning_revision
                && next_suffix == proposal.draft.collision_suffix
            {
                proposal.draft.clone()
            } else {
                self.generate_valid_draft(&proposal.request, &context, generated_at, next_suffix)?
            };
            let (next_task_authority, queue_projection) =
                self.build_accepted_mutation(&proposal.request, &draft, &context)?;

            match self
                .planning_task_repository_port
                .commit_task_authority_snapshot(
                    &proposal.request.workspace_directory,
                    PlanningTaskAuthorityCommit {
                        observed_planning_revision: Some(observed_revision),
                        task_authority: &next_task_authority,
                        queue_projection: &queue_projection,
                    },
                )? {
                PlanningTaskAuthorityCommitResult::Committed { planning_revision } => {
                    return Ok(PlanningTaskIntakeCommitResult {
                        committed_task_id: draft.task.id,
                        committed_planning_revision: planning_revision,
                        queue_head: queue_projection.next_task,
                        task_authority_committed: true,
                    });
                }
                PlanningTaskAuthorityCommitResult::Conflict {
                    current_planning_revision,
                    ..
                } => {
                    observed_revision = current_planning_revision;
                    next_suffix = increment_suffix(next_suffix);
                    continue;
                }
            }
        }

        Err(anyhow!(
            "planning task intake could not commit because planning state kept changing; retry :task"
        ))
    }

    fn load_context(
        &self,
        request: &PlanningTaskIntakeRequest,
    ) -> Result<PlanningTaskIntakeContext> {
        self.authority_seed_service
            .ensure_default_authority(&request.workspace_directory)?;
        let workspace_record = self
            .planning_workspace_port
            .load_planning_workspace_files(&request.workspace_directory)?;
        if !workspace_record.has_any_files() {
            return Err(anyhow!(
                "Planning workspace is unavailable; :task can initialize a new default workspace, but this workspace could not be loaded. Run :doctor for details."
            ));
        }

        let directions_toml = required_workspace_body(
            &workspace_record,
            DIRECTIONS_FILE_PATH,
            workspace_record.directions_toml.as_deref(),
        )?;
        let result_output_markdown = required_workspace_body(
            &workspace_record,
            RESULT_OUTPUT_FILE_PATH,
            workspace_record.result_output_markdown.as_deref(),
        )?;
        let repository_snapshot = self
            .planning_task_repository_port
            .load_task_authority_snapshot(&request.workspace_directory)?
            .ok_or_else(|| {
                anyhow!(
                    "Planning task authority is unavailable; initialize or repair the planning database before using :task."
                )
            })?;
        let task_authority_json = serde_json::to_string_pretty(&repository_snapshot.task_authority)
            .context("failed to serialize task authority ledger")?;

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
            return Err(anyhow!(
                "Planning workspace is invalid; {first_failure}. {}",
                task_intake_repair_guidance(first_failure)
            ));
        }

        let directions = validation_result
            .directions
            .ok_or_else(|| anyhow!("valid planning workspace did not include directions"))?;
        let task_authority = validation_result
            .task_authority
            .ok_or_else(|| anyhow!("valid planning workspace did not include task-authority"))?;
        if task_authority.version != PLANNING_FORMAT_VERSION {
            return Err(anyhow!(
                "Unsupported task-authority version {}; expected {}.",
                task_authority.version,
                PLANNING_FORMAT_VERSION
            ));
        }
        let planning_revision = repository_snapshot.planning_revision;

        Ok(PlanningTaskIntakeContext {
            workspace_record,
            directions,
            task_authority,
            planning_revision,
        })
    }

    fn generate_valid_draft(
        &self,
        request: &PlanningTaskIntakeRequest,
        context: &PlanningTaskIntakeContext,
        generated_at: DateTime<Utc>,
        starting_suffix: Option<u32>,
    ) -> Result<PlanningTaskIntakeDraft> {
        let mut suffix = starting_suffix;
        for _ in 0..MAX_COLLISION_SUFFIX_ATTEMPTS {
            let draft = self
                .draft_generator
                .generate(&PlanningTaskIntakeGenerationRequest {
                    request,
                    directions: &context.directions,
                    generated_at,
                    collision_suffix: suffix,
                })?;
            match self.intake_validation_service.validate_draft(
                request,
                &draft,
                &context.directions,
                &context.task_authority,
            ) {
                Ok(()) => return Ok(draft),
                Err(error) if error.code == "duplicate_task_id" => {
                    suffix = increment_suffix(suffix);
                }
                Err(error) => return Err(error.into_anyhow()),
            }
        }

        Err(anyhow!(
            "Runtime task intake could not allocate a unique task id; retry with a more specific prompt."
        ))
    }

    fn build_accepted_mutation(
        &self,
        request: &PlanningTaskIntakeRequest,
        draft: &PlanningTaskIntakeDraft,
        context: &PlanningTaskIntakeContext,
    ) -> Result<(TaskAuthorityDocument, PriorityQueueProjection)> {
        self.intake_validation_service
            .validate_draft(request, draft, &context.directions, &context.task_authority)
            .map_err(PlanningTaskIntakeValidationError::into_anyhow)?;

        let mut next_task_authority = context.task_authority.clone();
        next_task_authority.tasks.push(draft.task.clone());
        let next_task_authority_json = serde_json::to_string_pretty(&next_task_authority)
            .context("failed to serialize runtime task intake ledger")?;
        let validation_result =
            self.planning_validation_service
                .validate_workspace_files(PlanningWorkspaceFiles {
                    directions_toml: required_workspace_body(
                        &context.workspace_record,
                        DIRECTIONS_FILE_PATH,
                        context.workspace_record.directions_toml.as_deref(),
                    )?,
                    task_authority_json: &next_task_authority_json,
                    result_output_markdown: required_workspace_body(
                        &context.workspace_record,
                        RESULT_OUTPUT_FILE_PATH,
                        context.workspace_record.result_output_markdown.as_deref(),
                    )?,
                });
        if !validation_result.is_valid() {
            return Err(anyhow!(
                "Runtime task intake produced an invalid task ledger: {}",
                validation_result
                    .report
                    .errors()
                    .first()
                    .map(|issue| issue.message.as_str())
                    .unwrap_or("planning validation failed")
            ));
        }
        let queue_projection = self
            .priority_queue_service
            .build_projection(&context.directions, &next_task_authority)
            .map_err(|error| anyhow!("Runtime task intake queue rebuild failed: {error}"))?;

        Ok((next_task_authority, queue_projection))
    }
}

#[derive(Debug, Clone)]
struct PlanningTaskIntakeContext {
    workspace_record: PlanningWorkspaceLoadRecord,
    directions: DirectionCatalogDocument,
    task_authority: TaskAuthorityDocument,
    planning_revision: i64,
}

fn required_workspace_body<'a>(
    _workspace_record: &'a PlanningWorkspaceLoadRecord,
    path: &'static str,
    body: Option<&'a str>,
) -> Result<&'a str> {
    body.ok_or_else(|| {
        anyhow!(
            "Planning workspace is incomplete: missing {path}. Run :doctor to inspect the workspace, then use :directions apply if tracked directions are newer."
        )
    })
}

fn task_intake_repair_guidance(first_failure: &str) -> &'static str {
    if first_failure.contains("references unknown direction_id") {
        return "Next action: run :directions apply if tracked directions.toml contains the missing direction; otherwise run :doctor.";
    }
    if first_failure.contains("DB task authority")
        || first_failure.contains("task ")
        || first_failure.contains("task-authority")
    {
        return "Next action: run :doctor to inspect task authority.";
    }
    if first_failure.contains("directions.toml")
        || first_failure.contains("direction ")
        || first_failure.contains("queue_idle")
    {
        return "Next action: run :directions apply if tracked directions.toml is newer; otherwise run :doctor.";
    }
    "Next action: run :doctor to inspect the workspace, then use :directions apply for tracked direction drift."
}

fn validate_task_link(
    link_kind: &'static str,
    task_id: &str,
    target_task_id: &str,
    existing_task_ids: &HashSet<String>,
) -> std::result::Result<(), PlanningTaskIntakeValidationError> {
    let normalized = target_task_id.trim();
    if normalized.is_empty() {
        return Err(PlanningTaskIntakeValidationError::new(
            "blank_task_link",
            format!("Generated task has a blank {link_kind}."),
        ));
    }
    if normalized == task_id {
        return Err(PlanningTaskIntakeValidationError::new(
            "self_reference",
            format!("Generated task `{task_id}` cannot reference itself as a {link_kind}."),
        ));
    }
    if !existing_task_ids.contains(normalized) {
        return Err(PlanningTaskIntakeValidationError::new(
            "missing_task_link",
            format!("Generated task references unknown {link_kind} `{normalized}`."),
        ));
    }
    Ok(())
}

fn select_direction<'a>(
    requested_direction_id: Option<&str>,
    directions: &'a DirectionCatalogDocument,
) -> std::result::Result<&'a DirectionDefinition, PlanningTaskIntakeValidationError> {
    if let Some(requested_direction_id) = requested_direction_id
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        let direction = directions
            .directions
            .iter()
            .find(|direction| direction.id.trim() == requested_direction_id)
            .ok_or_else(|| {
                PlanningTaskIntakeValidationError::new(
                    "unknown_direction",
                    format!("Requested direction `{requested_direction_id}` does not exist."),
                )
            })?;
        if direction.state != DirectionState::Active {
            return Err(PlanningTaskIntakeValidationError::new(
                "inactive_direction",
                format!(
                    "Requested direction `{requested_direction_id}` is not active; use :directions or :planning first."
                ),
            ));
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
        .ok_or_else(|| {
            PlanningTaskIntakeValidationError::new(
                "no_active_direction",
                "Task intake requires an active planning direction; use :directions or :planning first.",
            )
        })
}

fn normalize_prompt(prompt: &str) -> String {
    prompt.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn build_task_title(normalized_prompt: &str) -> String {
    let mut title = normalized_prompt
        .split(['.', '!', '?'])
        .next()
        .unwrap_or(normalized_prompt)
        .trim()
        .to_string();
    if title.is_empty() {
        title = "User requested runtime task".to_string();
    }
    if title.chars().count() <= TASK_TITLE_LIMIT {
        return title;
    }

    let mut compact = title
        .chars()
        .take(TASK_TITLE_LIMIT.saturating_sub(3))
        .collect::<String>();
    compact.push_str("...");
    compact
}

fn build_task_id(
    generated_at: DateTime<Utc>,
    normalized_prompt: &str,
    collision_suffix: Option<u32>,
) -> String {
    let timestamp = generated_at.format("%Y%m%dT%H%M%SZ");
    let base = format!(
        "task-user-{timestamp}-{}",
        stable_short_hash(normalized_prompt)
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

    format!("{hash:016x}")[..12].to_string()
}

fn increment_suffix(suffix: Option<u32>) -> Option<u32> {
    Some(suffix.unwrap_or(0) + 1)
}

fn build_preview_lines(draft: &PlanningTaskIntakeDraft) -> Vec<String> {
    vec![
        format!("title: {}", draft.task.title.trim()),
        format!(
            "direction: {} ({})",
            draft.direction_title.trim(),
            draft.task.direction_id.trim()
        ),
        format!("status: {}", draft.task.status.label()),
        format!(
            "priority: base {} / delta {}",
            draft.task.base_priority, draft.task.dynamic_priority_delta
        ),
        format!(
            "description: {}",
            draft
                .normalized_prompt
                .chars()
                .take(120)
                .collect::<String>()
        ),
    ]
}

#[cfg(test)]
mod tests {
    use chrono::{TimeZone, Utc};

    use super::{
        LocalPromptTaskDraftGenerator, PlanningTaskDraftGenerator,
        PlanningTaskIntakeGenerationRequest, PlanningTaskIntakeRequest,
        PlanningTaskIntakeValidationService, increment_suffix,
    };
    use crate::domain::planning::{
        DirectionCatalogDocument, DirectionDefinition, DirectionState, QueueIdleConfig, TaskActor,
        TaskAuthorityDocument, TaskStatus,
    };

    fn directions() -> DirectionCatalogDocument {
        DirectionCatalogDocument {
            version: 1,
            queue_idle: QueueIdleConfig::default(),
            directions: vec![
                DirectionDefinition {
                    id: "other-direction".to_string(),
                    title: "Other Direction".to_string(),
                    summary: "secondary".to_string(),
                    success_criteria: vec!["done".to_string()],
                    scope_hints: Vec::new(),
                    detail_doc_path: String::new(),
                    state: DirectionState::Active,
                },
                DirectionDefinition {
                    id: "general-workstream".to_string(),
                    title: "General Workstream".to_string(),
                    summary: "default".to_string(),
                    success_criteria: vec!["done".to_string()],
                    scope_hints: Vec::new(),
                    detail_doc_path: String::new(),
                    state: DirectionState::Active,
                },
            ],
        }
    }

    fn request(prompt: &str) -> PlanningTaskIntakeRequest {
        PlanningTaskIntakeRequest {
            workspace_directory: "/tmp/workspace".to_string(),
            raw_prompt: prompt.to_string(),
            active_turn_id: Some("turn-1".to_string()),
            requested_direction_id: None,
            observed_planning_revision: None,
        }
    }

    #[test]
    fn local_generator_sets_runtime_task_defaults_and_prefers_general_workstream() {
        let request = request("Ship the runtime intake UI\nwith preview");
        let generated_at = Utc.with_ymd_and_hms(2026, 4, 24, 1, 2, 3).unwrap();

        let draft = LocalPromptTaskDraftGenerator::new()
            .generate(&PlanningTaskIntakeGenerationRequest {
                request: &request,
                directions: &directions(),
                generated_at,
                collision_suffix: None,
            })
            .expect("draft should generate");

        assert_eq!(draft.task.direction_id, "general-workstream");
        assert_eq!(draft.task.status, TaskStatus::Ready);
        assert_eq!(draft.task.created_by, TaskActor::User);
        assert_eq!(draft.task.last_updated_by, TaskActor::User);
        assert_eq!(draft.task.base_priority, 80);
        assert_eq!(draft.task.dynamic_priority_delta, 0);
        assert!(draft.task.depends_on.is_empty());
        assert!(draft.task.blocked_by.is_empty());
        assert_eq!(draft.task.source_turn_id.as_deref(), Some("turn-1"));
        assert!(draft.task.id.starts_with("task-user-20260424T010203Z-"));
        assert_eq!(draft.task.title, "Ship the runtime intake UI with preview");
        assert!(
            draft
                .task
                .description
                .contains("Ship the runtime intake UI")
        );
    }

    #[test]
    fn validation_rejects_blank_prompt_duplicate_ids_and_priority_bounds() {
        let directions = directions();
        let existing_request = request("Existing task");
        let generated_at = Utc.with_ymd_and_hms(2026, 4, 24, 1, 2, 3).unwrap();
        let draft = LocalPromptTaskDraftGenerator::new()
            .generate(&PlanningTaskIntakeGenerationRequest {
                request: &existing_request,
                directions: &directions,
                generated_at,
                collision_suffix: None,
            })
            .expect("draft should generate");
        let validation = PlanningTaskIntakeValidationService::new();
        let mut ledger = TaskAuthorityDocument {
            version: 1,
            tasks: vec![draft.task.clone()],
        };

        let duplicate = validation
            .validate_draft(&existing_request, &draft, &directions, &ledger)
            .expect_err("duplicate id should reject");
        assert_eq!(duplicate.code, "duplicate_task_id");

        ledger.tasks.clear();
        let blank = validation
            .validate_draft(&request("   "), &draft, &directions, &ledger)
            .expect_err("blank prompt should reject");
        assert_eq!(blank.code, "blank_prompt");

        let mut invalid_priority = draft.clone();
        invalid_priority.task.base_priority = 101;
        let priority = validation
            .validate_draft(&existing_request, &invalid_priority, &directions, &ledger)
            .expect_err("priority should reject");
        assert_eq!(priority.code, "invalid_priority");
    }

    #[test]
    fn increment_suffix_starts_with_one() {
        assert_eq!(increment_suffix(None), Some(1));
        assert_eq!(increment_suffix(Some(1)), Some(2));
    }
}
