use std::collections::{BTreeSet, HashMap};

use crate::domain::planning::{TaskDefinition, TaskLedgerDocument};

use super::reconciliation::PlanningRepairRequest;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlanningRepairPromptHandoff<'a> {
    pub task_id: &'a str,
    pub task_title: &'a str,
    pub updated_at: &'a str,
    pub status_label: &'a str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlanningRepairRetryReason {
    TaskLedgerUnchanged,
    TaskLedgerStillInvalid,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
struct PlanningRepairPromptContext {
    accepted_heading: Option<String>,
    accepted_excerpt: Option<String>,
    rejected_heading: Option<String>,
    rejected_excerpt: Option<String>,
}

pub fn build_planning_repair_prompt(
    request: &PlanningRepairRequest,
    previous_handoff: Option<PlanningRepairPromptHandoff<'_>>,
    attempt_number: usize,
    max_attempts: usize,
    retry_reason: Option<PlanningRepairRetryReason>,
) -> String {
    let mut lines = vec![
        "대리인입니다.".to_string(),
        format!("planning repair {attempt_number}/{max_attempts} 입니다."),
        "이전 턴에서 `task-ledger.json` 후보가 validation을 통과하지 못했습니다.".to_string(),
        "이번 턴에서는 `.codex-exec-loop/planning/task-ledger.json` 하나만 고치세요.".to_string(),
        "- `directions.toml`, `task-ledger.schema.json`, `result-output.md`, `queue.snapshot.json` 은 수정하지 마세요.".to_string(),
        "- 현재 작업공간에는 마지막 accepted `task-ledger.json` 이 이미 복원돼 있습니다."
            .to_string(),
        "- 아래 validation 오류를 모두 해결하는 유효한 JSON으로 다시 작성하세요.".to_string(),
        "- 기존 direction frame 밖의 관련 없는 새 작업은 추가하지 마세요.".to_string(),
    ];

    if let Some(retry_reason) = retry_reason {
        lines.push(format!("- 추가 지시: {}", retry_reason.instruction()));
    }

    if let Some(previous_handoff) = previous_handoff {
        lines.push(String::new());
        lines.push("직전에 main session으로 넘긴 task:".to_string());
        lines.push(format!("- task_id: {}", previous_handoff.task_id));
        lines.push(format!("- title: {}", previous_handoff.task_title));
        lines.push(format!("- updated_at: {}", previous_handoff.updated_at));
        lines.push(format!("- status: {}", previous_handoff.status_label));
        lines.push(
            "- 같은 task를 유지하려면 그 task 자체가 바뀌었다는 근거가 ledger에 있어야 합니다."
                .to_string(),
        );
    }

    lines.push(String::new());
    lines.push(format!("Failure summary: {}", request.failure_summary));
    lines.push(String::new());
    lines.push("Validation errors:".to_string());
    for error in &request.validation_errors {
        lines.push(format!("- {error}"));
    }
    if let Some(rejected_archive_path) = request.rejected_archive_path.as_deref() {
        lines.push(format!("- rejected archive: {rejected_archive_path}"));
    }

    lines.push(String::new());
    lines.push("Accepted directions (`directions.toml`):".to_string());
    lines.push(prompt_code_block(
        "toml",
        truncate_prompt_section(&request.directions_toml, 4_000).as_str(),
    ));

    lines.push(String::new());
    lines.push("Allowed schema (`task-ledger.schema.json`):".to_string());
    lines.push(prompt_code_block(
        "json",
        truncate_prompt_section(&request.task_ledger_schema_json, 4_000).as_str(),
    ));

    let prompt_context = build_planning_repair_prompt_context(request, previous_handoff);
    let accepted_excerpt = prompt_context
        .accepted_excerpt
        .clone()
        .unwrap_or_else(|| truncate_prompt_section(&request.accepted_task_ledger_json, 4_000));

    lines.push(String::new());
    lines.push(
        prompt_context.accepted_heading.unwrap_or_else(|| {
            "Current accepted `task-ledger.json` (restored on disk):".to_string()
        }),
    );
    lines.push(prompt_code_block("json", &accepted_excerpt));

    if let Some(rejected_task_ledger_json) = request
        .rejected_task_ledger_json
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    {
        let rejected_excerpt = prompt_context
            .rejected_excerpt
            .clone()
            .unwrap_or_else(|| truncate_prompt_section(rejected_task_ledger_json, 4_000));
        lines.push(String::new());
        lines.push(
            prompt_context
                .rejected_heading
                .unwrap_or_else(|| "Rejected candidate excerpt:".to_string()),
        );
        lines.push(prompt_code_block("json", &rejected_excerpt));
    }

    lines.push(String::new());
    lines.push(
        "수정이 끝나면 무엇을 고쳤는지 짧게 요약하세요. 더 이상 고칠 것이 없어도 `DONE` 만 단독으로 출력하지 말고 이유를 설명하세요."
            .to_string(),
    );

    lines.join("\n")
}

fn build_planning_repair_prompt_context(
    request: &PlanningRepairRequest,
    previous_handoff: Option<PlanningRepairPromptHandoff<'_>>,
) -> PlanningRepairPromptContext {
    let accepted_task_ledger = parse_task_ledger_document(&request.accepted_task_ledger_json);
    let rejected_task_ledger = request
        .rejected_task_ledger_json
        .as_deref()
        .and_then(parse_task_ledger_document);
    let Some(accepted_task_ledger) = accepted_task_ledger.as_ref() else {
        return PlanningRepairPromptContext::default();
    };

    let focus_ids = collect_focus_task_ids(
        accepted_task_ledger,
        rejected_task_ledger.as_ref(),
        &request.validation_errors,
        previous_handoff,
    );
    if focus_ids.is_empty() {
        return PlanningRepairPromptContext::default();
    }

    PlanningRepairPromptContext {
        accepted_heading: Some(
            "Current accepted `task-ledger.json` focus (current handoff + validation context):"
                .to_string(),
        ),
        accepted_excerpt: serialize_focused_task_ledger_excerpt(accepted_task_ledger, &focus_ids),
        rejected_heading: rejected_task_ledger
            .as_ref()
            .map(|_| "Rejected candidate focus (changed tasks + validation context):".to_string()),
        rejected_excerpt: rejected_task_ledger
            .as_ref()
            .and_then(|task_ledger| serialize_focused_task_ledger_excerpt(task_ledger, &focus_ids)),
    }
}

fn parse_task_ledger_document(body: &str) -> Option<TaskLedgerDocument> {
    serde_json::from_str(body).ok()
}

fn collect_focus_task_ids(
    accepted_task_ledger: &TaskLedgerDocument,
    rejected_task_ledger: Option<&TaskLedgerDocument>,
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

    let mut known_task_ids = accepted_task_ledger
        .tasks
        .iter()
        .map(|task| task.id.trim().to_string())
        .collect::<BTreeSet<_>>();
    if let Some(rejected_task_ledger) = rejected_task_ledger {
        known_task_ids.extend(
            rejected_task_ledger
                .tasks
                .iter()
                .map(|task| task.id.trim().to_string()),
        );
        focus_ids.extend(changed_task_ids(accepted_task_ledger, rejected_task_ledger));
    }

    for validation_error in validation_errors {
        for task_id in &known_task_ids {
            if validation_error_mentions_task_id(validation_error, task_id) {
                focus_ids.insert(task_id.clone());
            }
        }
    }

    expand_related_task_ids(&mut focus_ids, accepted_task_ledger);
    if let Some(rejected_task_ledger) = rejected_task_ledger {
        expand_related_task_ids(&mut focus_ids, rejected_task_ledger);
    }

    focus_ids
}

fn changed_task_ids(
    accepted_task_ledger: &TaskLedgerDocument,
    rejected_task_ledger: &TaskLedgerDocument,
) -> BTreeSet<String> {
    let accepted_task_map = accepted_task_ledger
        .tasks
        .iter()
        .map(|task| (task.id.trim(), task))
        .collect::<HashMap<_, _>>();
    let rejected_task_map = rejected_task_ledger
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

fn normalized_task_definition(task: &TaskDefinition) -> TaskDefinition {
    task.normalized()
}

fn validation_error_mentions_task_id(validation_error: &str, task_id: &str) -> bool {
    validation_error
        .split(|character: char| {
            !(character.is_ascii_alphanumeric() || character == '-' || character == '_')
        })
        .any(|token| token == task_id)
}

fn expand_related_task_ids(focus_ids: &mut BTreeSet<String>, task_ledger: &TaskLedgerDocument) {
    let mut expanded = true;
    while expanded {
        expanded = false;
        let seed_ids = focus_ids.clone();
        for task in &task_ledger.tasks {
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

fn serialize_focused_task_ledger_excerpt(
    task_ledger: &TaskLedgerDocument,
    focus_ids: &BTreeSet<String>,
) -> Option<String> {
    let focused_tasks = task_ledger
        .tasks
        .iter()
        .filter(|task| focus_ids.contains(task.id.trim()))
        .cloned()
        .collect::<Vec<_>>();
    if focused_tasks.is_empty() {
        return None;
    }

    serde_json::to_string_pretty(&TaskLedgerDocument {
        version: task_ledger.version,
        tasks: focused_tasks,
    })
    .ok()
}

fn prompt_code_block(language: &str, body: &str) -> String {
    format!("```{language}\n{body}\n```")
}

fn truncate_prompt_section(body: &str, max_chars: usize) -> String {
    let body = body.trim();
    if body.chars().count() <= max_chars {
        return body.to_string();
    }

    let truncated = body.chars().take(max_chars).collect::<String>();
    format!("{truncated}\n... [truncated]")
}

impl PlanningRepairRetryReason {
    fn instruction(self) -> &'static str {
        match self {
            Self::TaskLedgerUnchanged => {
                "직전 repair 시도에서 `task-ledger.json` 이 바뀌지 않았습니다. 이번 턴에서는 그 파일을 반드시 다시 작성하세요."
            }
            Self::TaskLedgerStillInvalid => {
                "직전 repair 시도에서 `task-ledger.json` 을 수정했지만 여전히 유효하지 않습니다. 이번 턴에서는 validation 오류를 모두 해결하도록 그 파일을 다시 작성하세요."
            }
        }
    }
}
