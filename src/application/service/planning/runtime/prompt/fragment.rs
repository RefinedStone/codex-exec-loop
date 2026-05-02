use crate::application::service::planning::shared::prompt_sections::runtime_task_authority_contract_rules;
use crate::application::service::prompt_component::PromptDocument;
use crate::domain::planning::{
    DirectionCatalogDocument, DirectionState, PriorityQueueProjection, PriorityQueueTask,
};

// 학습 주석: prompt fragment는 queue 전체를 덤프하지 않고, agent가 다음 결정을 내릴 만큼만 잘라 넣습니다.
const MAX_VISIBLE_QUEUE_TASKS: usize = 5;
const MAX_SKIPPED_QUEUE_TASKS: usize = 3;
const MAX_VISIBLE_PROPOSED_TASKS: usize = 3;

pub(super) fn build_prompt_fragment(
    directions: &DirectionCatalogDocument,
    queue_projection: &PriorityQueueProjection,
    result_output_markdown: &str,
) -> String {
    /*
     * 학습 주석: runtime prompt fragment는 planning runtime이 현재 방향과 queue 상태를 Codex turn에 주입하는 경계입니다.
     * section 이름을 고정해 두면 downstream prompt tests가 큰 markdown blob 대신 의미 단위로 회귀를 잡을 수 있습니다.
     */
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
    // 학습 주석: direction은 agent가 task를 새로 제안하거나 queue 작업을 해석할 때 쓰는 장기 목표 context입니다.
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
    // 학습 주석: queue가 비었을 때 agent가 follow-up을 만들지, 멈출지 판단하는 정책만 별도 section으로 둡니다.
    let mut lines = vec![format!("policy={}", directions.queue_idle.policy.label())];
    if let Some(prompt_path) = trimmed_non_empty(directions.queue_idle.prompt_path.as_str()) {
        lines.push(format!("prompt_path={prompt_path}"));
    }
    lines
}

fn queue_context_lines(queue_projection: &PriorityQueueProjection) -> Vec<String> {
    // 학습 주석: queue projection은 이미 domain에서 rank가 계산된 view라 prompt builder는 순서를 재해석하지 않습니다.
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

    // 학습 주석: active task는 긴 backlog일 수 있으므로 상위 몇 개와 전체 개수만 함께 노출합니다.
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

    // 학습 주석: proposed task는 자동 실행 대상이 아니라 promote/queue intent를 기다리는 후보입니다.
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

    // 학습 주석: skipped task는 왜 제외됐는지 알려야 하지만 prompt budget 보호를 위해 일부만 보여 줍니다.
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
    // 학습 주석: 한 줄 포맷은 next/active/proposed task가 같은 prompt schema를 공유하게 합니다.
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
    // 학습 주석: follow-up 제안은 곧바로 실행하지 않고 task authority를 거쳐 proposed task로 남겨 중복 생성을 막습니다.
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
