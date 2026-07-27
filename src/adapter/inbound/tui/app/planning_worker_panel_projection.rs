use crate::domain::planning::PlanningWorkerPanelState;

/*
 * The planning worker panel is a read-only TUI projection of Core lifecycle
 * events. Keeping the state private prevents shell intents and render code from
 * optimistically rewriting semantic worker history before Core accepts a
 * transition.
 */
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct CorePlanningWorkerPanelProjection {
    current: PlanningWorkerPanelState,
}

impl CorePlanningWorkerPanelProjection {
    pub(super) fn current(&self) -> &PlanningWorkerPanelState {
        &self.current
    }

    pub(super) fn apply_started(&mut self, state: PlanningWorkerPanelState) {
        self.current = state;
    }

    pub(super) fn apply_completed(&mut self, state: PlanningWorkerPanelState) {
        self.current = state;
    }

    pub(super) fn reset_for_conversation_lifecycle(&mut self) {
        self.current = PlanningWorkerPanelState::default();
    }

    #[cfg(test)]
    pub(super) fn replace_for_test(&mut self, state: PlanningWorkerPanelState) {
        self.current = state;
    }
}
