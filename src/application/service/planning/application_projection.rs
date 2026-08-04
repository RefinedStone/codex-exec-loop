use crate::application::port::inbound::planning_projection_port::{
    PlanningApplicationProjection, PlanningApplicationQueueTask, PlanningApplicationSkippedTask,
    PlanningProjectionPort,
};
use crate::application::service::planning::runtime::facade::PlanningRuntimeFacadeService;
use crate::domain::planning::{
    PriorityQueueSkippedTask, PriorityQueueTask, RuntimeProjection as PlanningRuntimeProjection,
};

/*
 * PlanningApplicationProjection은 inbound surface가 planning runtime facts를 읽기 위한
 * 공통 read model이다. 지금은 PlanningRuntimeProjection에서 시작하지만, 목표는 admin/TUI/CLI/Telegram이
 * queue/proposal/blocked 상태를 각자 다시 해석하지 않고 이 타입을 통해 같은 사실을 보는 것이다.
 */
impl PlanningApplicationProjection {
    pub fn from_runtime_projection(runtime_projection: &PlanningRuntimeProjection) -> Self {
        /*
         * Queue projection이 있으면 그 structured lane을 source로 삼는다. projection이 없는
         * old/invalid projection은 accessor만 사용해 status와 optional queue head를 보존한다.
         */
        let queue_projection = runtime_projection.queue_projection();
        let queue_head = runtime_projection
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
            workspace_present: runtime_projection.workspace_present(),
            workspace_status: runtime_projection.workspace_status(),
            planning_revision: runtime_projection.planning_revision(),
            task_authority_signature: runtime_projection.task_authority_signature(),
            queue_head_task_signature: runtime_projection.queue_head_task_signature(),
            auto_follow_paused: runtime_projection.auto_follow_pause_reason().is_some(),
            status_label: runtime_projection.preview_status_label().to_string(),
            status_detail: runtime_projection.preview_detail().map(str::to_string),
            queue_summary: runtime_projection.queue_summary().map(str::to_string),
            proposal_summary: runtime_projection.proposal_summary().map(str::to_string),
            queue_idle_policy: runtime_projection.queue_idle_policy(),
            queue_idle_prompt_path: runtime_projection
                .queue_idle_prompt_path()
                .map(str::to_string),
            has_structured_queue_projection: queue_projection.is_some(),
            queue_head,
            visible_tasks,
            proposed_tasks,
            skipped_tasks,
        }
    }
}

impl PlanningProjectionPort for PlanningRuntimeFacadeService {
    fn load_application_projection(
        &self,
        workspace_directory: &str,
    ) -> anyhow::Result<PlanningApplicationProjection> {
        self.inspect_runtime_projection(workspace_directory)
            .map(|projection| PlanningApplicationProjection::from_runtime_projection(&projection))
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
    use crate::application::service::planning::PlanningRuntimeProjection;
    use crate::domain::planning::{
        PriorityQueueProjection, PriorityQueueSkippedTask, PriorityQueueTask, QueueIdlePolicy,
        TaskStatus,
    };

    #[test]
    fn projection_preserves_runtime_status_and_structured_queue_lanes() {
        let head = queue_task(1, "task-1", "Current task", TaskStatus::Ready);
        let runtime_projection = PlanningRuntimeProjection::ready_with_queue_projection(
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
        .with_planning_revision(Some(11))
        .with_test_signatures(Some(42), Some(7));

        let projection =
            PlanningApplicationProjection::from_runtime_projection(&runtime_projection);

        assert!(projection.workspace_present);
        assert_eq!(projection.planning_revision, Some(11));
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
    fn projection_preserves_invalid_projection_without_queue_lanes() {
        let runtime_projection = PlanningRuntimeProjection::invalid(
            "planning validation failed: task authority is unavailable".to_string(),
        );
        let projection =
            PlanningApplicationProjection::from_runtime_projection(&runtime_projection);

        assert!(projection.workspace_present);
        assert_eq!(projection.status_label, "blocked");
        assert_eq!(
            projection.status_detail.as_deref(),
            Some("planning validation failed: task authority is unavailable")
        );
        assert_eq!(projection.task_authority_signature, None);
        assert_eq!(projection.planning_revision, None);
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
