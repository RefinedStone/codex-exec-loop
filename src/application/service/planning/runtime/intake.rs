use std::collections::HashSet;
use std::sync::Arc;

use anyhow::{Context, Result, anyhow};
use chrono::{DateTime, Utc};

use crate::application::port::outbound::planning_task_repository_port::PlanningTaskRepositoryPort;
use crate::application::port::outbound::planning_workspace_port::{
    PlanningWorkspaceLoadRecord, PlanningWorkspacePort,
};
use crate::application::service::planning::runtime::validation::PlanningValidationService;
use crate::application::service::planning::shared::authority_seed::PlanningAuthoritySeedService;
use crate::application::service::planning::shared::contract::RESULT_OUTPUT_FILE_PATH;
use crate::application::service::planning::task_mutation::{
    PlanningTaskCreateInput, PlanningTaskCreatePreview, PlanningTaskCreatePreviewRequest,
    PlanningTaskMutationService, PlanningTaskMutationSource,
};
use crate::domain::planning::PriorityQueueService;
use crate::domain::planning::{
    DirectionCatalogDocument, DirectionState, PLANNING_FORMAT_VERSION, PlanningWorkspaceFiles,
    PriorityQueueTask, TaskAuthorityDocument, TaskDefinition,
};

mod draft;

use self::draft::normalize_prompt;
pub use self::draft::{
    LocalPromptTaskDraftGenerator, PlanningTaskDraftGenerator, PlanningTaskIntakeGenerationRequest,
};

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
    pub mutation_preview: PlanningTaskCreatePreview,
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
                        "Task direction `{}` is not in direction authority.",
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
    authority_seed_service: PlanningAuthoritySeedService,
    mutation_service: PlanningTaskMutationService,
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
        let mutation_service = PlanningTaskMutationService::new(
            planning_task_repository_port.clone(),
            priority_queue_service.clone(),
        );
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
            mutation_service,
            draft_generator,
        }
    }

    pub fn prepare_task_intake(
        &self,
        request: PlanningTaskIntakeRequest,
    ) -> Result<PlanningTaskIntakeProposal> {
        if normalize_prompt(&request.raw_prompt).is_empty() {
            return Err(PlanningTaskIntakeValidationError::new(
                "blank_prompt",
                "Type a task prompt before previewing runtime intake.",
            )
            .into_anyhow());
        }
        let context = self.load_intake_context(&request)?;
        let generated_at = Utc::now();
        let generated_draft =
            self.draft_generator
                .generate(&PlanningTaskIntakeGenerationRequest {
                    request: &request,
                    directions: &context.directions,
                    generated_at,
                    collision_suffix: None,
                })?;
        let mutation_preview = self.mutation_service.preview_create_task_with_authority(
            PlanningTaskCreatePreviewRequest {
                workspace_directory: request.workspace_directory.clone(),
                source: PlanningTaskMutationSource::User,
                source_turn_id: request.active_turn_id.clone(),
                input: create_input_from_draft(&generated_draft),
            },
            &context.directions,
            &context.task_authority,
            context.task_planning_revision,
        )?;
        let draft = draft_from_mutation_preview(&request, &mutation_preview);
        Ok(PlanningTaskIntakeProposal {
            preview_lines: build_preview_lines(&draft),
            warnings: Vec::new(),
            observed_planning_revision: mutation_preview.observed_planning_revision,
            request,
            draft,
            mutation_preview,
        })
    }

    pub fn commit_task_intake(
        &self,
        proposal: &PlanningTaskIntakeProposal,
    ) -> Result<PlanningTaskIntakeCommitResult> {
        let result = self
            .mutation_service
            .commit_create_preview(&proposal.mutation_preview)?;
        Ok(PlanningTaskIntakeCommitResult {
            committed_task_id: result
                .committed_task_ids
                .first()
                .cloned()
                .unwrap_or_else(|| proposal.draft.task.id.clone()),
            committed_planning_revision: result.committed_planning_revision,
            queue_head: result.queue_head,
            task_authority_committed: result.task_authority_changed,
        })
    }

    fn load_intake_context(
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

        let result_output_markdown = required_workspace_body(
            &workspace_record,
            RESULT_OUTPUT_FILE_PATH,
            workspace_record.result_output_markdown.as_deref(),
        )?;
        let direction_snapshot = self
            .planning_task_repository_port
            .load_direction_authority_snapshot(&request.workspace_directory)?
            .ok_or_else(|| {
                anyhow!(
                    "Planning direction authority is unavailable; initialize or repair the planning database before using :task."
                )
            })?;
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
                    directions: &direction_snapshot.directions,
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
        Ok(PlanningTaskIntakeContext {
            directions,
            task_authority,
            task_planning_revision: repository_snapshot.planning_revision,
        })
    }
}

#[derive(Debug, Clone)]
struct PlanningTaskIntakeContext {
    directions: DirectionCatalogDocument,
    task_authority: TaskAuthorityDocument,
    task_planning_revision: i64,
}

fn required_workspace_body<'a>(
    _workspace_record: &'a PlanningWorkspaceLoadRecord,
    path: &'static str,
    body: Option<&'a str>,
) -> Result<&'a str> {
    body.ok_or_else(|| {
        anyhow!(
            "Planning workspace is incomplete: missing {path}. Run :doctor to inspect the workspace, then use :init or admin controls to restore planning files."
        )
    })
}

fn task_intake_repair_guidance(first_failure: &str) -> &'static str {
    if first_failure.contains("references unknown direction_id") {
        return "Next action: run :doctor to inspect direction authority.";
    }
    if first_failure.contains("DB task authority")
        || first_failure.contains("task ")
        || first_failure.contains("task-authority")
    {
        return "Next action: run :doctor to inspect task authority.";
    }
    if first_failure.contains("direction ") || first_failure.contains("queue_idle") {
        return "Next action: run :doctor to inspect direction authority.";
    }
    "Next action: run :doctor to inspect the workspace."
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

fn create_input_from_draft(draft: &PlanningTaskIntakeDraft) -> PlanningTaskCreateInput {
    PlanningTaskCreateInput {
        direction_id: Some(draft.task.direction_id.clone()),
        direction_relation_note: Some(draft.task.direction_relation_note.clone()),
        title: draft.task.title.clone(),
        description: Some(draft.task.description.clone()),
        status: Some(draft.task.status),
        base_priority: Some(draft.task.base_priority),
        dynamic_priority_delta: Some(draft.task.dynamic_priority_delta),
        priority_reason: Some(draft.task.priority_reason.clone()),
        depends_on: draft.task.depends_on.clone(),
        blocked_by: draft.task.blocked_by.clone(),
    }
}

fn draft_from_mutation_preview(
    request: &PlanningTaskIntakeRequest,
    preview: &PlanningTaskCreatePreview,
) -> PlanningTaskIntakeDraft {
    PlanningTaskIntakeDraft {
        task: preview.task.clone(),
        direction_title: preview.direction_title.clone(),
        normalized_prompt: normalize_prompt(&request.raw_prompt),
        generated_at: preview.generated_at,
        collision_suffix: preview.collision_suffix,
    }
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
pub(super) mod tests {
    use chrono::{TimeZone, Utc};

    use super::{
        LocalPromptTaskDraftGenerator, PlanningTaskDraftGenerator,
        PlanningTaskIntakeGenerationRequest, PlanningTaskIntakeRequest,
        PlanningTaskIntakeValidationService,
    };
    use crate::domain::planning::{
        DirectionCatalogDocument, DirectionDefinition, DirectionState, QueueIdleConfig,
        TaskAuthorityDocument,
    };

    pub(super) fn directions() -> DirectionCatalogDocument {
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

    pub(super) fn request(prompt: &str) -> PlanningTaskIntakeRequest {
        PlanningTaskIntakeRequest {
            workspace_directory: "/tmp/workspace".to_string(),
            raw_prompt: prompt.to_string(),
            active_turn_id: Some("turn-1".to_string()),
            requested_direction_id: None,
            observed_planning_revision: None,
        }
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
}
