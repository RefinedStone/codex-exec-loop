use super::reconciliation::PlanningExecutionSnapshot;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlanningQueueProjectionAction {
    RebuiltFromAcceptedPlanning,
    RestoredFromExecutionSnapshot,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct QueueProjectionRecoveryOutcome {
    pub(super) action: Option<PlanningQueueProjectionAction>,
    pub(super) notices: Vec<String>,
}

pub(super) fn recover_queue_projection(
    current_queue_snapshot_json: Option<&str>,
    execution_snapshot: &PlanningExecutionSnapshot,
) -> QueueProjectionRecoveryOutcome {
    if current_queue_snapshot_json == execution_snapshot.queue_snapshot_json.as_deref() {
        return QueueProjectionRecoveryOutcome::default();
    }

    QueueProjectionRecoveryOutcome {
        action: Some(PlanningQueueProjectionAction::RestoredFromExecutionSnapshot),
        notices: vec![
            "planning reconciliation restored queue.snapshot.json to the last accepted state"
                .to_string(),
        ],
    }
}
