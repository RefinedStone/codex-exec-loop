use crate::application::service::planning::{
    PlanningRuntimeSnapshot, PlanningRuntimeWorkspaceStatus,
};
use crate::domain::planning::{
    PriorityQueueSkippedTask, PriorityQueueTask, QueueIdlePolicy, TaskStatus,
};

/*
 * PlanningApplicationProjection은 inbound surface가 planning runtime facts를 읽기 위한
 * 공통 read model이다. 지금은 PlanningRuntimeSnapshot에서 시작하지만, 목표는 admin/TUI/CLI/Telegram이
 * queue/proposal/blocked 상태를 각자 다시 해석하지 않고 이 타입을 통해 같은 사실을 보는 것이다.
 */
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanningApplicationProjection {
    pub workspace_present: bool,
    pub workspace_status: PlanningRuntimeWorkspaceStatus,
    pub task_authority_signature: Option<u64>,
    pub queue_head_task_signature: Option<u64>,
    pub auto_follow_paused: bool,
    pub status_label: String,
    pub status_detail: Option<String>,
    pub queue_summary: Option<String>,
    pub proposal_summary: Option<String>,
    pub queue_idle_policy: QueueIdlePolicy,
    pub queue_idle_prompt_path: Option<String>,
    pub has_structured_queue_projection: bool,
    pub queue_head: Option<PlanningApplicationQueueTask>,
    pub visible_tasks: Vec<PlanningApplicationQueueTask>,
    pub proposed_tasks: Vec<PlanningApplicationQueueTask>,
    pub skipped_tasks: Vec<PlanningApplicationSkippedTask>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanningApplicationQueueTask {
    pub rank: usize,
    pub task_id: String,
    pub task_title: String,
    pub direction_id: String,
    pub direction_title: String,
    pub status: TaskStatus,
    pub status_label: String,
    pub combined_priority: i32,
    pub updated_at: String,
    pub rank_reasons: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanningApplicationSkippedTask {
    pub task_id: String,
    pub task_title: String,
    pub direction_id: String,
    pub status: TaskStatus,
    pub status_label: String,
    pub reason: String,
}

impl PlanningApplicationProjection {
    pub fn from_runtime_snapshot(snapshot: &PlanningRuntimeSnapshot) -> Self {
        /*
         * Queue projection이 있으면 그 structured lane을 source로 삼는다. projection이 없는
         * old/invalid snapshot은 snapshot accessor만 사용해 status와 optional queue head를 보존한다.
         */
        let queue_projection = snapshot.queue_projection();
        let queue_head = snapshot
            .queue_head()
            .map(PlanningApplicationQueueTask::from);
        let visible_tasks = queue_projection
            .map(|projection| {
                projection
                    .active_tasks
                    .iter()
                    .map(PlanningApplicationQueueTask::from)
                    .collect()
            })
            .unwrap_or_else(|| queue_head.iter().cloned().collect());
        let proposed_tasks = queue_projection
            .map(|projection| {
                projection
                    .proposed_tasks
                    .iter()
                    .map(PlanningApplicationQueueTask::from)
                    .collect()
            })
            .unwrap_or_default();
        let skipped_tasks = queue_projection
            .map(|projection| {
                projection
                    .skipped_tasks
                    .iter()
                    .map(PlanningApplicationSkippedTask::from)
                    .collect()
            })
            .unwrap_or_default();

        Self {
            workspace_present: snapshot.workspace_present(),
            workspace_status: snapshot.workspace_status(),
            task_authority_signature: snapshot.task_authority_signature(),
            queue_head_task_signature: snapshot.queue_head_task_signature(),
            auto_follow_paused: snapshot.auto_follow_pause_reason().is_some(),
            status_label: snapshot.preview_status_label().to_string(),
            status_detail: snapshot.preview_detail().map(str::to_string),
            queue_summary: snapshot.queue_summary().map(str::to_string),
            proposal_summary: snapshot.proposal_summary().map(str::to_string),
            queue_idle_policy: snapshot.queue_idle_policy(),
            queue_idle_prompt_path: snapshot.queue_idle_prompt_path().map(str::to_string),
            has_structured_queue_projection: queue_projection.is_some(),
            queue_head,
            visible_tasks,
            proposed_tasks,
            skipped_tasks,
        }
    }
}

impl From<&PriorityQueueTask> for PlanningApplicationQueueTask {
    fn from(task: &PriorityQueueTask) -> Self {
        Self {
            rank: task.rank,
            task_id: task.task_id.clone(),
            task_title: task.task_title.clone(),
            direction_id: task.direction_id.clone(),
            direction_title: task.direction_title.clone(),
            status: task.status,
            status_label: task.status.label().to_string(),
            combined_priority: task.combined_priority,
            updated_at: task.updated_at.clone(),
            rank_reasons: task.rank_reasons.clone(),
        }
    }
}

impl From<&PriorityQueueSkippedTask> for PlanningApplicationSkippedTask {
    fn from(task: &PriorityQueueSkippedTask) -> Self {
        Self {
            task_id: task.task_id.clone(),
            task_title: task.task_title.clone(),
            direction_id: task.direction_id.clone(),
            status: task.status,
            status_label: task.status.label().to_string(),
            reason: task.reason.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::PlanningApplicationProjection;
    use crate::application::service::planning::PlanningRuntimeSnapshot;
    use crate::domain::planning::{
        PriorityQueueProjection, PriorityQueueSkippedTask, PriorityQueueTask, QueueIdlePolicy,
        TaskStatus,
    };

    #[test]
    fn projection_preserves_runtime_status_and_structured_queue_lanes() {
        let head = queue_task(1, "task-1", "Current task", TaskStatus::Ready);
        let snapshot = PlanningRuntimeSnapshot::ready_with_queue_projection(
            "Planning Context".to_string(),
            "queue head: rank 1 / task-1 / Current task / priority 90".to_string(),
            Some("2 promotable follow-up proposals available".to_string()),
            Some(head.clone()),
            PriorityQueueProjection {
                next_task: Some(head),
                active_tasks: vec![
                    queue_task(1, "task-1", "Current task", TaskStatus::Ready),
                    queue_task(2, "task-2", "Next task", TaskStatus::Ready),
                ],
                proposed_tasks: vec![queue_task(
                    1,
                    "proposal-1",
                    "Candidate task",
                    TaskStatus::Proposed,
                )],
                skipped_tasks: vec![skipped_task(
                    "blocked-1",
                    "Blocked task",
                    "dependency-open(ready)",
                )],
            },
        )
        .with_queue_idle_policy(
            QueueIdlePolicy::ReviewAndEnqueue,
            Some(".codex-exec-loop/planning/prompts/queue-idle-review.md".to_string()),
        )
        .with_test_signatures(Some(42), Some(7));

        let projection = PlanningApplicationProjection::from_runtime_snapshot(&snapshot);

        assert!(projection.workspace_present);
        assert_eq!(projection.task_authority_signature, Some(42));
        assert_eq!(projection.queue_head_task_signature, Some(7));
        assert_eq!(projection.status_label, "ready");
        assert_eq!(
            projection.status_detail.as_deref(),
            Some("queue head: rank 1 / task-1 / Current task / priority 90")
        );
        assert_eq!(
            projection.queue_idle_policy,
            QueueIdlePolicy::ReviewAndEnqueue
        );
        assert!(!projection.auto_follow_paused);
        assert!(projection.has_structured_queue_projection);
        assert_eq!(
            projection
                .queue_head
                .as_ref()
                .map(|task| task.task_id.as_str()),
            Some("task-1")
        );
        assert_eq!(
            projection
                .visible_tasks
                .iter()
                .map(|task| task.task_id.as_str())
                .collect::<Vec<_>>(),
            vec!["task-1", "task-2"]
        );
        assert_eq!(projection.proposed_tasks[0].status_label, "proposed");
        assert_eq!(projection.skipped_tasks[0].reason, "dependency-open(ready)");
    }

    #[test]
    fn projection_preserves_invalid_snapshot_without_queue_lanes() {
        let snapshot = PlanningRuntimeSnapshot::invalid(
            "planning validation failed: task authority is unavailable".to_string(),
        );
        let projection = PlanningApplicationProjection::from_runtime_snapshot(&snapshot);

        assert!(projection.workspace_present);
        assert_eq!(projection.status_label, "blocked");
        assert_eq!(
            projection.status_detail.as_deref(),
            Some("planning validation failed: task authority is unavailable")
        );
        assert_eq!(projection.task_authority_signature, None);
        assert_eq!(projection.queue_head_task_signature, None);
        assert!(!projection.auto_follow_paused);
        assert!(projection.queue_head.is_none());
        assert!(!projection.has_structured_queue_projection);
        assert!(projection.visible_tasks.is_empty());
        assert!(projection.proposed_tasks.is_empty());
        assert!(projection.skipped_tasks.is_empty());
    }

    fn queue_task(
        rank: usize,
        task_id: &str,
        task_title: &str,
        status: TaskStatus,
    ) -> PriorityQueueTask {
        PriorityQueueTask {
            rank,
            task_id: task_id.to_string(),
            direction_id: "direction-a".to_string(),
            direction_title: "Direction A".to_string(),
            task_title: task_title.to_string(),
            status,
            combined_priority: 90 - rank as i32,
            updated_at: "2026-05-08T00:00:00Z".to_string(),
            rank_reasons: vec![format!("rank={rank}")],
        }
    }

    fn skipped_task(task_id: &str, task_title: &str, reason: &str) -> PriorityQueueSkippedTask {
        PriorityQueueSkippedTask {
            task_id: task_id.to_string(),
            task_title: task_title.to_string(),
            direction_id: "direction-a".to_string(),
            status: TaskStatus::Blocked,
            reason: reason.to_string(),
        }
    }
}
