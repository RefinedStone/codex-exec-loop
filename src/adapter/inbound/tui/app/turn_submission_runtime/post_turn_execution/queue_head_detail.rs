use crate::application::service::planning::PlanningRuntimeSnapshot;
use crate::application::service::planning::PlanningTaskHandoff;

pub(super) fn repeated_queue_head_detail(
    previous_handoff: Option<&PlanningTaskHandoff>,
    previous_snapshot: &PlanningRuntimeSnapshot,
    snapshot: &PlanningRuntimeSnapshot,
) -> Option<String> {
    let previous_handoff = previous_handoff?;
    let queue_head = snapshot.queue_head()?;
    if queue_head.task_id.trim() != previous_handoff.task_id.trim() {
        return None;
    }

    let unchanged = queue_head.task_title.trim() == previous_handoff.task_title.trim()
        && queue_head.direction_id.trim() == previous_handoff.direction_id.trim()
        && queue_head.combined_priority == previous_handoff.combined_priority
        && queue_head.updated_at.trim() == previous_handoff.updated_at.trim()
        && queue_head.status.label() == previous_handoff.status_label;
    if !unchanged {
        return None;
    }

    let queue_head_task_unchanged = match (
        previous_snapshot.queue_head_task_signature(),
        snapshot.queue_head_task_signature(),
    ) {
        (Some(previous), Some(current)) => previous == current,
        (None, None) => true,
        _ => false,
    };
    if !queue_head_task_unchanged {
        return None;
    }

    Some(format!(
        "planner refresh kept the previously handed-off task unchanged as the queue head; unrelated ledger edits do not count as queue advancement: {}",
        previous_handoff.task_title
    ))
}

#[cfg(test)]
mod tests {
    use crate::application::service::planning::PlanningRuntimeSnapshot;
    use crate::application::service::planning::PlanningTaskHandoff;
    use crate::domain::planning::{PriorityQueueTask, TaskStatus};

    use super::repeated_queue_head_detail;

    fn sample_queue_head() -> PriorityQueueTask {
        PriorityQueueTask {
            rank: 1,
            task_id: "task-1".to_string(),
            direction_id: "direction-1".to_string(),
            direction_title: "Direction".to_string(),
            task_title: "Queue head".to_string(),
            status: TaskStatus::Ready,
            combined_priority: 80,
            updated_at: "2026-04-23T00:00:00Z".to_string(),
            rank_reasons: vec!["ready".to_string()],
        }
    }

    fn sample_handoff() -> PlanningTaskHandoff {
        PlanningTaskHandoff {
            task_id: "task-1".to_string(),
            task_title: "Queue head".to_string(),
            direction_id: "direction-1".to_string(),
            combined_priority: 80,
            updated_at: "2026-04-23T00:00:00Z".to_string(),
            status_label: "ready".to_string(),
        }
    }

    fn snapshot_with_signature(signature: Option<u64>) -> PlanningRuntimeSnapshot {
        PlanningRuntimeSnapshot::ready(
            "prompt".to_string(),
            "summary".to_string(),
            Some(sample_queue_head()),
        )
        .with_test_signatures(None, signature)
    }

    #[test]
    fn repeated_queue_head_detail_treats_missing_and_present_signatures_as_changed() {
        let detail = repeated_queue_head_detail(
            Some(&sample_handoff()),
            &snapshot_with_signature(None),
            &snapshot_with_signature(Some(7)),
        );

        assert!(detail.is_none());
    }

    #[test]
    fn repeated_queue_head_detail_accepts_both_missing_signatures_as_unchanged() {
        let detail = repeated_queue_head_detail(
            Some(&sample_handoff()),
            &snapshot_with_signature(None),
            &snapshot_with_signature(None),
        );

        assert!(detail.is_some());
    }
}
