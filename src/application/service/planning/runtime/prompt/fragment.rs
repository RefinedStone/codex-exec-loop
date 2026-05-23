use crate::application::service::planning::shared::prompt_sections::runtime_task_authority_contract_rules;
use crate::application::service::prompt_component::PromptDocument;
use crate::domain::planning::{
    DirectionCatalogDocument, DirectionState, PriorityQueueProjection, PriorityQueueTask,
};

// prompt fragment는 queue 전체를 덤프하지 않고, agent가 다음 결정을 내릴 만큼만 잘라 넣는다.
// 이 상한들은 prompt budget을 보호하면서도 next/active/proposed/skipped 상태의 균형을 유지하는 contract다.
const MAX_VISIBLE_QUEUE_TASKS: usize = 5;
const MAX_SKIPPED_QUEUE_TASKS: usize = 3;
const MAX_VISIBLE_PROPOSED_TASKS: usize = 3;

pub(super) fn build_prompt_fragment(
    directions: &DirectionCatalogDocument,
    queue_projection: &PriorityQueueProjection,
    result_output_markdown: &str,
) -> String {
    /*
     * Runtime prompt fragment는 planning runtime이 현재 방향과 queue 상태를 Codex turn에 주입하는 경계다.
     * section 이름을 고정해 두면 downstream prompt tests가 큰 markdown blob 대신 의미 단위로 회귀를 잡을 수 있다.
     * result-output은 operator가 작성한 free-form 지시라 optional text로 붙이고, 나머지 queue/contract 정보는
     * PromptDocument의 section schema로 감싸 worker prompt가 안정적으로 파싱할 수 있게 한다.
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
    // direction은 agent가 task를 새로 제안하거나 queue 작업을 해석할 때 쓰는 장기 목표 context다.
    // 각 direction을 여러 줄로 펼쳐 title/state, summary, 성공 기준, optional detail doc path를 같은 schema로 노출한다.
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
    // queue가 비었을 때 agent가 follow-up을 만들지, 멈출지 판단하는 정책만 별도 section으로 둔다.
    // active queue 내용과 섞지 않아 "일이 없음"과 "다음 일을 제안해도 됨"을 worker가 분리해서 읽게 한다.
    let mut lines = vec![format!("policy={}", directions.queue_idle.policy.label())];
    if let Some(prompt_path) = trimmed_non_empty(directions.queue_idle.prompt_path.as_str()) {
        lines.push(format!("prompt_path={prompt_path}"));
    }
    lines
}

fn queue_context_lines(queue_projection: &PriorityQueueProjection) -> Vec<String> {
    // queue projection은 이미 domain에서 rank가 계산된 view라 prompt builder는 순서를 재해석하지 않는다.
    // 여기서는 queue head(next_task schema), active backlog, proposed 후보, skipped 사유를 prompt line으로 직렬화하는 역할만 맡는다.
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

    // active task는 긴 backlog일 수 있으므로 상위 몇 개와 전체 개수만 함께 노출한다.
    // 전체 개수는 backlog 규모를 알려 주고, visible slice는 worker가 바로 이어갈 후보를 볼 수 있게 한다.
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

    // proposed task는 자동 실행 대상이 아니라 promote/queue intent를 기다리는 후보다.
    // active task와 같은 line schema를 쓰되 section 이름으로 아직 실행 대기 상태가 아님을 구분한다.
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

    // skipped task는 왜 제외됐는지 알려야 하지만 prompt budget 보호를 위해 일부만 보여 준다.
    // reason을 함께 싣는 이유는 worker가 누락된 dependency나 paused direction을 새 task로 중복 제안하지 않게 하기 위해서다.
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
    // 한 줄 포맷은 next/active/proposed task가 같은 prompt schema를 공유하게 한다.
    // rank와 combined_priority는 domain queue가 계산한 결과라 prompt 쪽에서 다시 설명하지 않고 그대로 전달한다.
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
    // follow-up 제안은 곧바로 실행하지 않고 task authority를 거쳐 proposed task로 남겨 중복 생성을 막는다.
    // 이 문구들은 agent 행동 계약이라 영문 rule 자체는 tests와 prompt contract를 고려해 그대로 유지한다.
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

#[cfg(test)]
mod tests {
    use super::build_prompt_fragment;
    use crate::domain::planning::{
        DirectionCatalogDocument, DirectionDefinition, DirectionState, PLANNING_FORMAT_VERSION,
        PriorityQueueProjection, PriorityQueueSkippedTask, PriorityQueueTask, QueueIdleConfig,
        QueueIdlePolicy, TaskStatus,
    };

    fn direction(
        id: &str,
        title: &str,
        state: DirectionState,
        detail_doc_path: &str,
    ) -> DirectionDefinition {
        DirectionDefinition {
            id: id.to_string(),
            title: title.to_string(),
            summary: format!("{title} summary"),
            success_criteria: vec!["done".to_string()],
            scope_hints: vec!["scope-a".to_string()],
            detail_doc_path: detail_doc_path.to_string(),
            state,
        }
    }

    fn queue_task(
        rank: usize,
        task_id: &str,
        title: &str,
        status: TaskStatus,
    ) -> PriorityQueueTask {
        PriorityQueueTask {
            rank,
            task_id: task_id.to_string(),
            direction_id: "general-workstream".to_string(),
            direction_title: "General".to_string(),
            task_title: title.to_string(),
            status,
            combined_priority: 90 - rank as i32,
            updated_at: "2026-05-23T00:00:00Z".to_string(),
            rank_reasons: vec![format!("rank reason {rank}")],
        }
    }

    fn skipped_task(rank: usize) -> PriorityQueueSkippedTask {
        PriorityQueueSkippedTask {
            task_id: format!("skipped-{rank}"),
            task_title: format!("Skipped {rank}"),
            direction_id: "paused-workstream".to_string(),
            status: TaskStatus::Blocked,
            reason: format!("reason {rank}"),
        }
    }

    #[test]
    fn prompt_fragment_renders_direction_states_queue_idle_and_visible_queue_edges() {
        let directions = DirectionCatalogDocument {
            version: PLANNING_FORMAT_VERSION,
            queue_idle: QueueIdleConfig {
                policy: QueueIdlePolicy::ReviewAndEnqueue,
                prompt_path: " .codex-exec-loop/planning/prompts/queue-idle.md ".to_string(),
            },
            directions: vec![
                direction(
                    "general-workstream",
                    "General",
                    DirectionState::Active,
                    " .codex-exec-loop/planning/directions/general.md ",
                ),
                direction("paused-workstream", "Paused", DirectionState::Paused, ""),
                direction("done-workstream", "Done", DirectionState::Done, "   "),
            ],
        };
        let queue_projection = PriorityQueueProjection {
            next_task: Some(queue_task(1, "task-1", "Next task", TaskStatus::Ready)),
            active_tasks: (1..=6)
                .map(|rank| {
                    queue_task(
                        rank,
                        &format!("task-{rank}"),
                        &format!("Task {rank}"),
                        TaskStatus::Ready,
                    )
                })
                .collect(),
            proposed_tasks: (1..=4)
                .map(|rank| {
                    queue_task(
                        rank,
                        &format!("proposal-{rank}"),
                        &format!("Proposal {rank}"),
                        TaskStatus::Proposed,
                    )
                })
                .collect(),
            skipped_tasks: (1..=4).map(skipped_task).collect(),
        };

        let fragment = build_prompt_fragment(
            &directions,
            &queue_projection,
            "# Result output\nKeep constraints.",
        );

        assert!(fragment.contains("state=active"));
        assert!(fragment.contains("state=paused"));
        assert!(fragment.contains("state=done"));
        assert!(
            fragment.contains("detail_doc_path=.codex-exec-loop/planning/directions/general.md")
        );
        assert_eq!(fragment.matches("detail_doc_path=").count(), 1);
        assert!(fragment.contains("policy=review_and_enqueue"));
        assert!(fragment.contains("prompt_path=.codex-exec-loop/planning/prompts/queue-idle.md"));
        assert!(fragment.contains(
            "next_task=rank 1; id=task-1; title=Next task; direction=general-workstream; status=ready; combined_priority=89"
        ));
        assert!(fragment.contains("next_task_rank_reasons=rank reason 1"));
        assert!(fragment.contains("visible_tasks=top 5 of 6"));
        assert!(fragment.contains("- rank 5; id=task-5; title=Task 5"));
        assert!(!fragment.contains("- rank 6; id=task-6; title=Task 6"));
        assert!(fragment.contains("proposed_tasks=top 3 of 4 promotable"));
        assert!(fragment.contains("- rank 3; id=proposal-3; title=Proposal 3"));
        assert!(!fragment.contains("- rank 4; id=proposal-4; title=Proposal 4"));
        assert!(fragment.contains("skipped_tasks=showing 3 of 4"));
        assert!(
            fragment.contains("- id=skipped-3; title=Skipped 3; direction=paused-workstream; status=blocked; reason=reason 3")
        );
        assert!(!fragment.contains("skipped-4"));
        assert!(fragment.contains("[result-output-prompt]\n# Result output\nKeep constraints."));
        assert!(fragment.contains("[follow-up-proposals]"));
    }

    #[test]
    fn prompt_fragment_renders_idle_queue_without_optional_sections() {
        let directions = DirectionCatalogDocument {
            version: PLANNING_FORMAT_VERSION,
            queue_idle: QueueIdleConfig::default(),
            directions: vec![direction(
                "general-workstream",
                "General",
                DirectionState::Active,
                "",
            )],
        };
        let queue_projection = PriorityQueueProjection {
            next_task: None,
            active_tasks: Vec::new(),
            proposed_tasks: Vec::new(),
            skipped_tasks: Vec::new(),
        };

        let fragment = build_prompt_fragment(&directions, &queue_projection, "   ");

        assert!(fragment.contains("policy=stop"));
        assert!(fragment.contains("next_task=none"));
        assert!(fragment.contains("visible_tasks=none"));
        assert!(!fragment.contains("prompt_path="));
        assert!(!fragment.contains("proposed_tasks="));
        assert!(!fragment.contains("skipped_tasks="));
        assert!(!fragment.contains("[result-output-prompt]"));
    }
}
