use crate::application::service::planning_prompt_service::PlanningRuntimeSnapshot;
use crate::domain::planning::{PriorityQueueTask, TaskStatus};

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
    PlanningRuntimeSnapshot::ready(
        prompt_fragment.to_string(),
        queue_summary.to_string(),
        Some(sample_queue_head()),
    )
}

pub(crate) fn sample_proposal_only_planning_runtime_snapshot(
    prompt_fragment: &str,
    queue_summary: &str,
    proposal_summary: &str,
) -> PlanningRuntimeSnapshot {
    PlanningRuntimeSnapshot::ready_with_details(
        prompt_fragment.to_string(),
        queue_summary.to_string(),
        Some(proposal_summary.to_string()),
        None,
    )
}
