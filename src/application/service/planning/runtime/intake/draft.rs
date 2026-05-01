use anyhow::Result;
use chrono::{DateTime, SecondsFormat, Utc};

use crate::domain::planning::{
    DirectionCatalogDocument, DirectionDefinition, DirectionState, TaskActor, TaskStatus,
};

use super::{
    PlanningTaskIntakeDraft, PlanningTaskIntakeRequest, PlanningTaskIntakeValidationError,
};

const DEFAULT_RUNTIME_TASK_PRIORITY: i32 = 80;
const TASK_TITLE_LIMIT: usize = 72;

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
            task: crate::domain::planning::TaskDefinition {
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

pub(super) fn normalize_prompt(prompt: &str) -> String {
    prompt.split_whitespace().collect::<Vec<_>>().join(" ")
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

#[cfg(test)]
fn increment_suffix(suffix: Option<u32>) -> Option<u32> {
    Some(suffix.unwrap_or(0) + 1)
}

#[cfg(test)]
mod tests {
    use chrono::{TimeZone, Utc};

    use super::{
        LocalPromptTaskDraftGenerator, PlanningTaskDraftGenerator,
        PlanningTaskIntakeGenerationRequest, increment_suffix,
    };
    use crate::application::service::planning::runtime::intake::tests::{directions, request};
    use crate::domain::planning::{TaskActor, TaskStatus};

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
    fn increment_suffix_starts_with_one() {
        assert_eq!(increment_suffix(None), Some(1));
        assert_eq!(increment_suffix(Some(1)), Some(2));
    }
}
