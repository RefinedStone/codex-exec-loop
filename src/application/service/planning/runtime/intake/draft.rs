use super::{
    PlanningTaskIntakeDraft, PlanningTaskIntakeRequest, PlanningTaskIntakeValidationError,
};
use crate::domain::planning::{
    DirectionCatalogDocument, DirectionDefinition, DirectionState, TaskActor, TaskStatus,
};
use anyhow::Result;
use chrono::{DateTime, SecondsFormat, Utc};

const DEFAULT_RUNTIME_TASK_PRIORITY: i32 = 80;
const TASK_TITLE_LIMIT: usize = 72;

// 초안 생성기는 런타임 intake가 mutation preview 계층에 들어가기 직전의 마지막
// 결정적 변환 지점이다. 여기서는 사용자 프롬프트와 방향 카탈로그만 보고 태스크
// 후보를 만들고, 카탈로그 revision 검증이나 ID 충돌 판정은 preview 경계에 남긴다.
// 트레이트로 분리해 두면 서비스 테스트는 충돌/검증 시나리오에 맞춘 초안을 주입하고,
// 운영 경로는 동일한 로컬 prompt-to-task 투영을 계속 사용할 수 있다.
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
    // 생성 시각은 호출자가 소유한다. 그래야 preview 계층이 ID 충돌을 보고했을 때
    // 재시도마다 시간 기반 ID가 흔들리지 않고, 확인된 충돌 suffix만 바뀐다.
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

        // runtime intake로 태어난 태스크는 의도적으로 Ready, User 소유, 무의존 상태로
        // 시작한다. 이후 planner가 의존성과 우선순위를 다듬을 수 있지만, 이 경로는
        // 원문 프롬프트를 source description으로 보존하고 목록용 title만 정규화한다.
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
                source_turn_id: request.request.source_turn_id.clone(),
                provenance: request.request.provenance.clone(),
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
    // 의미 요약이 아니라 공백만 접는 안정화 단계다. title, hash, preview 표시가 줄바꿈
    // 위치에 따라 달라지지 않도록 같은 사용자 문장을 같은 문자열로 투영한다.
    prompt.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn select_direction<'a>(
    requested_direction_id: Option<&str>,
    directions: &'a DirectionCatalogDocument,
) -> std::result::Result<&'a DirectionDefinition, PlanningTaskIntakeValidationError> {
    // 명시된 direction은 권위 있는 선택으로 취급하지만, 이미 Active인 lane이어야 한다.
    // 사용자 프롬프트 하나로 archived planning lane을 되살리는 일은 mutation 정책 밖이다.
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

    // direction이 없으면 TUI의 안정적인 기본 lane인 general-workstream을 먼저 고른다.
    // 그 lane이 생기기 전의 오래된 카탈로그도 읽을 수 있도록 마지막에는 임의의 Active
    // direction으로 후퇴하되, Active가 전혀 없으면 intake 자체를 막는다.
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
    // 목록 title은 첫 문장 단위로 자르고, 공백뿐인 입력은 고정 fallback으로 대체한다.
    // 글자 수 기준 제한을 적용해 TUI session/task 목록 폭을 입력 길이에 종속시키지 않는다.
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
    // timestamp와 prompt hash를 결합해 사람이 읽을 수 있고 재시도에 안정적인 base를 만든다.
    // suffix는 preview 계층이 실제 충돌을 확인한 뒤에만 붙이므로 정상 생성 ID를 불필요하게
    // 흔들지 않는다.
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
    // 보안용 digest가 아니라 ID suffix 가독성을 위한 결정적 FNV 축약값이다. 같은 prompt는
    // 같은 suffix를 만들고, 충돌이 나면 상위 preview/retry 경로가 숫자 suffix로 구분한다.
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
    // 테스트 전용 helper로 preview 충돌 재시도가 suffix 없음 -> 1 -> 2 순서로 움직이는
    // 정책을 작은 단위에서 고정한다.
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
