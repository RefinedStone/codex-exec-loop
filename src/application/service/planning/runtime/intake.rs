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
    PriorityQueueTask, TaskAuthorityDocument, TaskDefinition, TaskMutationProvenance,
};

/*
 * runtime intake는 user prompt를 task-authority mutation으로 바꾸는 two-phase flow다. `prepare_task_intake`는
 * validated planning authority를 읽고 draft task를 생성한 뒤 mutation service에 preview를 요청한다.
 * `commit_task_intake`는 나중에 그 exact preview를 commit해, preview 때 관찰한 planning revision이 stale UI
 * confirmation을 막는 guard로 작동하게 한다.
 */

mod draft;

use self::draft::normalize_prompt;
pub use self::draft::{
    LocalPromptTaskDraftGenerator, PlanningTaskDraftGenerator, PlanningTaskIntakeGenerationRequest,
};

#[derive(Debug, Clone, PartialEq, Eq)]
// `:task` 스타일 runtime intake surface에서 들어오는 inbound request다.
pub struct PlanningTaskIntakeRequest {
    pub workspace_directory: String,
    pub raw_prompt: String,
    pub source_turn_id: Option<String>,
    pub provenance: TaskMutationProvenance,
    pub requested_direction_id: Option<String>,
    pub observed_planning_revision: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
// generated task와 preview/commit fallback에 필요한 display context를 함께 보관한다.
pub struct PlanningTaskIntakeDraft {
    pub task: TaskDefinition,
    pub direction_title: String,
    pub normalized_prompt: String,
    pub generated_at: DateTime<Utc>,
    pub collision_suffix: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
// prepared proposal은 preview UI와 eventual commit action 사이의 stable handoff다.
pub struct PlanningTaskIntakeProposal {
    pub request: PlanningTaskIntakeRequest,
    pub draft: PlanningTaskIntakeDraft,
    pub mutation_preview: PlanningTaskCreatePreview,
    pub observed_planning_revision: i64,
    pub preview_lines: Vec<String>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
// commit result는 lower-level mutation result를 runtime-intake terminology로 바꾼 결과다.
pub struct PlanningTaskIntakeCommitResult {
    pub committed_task_id: String,
    pub committed_planning_revision: i64,
    pub queue_head: Option<PriorityQueueTask>,
    pub task_authority_committed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
// validation error는 test가 볼 machine-readable code와 adapter가 보여 줄 user-facing message를 같이 가진다.
pub struct PlanningTaskIntakeValidationError {
    pub code: &'static str,
    pub message: String,
}
impl PlanningTaskIntakeValidationError {
    // local constructor는 모든 intake validation failure가 같은 code/message shape를 갖게 한다.
    fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }

    // public service method는 anyhow error를 반환하지만, test는 structured validation code를 직접 검증한다.
    fn into_anyhow(self) -> anyhow::Error {
        anyhow!("{}", self.message)
    }
}

#[derive(Clone, Default)]
// generated draft가 mutation preview layer로 넘어가기 전 검사하는 stateless validator다.
pub struct PlanningTaskIntakeValidationService;
impl PlanningTaskIntakeValidationService {
    pub fn new() -> Self {
        Self
    }

    /*
     * generated draft를 accepted direction/task authority에 맞춰 검증한다. mutation preview 전 generator mistake를 잡는
     * 단계이며, blank field, inactive/unknown direction, priority bound, duplicate id, existing task를 가리키지
     * 않는 dependency/blocker link를 여기서 차단한다.
     */
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
/*
 * intake service는 authority seeding, workspace validation, draft generation, mutation preview, final commit을
 * 조율한다. 의도적으로 PlanningTaskMutationService를 재사용해 `:task` intake가 worker-produced
 * planning_task_commands와 같은 DB authority path를 따르게 한다.
 */
pub struct PlanningTaskIntakeService {
    planning_workspace_port: Arc<dyn PlanningWorkspacePort>,
    planning_task_repository_port: Arc<dyn PlanningTaskRepositoryPort>,
    planning_validation_service: PlanningValidationService,
    authority_seed_service: PlanningAuthoritySeedService,
    mutation_service: PlanningTaskMutationService,
    draft_generator: Arc<dyn PlanningTaskDraftGenerator>,
}
impl PlanningTaskIntakeService {
    // production constructor는 local deterministic draft generator를 사용한다.
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

    // test는 generator만 주입하고 seeding/mutation collaborator는 같은 흐름으로 유지할 수 있다.
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

    /*
     * authority를 mutate하지 않고 preview 가능한 task proposal을 만든다. collision handling과 revision capture는
     * mutation preview layer가 소유하므로, 여기서 반환하는 draft는 raw generator output이 아니라 post-preview task다.
     */
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
                source_turn_id: request.source_turn_id.clone(),
                provenance: request.provenance.clone(),
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

    // prepare 중 capture된 preview를 commit한다. confirmation 단계에서는 draft를 다시 생성하지 않는다.
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

    /*
     * intake에 필요한 authority context를 load한다. 새로 초기화된 workspace를 위해 default authority를 먼저 seed하고,
     * 그 다음 file-backed result-output과 DB-backed direction/task authority를 함께 validate한 뒤에만 draft를 생성한다.
     */
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
// preview layer가 관찰한 revision과 validated authority snapshot을 함께 담는 내부 context다.
struct PlanningTaskIntakeContext {
    directions: DirectionCatalogDocument,
    task_authority: TaskAuthorityDocument,
    task_planning_revision: i64,
}

// required active planning file 누락을 repair-oriented message로 표면화한다.
fn required_workspace_body<'a>(
    _workspace_record: &'a PlanningWorkspaceLoadRecord,
    path: &'static str,
    body: Option<&'a str>,
) -> Result<&'a str> {
    body.ok_or_else(|| {
        anyhow!(
            "Planning workspace is incomplete: missing {path}. Run :doctor to inspect the workspace, then use :planning or admin controls to restore planning files."
        )
    })
}

// validation wording을 adapter에 validator 내부 구조를 노출하지 않는 doctor guidance로 낮춘다.
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

// generated dependency/blocker link는 existing task만 가리킬 수 있고 draft 자기 자신은 가리킬 수 없다.
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

// generated draft를 mutation service input shape로 변환한다.
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

// mutation preview task를 사용한다. preview 결과에는 collision suffix나 normalized mutation default가 반영될 수 있다.
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

// preview line은 CLI/TUI surface가 보여 줄 compact human-readable confirmation copy다.
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
// shared fixture는 sibling intake test에 공개해 generator와 validator expectation을 맞춘다.
pub(super) mod tests {
    use chrono::{TimeZone, Utc};

    use super::{
        LocalPromptTaskDraftGenerator, PlanningTaskDraftGenerator,
        PlanningTaskIntakeGenerationRequest, PlanningTaskIntakeRequest,
        PlanningTaskIntakeValidationService,
    };
    use crate::domain::planning::{
        DirectionCatalogDocument, DirectionDefinition, DirectionState, QueueIdleConfig,
        TaskAuthorityDocument, TaskMutationProvenance,
    };

    // active direction 두 개를 두어 generator test가 inactive noise 없이 default direction selection을 검증하게 한다.
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

    // stable turn metadata를 가진 minimal intake request fixture다.
    pub(super) fn request(prompt: &str) -> PlanningTaskIntakeRequest {
        PlanningTaskIntakeRequest {
            workspace_directory: "/tmp/workspace".to_string(),
            raw_prompt: prompt.to_string(),
            source_turn_id: Some("turn-1".to_string()),
            provenance: TaskMutationProvenance::default(),
            requested_direction_id: None,
            observed_planning_revision: None,
        }
    }

    #[test]
    // validator test는 가장 중요한 pre-preview guardrail을 하나의 compact fixture에 고정한다.
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
