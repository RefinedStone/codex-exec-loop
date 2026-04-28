#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlanningQueueProjectionAction {
    RebuiltFromAcceptedPlanning,
    RestoredFromExecutionSnapshot,
}
