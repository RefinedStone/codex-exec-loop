use crate::application::service::planning::PlanningRuntimeSnapshot;
use crate::domain::operator_alert::OperatorAlert;
use crate::domain::parallel_mode::ParallelModePostTurnQueueSignal;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PostTurnAutoFollowStopReason {
    PlanningQueueDrained,
    ParallelSessionCompleted,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PostTurnDecision {
    pub(crate) auto_follow_stop_reason: PostTurnAutoFollowStopReason,
    pub(crate) parallel_queue_signal: Option<ParallelModePostTurnQueueSignal>,
    pub(crate) operator_alerts: Vec<OperatorAlert>,
}

pub(crate) fn decide_parallel_official_completion_post_turn(
    runtime_snapshot: &PlanningRuntimeSnapshot,
) -> PostTurnDecision {
    if runtime_snapshot.queue_is_drained() {
        return PostTurnDecision {
            auto_follow_stop_reason: PostTurnAutoFollowStopReason::PlanningQueueDrained,
            parallel_queue_signal: None,
            operator_alerts: vec![OperatorAlert::planning_queue_drained()],
        };
    }

    PostTurnDecision {
        auto_follow_stop_reason: PostTurnAutoFollowStopReason::ParallelSessionCompleted,
        parallel_queue_signal: Some(ParallelModePostTurnQueueSignal::ParallelCompletionFinalized),
        operator_alerts: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::planning::{PriorityQueueProjection, PriorityQueueSkippedTask, TaskStatus};

    #[test]
    fn parallel_official_completion_reports_drained_queue_as_alert_without_dispatch_signal() {
        let runtime_snapshot = PlanningRuntimeSnapshot::ready_with_queue_projection(
            "Planning Context".to_string(),
            "queue idle: no executable planning task".to_string(),
            None,
            None,
            PriorityQueueProjection {
                next_task: None,
                active_tasks: Vec::new(),
                proposed_tasks: Vec::new(),
                skipped_tasks: vec![PriorityQueueSkippedTask {
                    task_id: "done-task".to_string(),
                    task_title: "Finished parallel task".to_string(),
                    direction_id: "general-workstream".to_string(),
                    status: TaskStatus::Done,
                    reason: "status done is not executable".to_string(),
                }],
            },
        );

        let decision = decide_parallel_official_completion_post_turn(&runtime_snapshot);

        assert_eq!(
            decision.auto_follow_stop_reason,
            PostTurnAutoFollowStopReason::PlanningQueueDrained
        );
        assert_eq!(decision.parallel_queue_signal, None);
        assert_eq!(decision.operator_alerts.len(), 1);
        assert_eq!(
            decision.operator_alerts[0].title,
            "All planning tasks complete"
        );
    }

    #[test]
    fn parallel_official_completion_keeps_supervisor_signal_when_queue_may_have_work() {
        let runtime_snapshot = PlanningRuntimeSnapshot::invalid("planning still blocked");

        let decision = decide_parallel_official_completion_post_turn(&runtime_snapshot);

        assert_eq!(
            decision.auto_follow_stop_reason,
            PostTurnAutoFollowStopReason::ParallelSessionCompleted
        );
        assert_eq!(
            decision.parallel_queue_signal,
            Some(ParallelModePostTurnQueueSignal::ParallelCompletionFinalized)
        );
        assert!(decision.operator_alerts.is_empty());
    }
}
