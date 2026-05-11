/*
 * repair prompt는 reconciliation failure와 planning-only worker turn 사이의 bridge다.
 * reconciliation은 safety decision을 소유하고 accepted DB authority, rejected candidate material,
 * validation error, retry metadata를 제공한다. 이 모듈은 그 evidence를 prompt로 렌더링하되,
 * worker에게 authority file 교체를 요구하지 않고 정상 planning-task command mutation contract를
 * 계속 사용하게 만든다.
 */
use std::collections::{BTreeSet, HashMap};

use crate::application::service::planning::shared::prompt_sections::{
    PlanningPromptHandoff, PlanningTaskMutationPromptMode, add_planning_task_mutation_sections,
    repair_constraints, repair_previous_handoff_lines, truncate_prompt_section,
};
use crate::application::service::prompt_component::PromptDocument;
use crate::domain::planning::{TaskAuthorityDocument, TaskDefinition};

use super::reconciliation::PlanningRepairRequest;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
// public handoff shape는 worker orchestration에서 필요한 값만 복사한다. repair prompt가
// orchestration domain type을 직접 노출하지 않게 하기 위한 얇은 DTO다.
pub struct PlanningRepairPromptHandoff<'a> {
    pub task_id: &'a str,
    pub task_title: &'a str,
    pub updated_at: &'a str,
    pub status_label: &'a str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
// retry reason은 host가 반복 실패한 repair attempt를 감지했을 때만 좁은 추가 지시를 넣는다.
pub enum PlanningRepairRetryReason {
    TaskAuthorityUnchanged,
    TaskAuthorityStillInvalid,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
/*
 * focused excerpt는 prompt가 관련 task를 식별할 수 있을 때 full authority blob을 대체한다.
 * 비어 있으면 의도적으로 full truncated request JSON으로 fallback한다. parse failure가 evidence를
 * 숨겨 repair worker가 무엇을 고쳐야 하는지 모르게 되는 상황을 피하기 위해서다.
 */
struct PlanningRepairPromptContext {
    accepted_heading: Option<String>,
    accepted_excerpt: Option<String>,
    rejected_heading: Option<String>,
    rejected_excerpt: Option<String>,
}

/*
 * 실패한 planning authority candidate 하나에 대한 repair prompt를 만든다. section 순서가 중요하다.
 * role/retry/handoff/validation이 worker 실행 이유를 먼저 고정하고, trusted direction/queue excerpt가
 * 현재 DB state를 세운다. 그 다음 shared repair mutation block이 final answer를
 * planning_task_commands로 제한한 뒤 candidate-specific excerpt를 보여 준다.
 */
pub fn build_planning_repair_prompt(
    request: &PlanningRepairRequest,
    previous_handoff: Option<PlanningRepairPromptHandoff<'_>>,
    attempt_number: usize,
    max_attempts: usize,
    retry_reason: Option<PlanningRepairRetryReason>,
) -> String {
    let prompt_context = build_planning_repair_prompt_context(request, previous_handoff);

    // focused accepted evidence를 우선하지만, 항상 bounded accepted DB authority baseline은 포함한다.
    let accepted_excerpt = prompt_context
        .accepted_excerpt
        .clone()
        .unwrap_or_else(|| truncate_prompt_section(&request.accepted_task_authority_json, 4_000));
    let accepted_heading = prompt_context
        .accepted_heading
        .clone()
        .unwrap_or_else(|| "accepted-task-authority".to_string());
    let rejected_excerpt = rejected_excerpt(request, &prompt_context);
    let rejected_heading = prompt_context
        .rejected_heading
        .clone()
        .unwrap_or_else(|| "rejected-candidate".to_string());
    let direction_authority_excerpt =
        truncate_prompt_section(&request.direction_authority_json, 4_000);
    let accepted_queue_projection_excerpt =
        truncate_prompt_section(&request.accepted_queue_projection_json, 2_000);

    // shared repair section은 accepted/rejected excerpt보다 먼저 들어간다. output rule이 evidence를
    // 해석하는 frame을 먼저 제공하게 하려는 순서다.
    let builder = PromptDocument::builder("planning-repair")
        .lines("role", repair_role_lines(attempt_number, max_attempts))
        .bullets("constraints", repair_constraints())
        .lines("retry", retry_instruction_lines(retry_reason))
        .lines(
            "previous-handoff",
            repair_previous_handoff_lines(previous_handoff.map(repair_handoff)),
        )
        .lines("validation", validation_lines(request))
        .optional_code_block(
            "direction-authority",
            "json",
            Some(&direction_authority_excerpt),
        )
        .optional_code_block(
            "accepted-db-queue-projection",
            "json",
            Some(&accepted_queue_projection_excerpt),
        );
    add_planning_task_mutation_sections(builder, PlanningTaskMutationPromptMode::Repair)
        .optional_code_block(&accepted_heading, "json", Some(&accepted_excerpt))
        .optional_code_block(&rejected_heading, "json", rejected_excerpt.as_deref())
        .bullets("final-response", final_response_rules())
        .build()
        .render()
}

// role line은 mutable evidence를 싣지 않고 repair attempt metadata만 logs/prompt projection에서 보이게 한다.
fn repair_role_lines(attempt_number: usize, max_attempts: usize) -> Vec<String> {
    vec![
        "session=planning-repair-only".to_string(),
        format!("attempt={attempt_number}/{max_attempts}"),
        "reason=previous DB task authority candidate failed validation".to_string(),
    ]
}

// retry instruction은 첫 attempt에는 없고, 이후 attempt에서만 좁게 들어간다. 불필요한 overfitting을 피하기 위해서다.
fn retry_instruction_lines(retry_reason: Option<PlanningRepairRetryReason>) -> Vec<String> {
    retry_reason
        .map(|retry_reason| vec![format!("instruction={}", retry_reason.instruction())])
        .unwrap_or_default()
}

// local public handoff wrapper를 refresh/repair가 공유하는 prompt-section type으로 변환한다.
fn repair_handoff(handoff: PlanningRepairPromptHandoff<'_>) -> PlanningPromptHandoff<'_> {
    PlanningPromptHandoff {
        task_id: handoff.task_id,
        task_title: handoff.task_title,
        updated_at: handoff.updated_at,
        status_label: handoff.status_label,
    }
}

// validation line은 human summary, 구체적인 validator message, archive pointer를 한 section에 모은다.
fn validation_lines(request: &PlanningRepairRequest) -> Vec<String> {
    let mut lines = vec![format!("failure_summary={}", request.failure_summary)];
    lines.extend(
        request
            .validation_errors
            .iter()
            .filter(|error| !error.trim().is_empty())
            .map(|error| format!("- {error}")),
    );
    if let Some(rejected_archive_path) = request.rejected_archive_path.as_deref() {
        lines.push(format!("rejected_archive={rejected_archive_path}"));
    }
    lines
}

// rejected evidence는 optional이다. 일부 실패는 parse 가능한 authority가 아니라 malformed command envelope에서 온다.
fn rejected_excerpt(
    request: &PlanningRepairRequest,
    prompt_context: &PlanningRepairPromptContext,
) -> Option<String> {
    request
        .rejected_task_authority_json
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        .map(|rejected_task_authority_json| {
            prompt_context
                .rejected_excerpt
                .clone()
                .unwrap_or_else(|| truncate_prompt_section(rejected_task_authority_json, 4_000))
        })
}

// final response section은 shared contract보다 의도적으로 짧다. 마지막 강조 역할만 맡는다.
fn final_response_rules() -> Vec<String> {
    vec![
        "Briefly summarize what was fixed.".to_string(),
        "Return the corrected planning task command envelope in the required fenced JSON object."
            .to_string(),
        "Do not answer with bare `DONE`; explain why if no ledger change is needed.".to_string(),
    ]
}

/*
 * repair prompt용 focused authority excerpt를 만든다. focus source는 세 가지다. loop 가능성이
 * 있는 previous handoff task, rejected candidate가 바꾼 task, validation error에 직접 언급된
 * task id다. parsing step이 하나라도 실패하면 caller가 full truncated JSON으로 fallback해 repair
 * 가능성을 유지한다.
 */
fn build_planning_repair_prompt_context(
    request: &PlanningRepairRequest,
    previous_handoff: Option<PlanningRepairPromptHandoff<'_>>,
) -> PlanningRepairPromptContext {
    let accepted_task_authority =
        parse_task_authority_document(&request.accepted_task_authority_json);
    let rejected_task_authority = request
        .rejected_task_authority_json
        .as_deref()
        .and_then(parse_task_authority_document);
    let Some(accepted_task_authority) = accepted_task_authority.as_ref() else {
        return PlanningRepairPromptContext::default();
    };
    let focus_ids = collect_focus_task_ids(
        accepted_task_authority,
        rejected_task_authority.as_ref(),
        &request.validation_errors,
        previous_handoff,
    );
    if focus_ids.is_empty() {
        return PlanningRepairPromptContext::default();
    }

    PlanningRepairPromptContext {
        accepted_heading: Some(
            "accepted-task-authority-focus-current-handoff-and-validation".to_string(),
        ),
        accepted_excerpt: serialize_focused_task_authority_excerpt(
            accepted_task_authority,
            &focus_ids,
        ),
        rejected_heading: rejected_task_authority
            .as_ref()
            .map(|_| "rejected-candidate-focus-changed-tasks-and-validation".to_string()),
        rejected_excerpt: rejected_task_authority.as_ref().and_then(|task_authority| {
            serialize_focused_task_authority_excerpt(task_authority, &focus_ids)
        }),
    }
}

// prompt construction은 host turn을 실패시키기보다 raw JSON excerpt로 degrade해야 하므로 best-effort parse다.
fn parse_task_authority_document(body: &str) -> Option<TaskAuthorityDocument> {
    serde_json::from_str(body).ok()
}

/*
 * repair prompt에서 full context를 받을 task id를 모은다. direct evidence에서 시작한 뒤
 * dependency/blocker edge를 따라 확장한다. worker가 주변 graph를 충분히 보고 adjacent task
 * constraint를 깨는 priority/status fix를 만들지 않게 하기 위한 focus set이다.
 */
fn collect_focus_task_ids(
    accepted_task_authority: &TaskAuthorityDocument,
    rejected_task_authority: Option<&TaskAuthorityDocument>,
    validation_errors: &[String],
    previous_handoff: Option<PlanningRepairPromptHandoff<'_>>,
) -> BTreeSet<String> {
    let mut focus_ids = BTreeSet::new();
    if let Some(previous_handoff) = previous_handoff {
        let task_id = previous_handoff.task_id.trim();
        if !task_id.is_empty() {
            focus_ids.insert(task_id.to_string());
        }
    }
    let mut known_task_ids = accepted_task_authority
        .tasks
        .iter()
        .map(|task| task.id.trim().to_string())
        .collect::<BTreeSet<_>>();
    if let Some(rejected_task_authority) = rejected_task_authority {
        known_task_ids.extend(
            rejected_task_authority
                .tasks
                .iter()
                .map(|task| task.id.trim().to_string()),
        );
        focus_ids.extend(changed_task_ids(
            accepted_task_authority,
            rejected_task_authority,
        ));
    }
    for validation_error in validation_errors {
        for task_id in &known_task_ids {
            if validation_error_mentions_task_id(validation_error, task_id) {
                focus_ids.insert(task_id.clone());
            }
        }
    }

    expand_related_task_ids(&mut focus_ids, accepted_task_authority);
    if let Some(rejected_task_authority) = rejected_task_authority {
        expand_related_task_ids(&mut focus_ids, rejected_task_authority);
    }

    focus_ids
}

// accepted/rejected authority는 normalized task content로 비교한다. 표면적인 formatting 차이는 변경으로 보지 않는다.
fn changed_task_ids(
    accepted_task_authority: &TaskAuthorityDocument,
    rejected_task_authority: &TaskAuthorityDocument,
) -> BTreeSet<String> {
    let accepted_task_map = accepted_task_authority
        .tasks
        .iter()
        .map(|task| (task.id.trim(), task))
        .collect::<HashMap<_, _>>();
    let rejected_task_map = rejected_task_authority
        .tasks
        .iter()
        .map(|task| (task.id.trim(), task))
        .collect::<HashMap<_, _>>();
    let all_task_ids = accepted_task_map
        .keys()
        .copied()
        .chain(rejected_task_map.keys().copied())
        .collect::<BTreeSet<_>>();
    let mut changed_task_ids = BTreeSet::new();
    for task_id in all_task_ids {
        match (
            accepted_task_map.get(task_id),
            rejected_task_map.get(task_id),
        ) {
            (Some(accepted_task), Some(rejected_task))
                if normalized_task_definition(accepted_task)
                    != normalized_task_definition(rejected_task) =>
            {
                changed_task_ids.insert(task_id.to_string());
            }
            (None, Some(_)) | (Some(_), None) => {
                changed_task_ids.insert(task_id.to_string());
            }
            _ => {}
        }
    }

    changed_task_ids
}

// 비교 규칙을 이름 있는 함수로 둔다. 미래의 domain normalization 변경이 repair boundary에서 눈에 띄게 하기 위해서다.
fn normalized_task_definition(task: &TaskDefinition) -> TaskDefinition {
    task.normalized()
}

// tokenization은 substring match를 피한다. `task-10` 같은 prose에서 `task-1`을 잘못 추론하지 않게 한다.
fn validation_error_mentions_task_id(validation_error: &str, task_id: &str) -> bool {
    validation_error
        .split(|character: char| {
            !(character.is_ascii_alphanumeric() || character == '-' || character == '_')
        })
        .any(|token| token == task_id)
}

/*
 * 새로 관련된 task가 더 이상 나타나지 않을 때까지 focus id를 task graph 전체로 확장한다.
 * repair candidate는 변경 task가 dependency/blocker 상태와 충돌해 실패하는 경우가 많다. transitive
 * neighborhood를 포함하면 worker가 validator error에 직접 언급된 task만이 아니라 관계 자체를 고칠 수 있다.
 */
fn expand_related_task_ids(
    focus_ids: &mut BTreeSet<String>,
    task_authority: &TaskAuthorityDocument,
) {
    let mut expanded = true;
    while expanded {
        expanded = false;
        let seed_ids = focus_ids.clone();
        for task in &task_authority.tasks {
            let task_id = task.id.trim();
            let directly_related = seed_ids.contains(task_id)
                || task
                    .depends_on
                    .iter()
                    .any(|dependency_id| seed_ids.contains(dependency_id.trim()))
                || task
                    .blocked_by
                    .iter()
                    .any(|blocker_id| seed_ids.contains(blocker_id.trim()));
            if !directly_related {
                continue;
            }

            expanded |= focus_ids.insert(task_id.to_string());
            for dependency_id in &task.depends_on {
                let dependency_id = dependency_id.trim();
                if !dependency_id.is_empty() {
                    expanded |= focus_ids.insert(dependency_id.to_string());
                }
            }
            for blocker_id in &task.blocked_by {
                let blocker_id = blocker_id.trim();
                if !blocker_id.is_empty() {
                    expanded |= focus_ids.insert(blocker_id.to_string());
                }
            }
        }
    }
}

// focused task만 기존 prompt test가 기대하는 authority document shape로 다시 직렬화한다.
fn serialize_focused_task_authority_excerpt(
    task_authority: &TaskAuthorityDocument,
    focus_ids: &BTreeSet<String>,
) -> Option<String> {
    let focused_tasks = task_authority
        .tasks
        .iter()
        .filter(|task| focus_ids.contains(task.id.trim()))
        .cloned()
        .collect::<Vec<_>>();
    if focused_tasks.is_empty() {
        return None;
    }
    serde_json::to_string_pretty(&TaskAuthorityDocument {
        version: task_authority.version,
        tasks: focused_tasks,
    })
    .ok()
}

impl PlanningRepairRetryReason {
    // 이 instruction은 operator-facing prompt copy라 enum 이름보다 직접적인 문장으로 쓴다.
    fn instruction(self) -> &'static str {
        match self {
            Self::TaskAuthorityUnchanged => {
                "The previous repair attempt did not change the task command payload; emit corrected `planning_task_commands` that apply a real task mutation or explain why a planning-tool mutation already applied it."
            }
            Self::TaskAuthorityStillInvalid => {
                "The previous repair attempt changed the task command payload but validation still failed; emit corrected `planning_task_commands` that resolve every listed validation error."
            }
        }
    }
}
