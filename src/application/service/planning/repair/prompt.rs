use std::collections::{BTreeSet, HashMap};

use crate::application::service::planning::shared::prompt_sections::{
    PlanningPromptHandoff, PlanningTaskMutationPromptMode, add_planning_task_mutation_sections,
    repair_constraints, repair_previous_handoff_lines, truncate_prompt_section,
};
use crate::application::service::prompt_component::PromptDocument;
use crate::domain::planning::{TaskAuthorityDocument, TaskDefinition};

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
    TaskAuthorityUnchanged,
    TaskAuthorityStillInvalid,
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
    let prompt_context = build_planning_repair_prompt_context(request, previous_handoff);
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

fn repair_role_lines(attempt_number: usize, max_attempts: usize) -> Vec<String> {
    vec![
        "session=planning-repair-only".to_string(),
        format!("attempt={attempt_number}/{max_attempts}"),
        "reason=previous DB task authority candidate failed validation".to_string(),
    ]
}

fn retry_instruction_lines(retry_reason: Option<PlanningRepairRetryReason>) -> Vec<String> {
    retry_reason
        .map(|retry_reason| vec![format!("instruction={}", retry_reason.instruction())])
        .unwrap_or_default()
}

fn repair_handoff(handoff: PlanningRepairPromptHandoff<'_>) -> PlanningPromptHandoff<'_> {
    PlanningPromptHandoff {
        task_id: handoff.task_id,
        task_title: handoff.task_title,
        updated_at: handoff.updated_at,
        status_label: handoff.status_label,
    }
}

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

fn final_response_rules() -> Vec<String> {
    vec![
        "Briefly summarize what was fixed.".to_string(),
        "Return the corrected planning task command envelope in the required fenced JSON object."
            .to_string(),
        "Do not answer with bare `DONE`; explain why if no ledger change is needed.".to_string(),
    ]
}

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

fn parse_task_authority_document(body: &str) -> Option<TaskAuthorityDocument> {
    serde_json::from_str(body).ok()
}

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
    fn instruction(self) -> &'static str {
        match self {
            Self::TaskAuthorityUnchanged => {
                "직전 repair 시도에서 task authority payload가 바뀌지 않았습니다. 이번 턴에서는 갱신된 `task_authority` JSON payload를 반드시 다시 반환하세요."
            }
            Self::TaskAuthorityStillInvalid => {
                "직전 repair 시도에서 task authority payload를 수정했지만 여전히 유효하지 않습니다. 이번 턴에서는 validation 오류를 모두 해결한 `task_authority` JSON payload를 다시 반환하세요."
            }
        }
    }
}
