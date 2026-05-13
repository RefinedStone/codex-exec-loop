use crate::application::service::planning::{
    PlanningApplicationProjection, PlanningRuntimeProjection, PlanningRuntimeWorkspaceStatus,
};

// Derive the execution-level substate that sits beside the broader workspace lifecycle label.
pub(super) fn plan_runtime_substate_label(
    runtime_projection: &PlanningRuntimeProjection,
) -> &'static str {
    let application_projection =
        PlanningApplicationProjection::from_runtime_projection(runtime_projection);
    plan_runtime_substate_label_from_projection(&application_projection)
}

fn plan_runtime_substate_label_from_projection(
    projection: &PlanningApplicationProjection,
) -> &'static str {
    if projection.workspace_status == PlanningRuntimeWorkspaceStatus::Invalid {
        "invalid"
    // A pause reason suppresses automatic continuation even when queue work exists, so it outranks queue readiness.
    } else if projection.auto_follow_paused {
        "paused"
    } else if projection.workspace_status == PlanningRuntimeWorkspaceStatus::ReadyWithTask {
        "ready"
    } else {
        "idle"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::planning::{PriorityQueueTask, TaskStatus};

    #[test]
    fn plan_substate_projects_invalid_runtime_projection_through_application_projection() {
        let runtime_projection = PlanningRuntimeProjection::invalid("planning validation failed");

        assert_eq!(plan_runtime_substate_label(&runtime_projection), "invalid");
    }

    #[test]
    fn plan_substate_keeps_pause_ahead_of_ready_queue() {
        let runtime_projection = PlanningRuntimeProjection::ready(
            "Planning Context".to_string(),
            "queue head ready".to_string(),
            Some(queue_task("task-1", "Ready task")),
        )
        .with_auto_follow_pause_reason("queue head repeated");
        let projection =
            PlanningApplicationProjection::from_runtime_projection(&runtime_projection);

        assert_eq!(
            plan_runtime_substate_label_from_projection(&projection),
            "paused"
        );
    }

    #[test]
    fn plan_substate_distinguishes_ready_and_idle_queue_state() {
        let ready = PlanningApplicationProjection::from_runtime_projection(
            &PlanningRuntimeProjection::ready(
                "Planning Context".to_string(),
                "queue head ready".to_string(),
                Some(queue_task("task-1", "Ready task")),
            ),
        );
        let idle = PlanningApplicationProjection::from_runtime_projection(
            &PlanningRuntimeProjection::ready(
                "Planning Context".to_string(),
                "no executable tasks".to_string(),
                None,
            ),
        );

        assert_eq!(plan_runtime_substate_label_from_projection(&ready), "ready");
        assert_eq!(plan_runtime_substate_label_from_projection(&idle), "idle");
    }

    fn queue_task(task_id: &str, task_title: &str) -> PriorityQueueTask {
        PriorityQueueTask {
            rank: 1,
            task_id: task_id.to_string(),
            direction_id: "direction-a".to_string(),
            direction_title: "Direction A".to_string(),
            task_title: task_title.to_string(),
            status: TaskStatus::Ready,
            combined_priority: 90,
            updated_at: "2026-05-08T00:00:00Z".to_string(),
            rank_reasons: vec!["priority".to_string()],
        }
    }
}
