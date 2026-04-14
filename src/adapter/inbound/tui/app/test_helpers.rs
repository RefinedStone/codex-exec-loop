use crate::application::service::planning::PlanningRuntimeSnapshot;
use crate::domain::planning::{
    PriorityQueueSkippedTask, PriorityQueueSnapshot, PriorityQueueTask, TaskStatus,
};

pub(crate) fn sample_queue_head() -> PriorityQueueTask {
    PriorityQueueTask {
        rank: 1,
        task_id: "task-1".to_string(),
        direction_id: "general-workstream".to_string(),
        direction_title: "General workstream".to_string(),
        task_title: "Implement shell planning status".to_string(),
        status: TaskStatus::Ready,
        combined_priority: 10,
        updated_at: "2026-04-10T00:00:00Z".to_string(),
        rank_reasons: vec!["status=ready".to_string()],
    }
}

pub(crate) fn sample_planning_runtime_snapshot(
    prompt_fragment: &str,
    queue_summary: &str,
) -> PlanningRuntimeSnapshot {
    let queue_head = sample_queue_head();
    PlanningRuntimeSnapshot::ready_with_queue_snapshot(
        prompt_fragment.to_string(),
        queue_summary.to_string(),
        None,
        Some(queue_head.clone()),
        PriorityQueueSnapshot {
            next_task: Some(queue_head.clone()),
            active_tasks: vec![
                queue_head,
                PriorityQueueTask {
                    rank: 2,
                    task_id: "task-2".to_string(),
                    direction_id: "general-workstream".to_string(),
                    direction_title: "General workstream".to_string(),
                    task_title: "Trim legacy shell code".to_string(),
                    status: TaskStatus::Ready,
                    combined_priority: 8,
                    updated_at: "2026-04-10T01:00:00Z".to_string(),
                    rank_reasons: vec!["status=ready".to_string()],
                },
            ],
            proposed_tasks: Vec::new(),
            skipped_tasks: vec![PriorityQueueSkippedTask {
                task_id: "task-blocked-1".to_string(),
                task_title: "Follow blocked review thread".to_string(),
                direction_id: "general-workstream".to_string(),
                status: TaskStatus::Blocked,
                reason: "blocked by tasks: task-2(in_progress)".to_string(),
            }],
        },
    )
}

pub(crate) fn sample_proposal_only_planning_runtime_snapshot(
    prompt_fragment: &str,
    queue_summary: &str,
    proposal_summary: &str,
) -> PlanningRuntimeSnapshot {
    PlanningRuntimeSnapshot::ready_with_queue_snapshot(
        prompt_fragment.to_string(),
        queue_summary.to_string(),
        Some(proposal_summary.to_string()),
        None,
        PriorityQueueSnapshot {
            next_task: None,
            active_tasks: Vec::new(),
            proposed_tasks: vec![PriorityQueueTask {
                rank: 1,
                task_id: "proposal-1".to_string(),
                direction_id: "general-workstream".to_string(),
                direction_title: "General workstream".to_string(),
                task_title: "Draft a queue inspection overlay".to_string(),
                status: TaskStatus::Proposed,
                combined_priority: 7,
                updated_at: "2026-04-10T02:00:00Z".to_string(),
                rank_reasons: vec!["combined_priority=7".to_string()],
            }],
            skipped_tasks: Vec::new(),
        },
    )
}
