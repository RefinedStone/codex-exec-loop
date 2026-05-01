use crate::application::service::planning::shared::prompt_sections::runtime_task_authority_contract_rules;
use crate::application::service::prompt_component::PromptDocument;
use crate::domain::planning::{
    DirectionCatalogDocument, DirectionState, PriorityQueueProjection, PriorityQueueTask,
};

const MAX_VISIBLE_QUEUE_TASKS: usize = 5;
const MAX_SKIPPED_QUEUE_TASKS: usize = 3;
const MAX_VISIBLE_PROPOSED_TASKS: usize = 3;

pub(super) fn build_prompt_fragment(
    directions: &DirectionCatalogDocument,
    queue_projection: &PriorityQueueProjection,
    result_output_markdown: &str,
) -> String {
    PromptDocument::builder("planning-context")
        .lines("directions", direction_context_lines(directions))
        .lines("queue-idle", queue_idle_lines(directions))
        .lines("queue", queue_context_lines(queue_projection))
        .bullets(
            "task-authority-contract",
            runtime_task_authority_contract_rules(),
        )
        .optional_text("result-output-prompt", Some(result_output_markdown))
        .bullets("follow-up-proposals", follow_up_proposal_rules())
        .build()
        .render()
}

pub(super) fn trimmed_non_empty(value: &str) -> Option<&str> {
    let value = value.trim();
    (!value.is_empty()).then_some(value)
}

fn direction_context_lines(directions: &DirectionCatalogDocument) -> Vec<String> {
    directions
        .directions
        .iter()
        .flat_map(|direction| {
            let mut lines = vec![
                format!(
                    "- id={}; title={}; state={}",
                    direction.id.trim(),
                    direction.title.trim(),
                    direction_state_label(direction.state),
                ),
                format!("  summary={}", direction.summary.trim()),
                format!(
                    "  success_criteria={}",
                    direction.success_criteria.join(" | ")
                ),
            ];
            if !direction.scope_hints.is_empty() {
                lines.push(format!(
                    "  scope_hints={}",
                    direction.scope_hints.join(" | ")
                ));
            }
            if let Some(detail_doc_path) = trimmed_non_empty(direction.detail_doc_path.as_str()) {
                lines.push(format!("  detail_doc_path={detail_doc_path}"));
            }
            lines
        })
        .collect()
}

fn queue_idle_lines(directions: &DirectionCatalogDocument) -> Vec<String> {
    let mut lines = vec![format!("policy={}", directions.queue_idle.policy.label())];
    if let Some(prompt_path) = trimmed_non_empty(directions.queue_idle.prompt_path.as_str()) {
        lines.push(format!("prompt_path={prompt_path}"));
    }
    lines
}

fn queue_context_lines(queue_projection: &PriorityQueueProjection) -> Vec<String> {
    let mut lines = Vec::new();
    match queue_projection.next_task.as_ref() {
        Some(task) => {
            lines.push(format!("next_task={}", queue_task_line(task)));
            lines.push(format!(
                "next_task_rank_reasons={}",
                task.rank_reasons.join(" | ")
            ));
        }
        None => lines.push("next_task=none".to_string()),
    }

    lines.extend(active_task_lines(queue_projection));
    lines.extend(proposed_task_lines(queue_projection));
    lines.extend(skipped_task_lines(queue_projection));
    lines
}

fn active_task_lines(queue_projection: &PriorityQueueProjection) -> Vec<String> {
    if queue_projection.active_tasks.is_empty() {
        return vec!["visible_tasks=none".to_string()];
    }

    let visible_tasks = queue_projection.visible_tasks(MAX_VISIBLE_QUEUE_TASKS);
    let mut lines = vec![format!(
        "visible_tasks=top {} of {}",
        visible_tasks.len(),
        queue_projection.active_tasks.len()
    )];
    for task in visible_tasks {
        lines.push(format!("- {}", queue_task_line(&task)));
        lines.push(format!("  rank_reasons={}", task.rank_reasons.join(" | ")));
    }
    lines
}

fn proposed_task_lines(queue_projection: &PriorityQueueProjection) -> Vec<String> {
    if queue_projection.proposed_tasks.is_empty() {
        return Vec::new();
    }

    let proposed_tasks = queue_projection.visible_proposed_tasks(MAX_VISIBLE_PROPOSED_TASKS);
    let mut lines = vec![format!(
        "proposed_tasks=top {} of {} promotable",
        proposed_tasks.len(),
        queue_projection.proposed_tasks.len()
    )];
    for task in proposed_tasks {
        lines.push(format!("- {}", queue_task_line(&task)));
        lines.push(format!("  rank_reasons={}", task.rank_reasons.join(" | ")));
    }
    lines
}

fn skipped_task_lines(queue_projection: &PriorityQueueProjection) -> Vec<String> {
    if queue_projection.skipped_tasks.is_empty() {
        return Vec::new();
    }

    let skipped_tasks = queue_projection
        .skipped_tasks
        .iter()
        .take(MAX_SKIPPED_QUEUE_TASKS)
        .collect::<Vec<_>>();
    let mut lines = vec![format!(
        "skipped_tasks=showing {} of {}",
        skipped_tasks.len(),
        queue_projection.skipped_tasks.len()
    )];
    for skipped_task in skipped_tasks {
        lines.push(format!(
            "- id={}; title={}; direction={}; status={}; reason={}",
            skipped_task.task_id.trim(),
            skipped_task.task_title.trim(),
            skipped_task.direction_id.trim(),
            skipped_task.status.label(),
            skipped_task.reason.trim(),
        ));
    }
    lines
}

fn queue_task_line(task: &PriorityQueueTask) -> String {
    format!(
        "rank {}; id={}; title={}; direction={}; status={}; combined_priority={}",
        task.rank,
        task.task_id.trim(),
        task.task_title.trim(),
        task.direction_id.trim(),
        task.status.label(),
        task.combined_priority,
    )
}

fn follow_up_proposal_rules() -> Vec<String> {
    vec![
        "If the final answer offers concrete follow-up options, create each option through task authority as a separate `proposed` task linked to an existing direction."
            .to_string(),
        "Use `proposed` only for direction-linked candidates that should wait for explicit promote, prioritize, queue, or execute intent."
            .to_string(),
        "If `next_task=none` but proposals exist and the user asks to keep going, promote the single highest-priority executable task and keep the rest queued or proposed."
            .to_string(),
        "When the user later asks to prioritize, queue, or execute earlier proposals, update the relevant proposal tasks instead of creating duplicates."
            .to_string(),
    ]
}

fn direction_state_label(state: DirectionState) -> &'static str {
    match state {
        DirectionState::Active => "active",
        DirectionState::Paused => "paused",
        DirectionState::Done => "done",
    }
}
