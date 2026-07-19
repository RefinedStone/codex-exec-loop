use crate::core::app::PlanningWorkspaceOperationCorrelation;

#[derive(Debug, Clone, PartialEq, Eq)]
struct PendingPlanningWorkspaceOperation {
    correlation: PlanningWorkspaceOperationCorrelation,
    presentation_revision: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum PlanningWorkspaceOperationUiSettlement {
    Applied,
    PresentationSuperseded,
    WorkspaceSuperseded,
    Rejected,
}

#[derive(Debug, Clone, Default)]
pub(super) struct PlanningWorkspaceOperationUiState {
    pending: Option<PendingPlanningWorkspaceOperation>,
}

impl PlanningWorkspaceOperationUiState {
    pub(super) fn begin(
        &mut self,
        correlation: PlanningWorkspaceOperationCorrelation,
        presentation_revision: u64,
    ) {
        self.pending = Some(PendingPlanningWorkspaceOperation {
            correlation,
            presentation_revision,
        });
    }

    pub(super) fn active_correlation(&self) -> Option<&PlanningWorkspaceOperationCorrelation> {
        self.pending.as_ref().map(|pending| &pending.correlation)
    }

    pub(super) fn settle(
        &mut self,
        correlation: &PlanningWorkspaceOperationCorrelation,
        current_workspace_directory: &str,
        current_presentation_revision: u64,
    ) -> PlanningWorkspaceOperationUiSettlement {
        if self
            .pending
            .as_ref()
            .is_none_or(|pending| pending.correlation != *correlation)
        {
            return PlanningWorkspaceOperationUiSettlement::Rejected;
        }
        let pending = self
            .pending
            .take()
            .expect("exact planning workspace operation must remain pending");
        if pending.correlation.workspace_directory != current_workspace_directory {
            return PlanningWorkspaceOperationUiSettlement::WorkspaceSuperseded;
        }
        if pending.presentation_revision != current_presentation_revision {
            return PlanningWorkspaceOperationUiSettlement::PresentationSuperseded;
        }
        PlanningWorkspaceOperationUiSettlement::Applied
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::app::PlanningWorkspaceResetTarget;

    fn correlation(
        generation: u64,
        workspace_directory: &str,
    ) -> PlanningWorkspaceOperationCorrelation {
        PlanningWorkspaceOperationCorrelation {
            generation,
            workspace_directory: workspace_directory.to_string(),
            reset_target: PlanningWorkspaceResetTarget::Queue,
        }
    }

    #[test]
    fn exact_completion_settles_once() {
        let mut state = PlanningWorkspaceOperationUiState::default();
        let reset = correlation(1, "/workspace");
        state.begin(reset.clone(), 7);

        assert_eq!(
            state.settle(&reset, "/workspace", 7),
            PlanningWorkspaceOperationUiSettlement::Applied
        );
        assert_eq!(
            state.settle(&reset, "/workspace", 7),
            PlanningWorkspaceOperationUiSettlement::Rejected
        );
    }

    #[test]
    fn stale_and_aba_completions_cannot_settle_a_new_generation() {
        let mut state = PlanningWorkspaceOperationUiState::default();
        let first = correlation(1, "/workspace");
        let second = correlation(2, "/workspace");
        state.begin(first.clone(), 3);

        assert_eq!(
            state.settle(&correlation(99, "/workspace"), "/workspace", 3),
            PlanningWorkspaceOperationUiSettlement::Rejected
        );
        assert_eq!(state.active_correlation(), Some(&first));
        assert_eq!(
            state.settle(&first, "/workspace", 3),
            PlanningWorkspaceOperationUiSettlement::Applied
        );

        state.begin(second.clone(), 4);
        assert_eq!(
            state.settle(&first, "/workspace", 4),
            PlanningWorkspaceOperationUiSettlement::Rejected
        );
        assert_eq!(state.active_correlation(), Some(&second));
    }

    #[test]
    fn workspace_and_presentation_drift_are_distinguished_without_leaving_busy_state() {
        let reset = correlation(1, "/workspace");
        let mut workspace_drift = PlanningWorkspaceOperationUiState::default();
        workspace_drift.begin(reset.clone(), 5);
        assert_eq!(
            workspace_drift.settle(&reset, "/other", 5),
            PlanningWorkspaceOperationUiSettlement::WorkspaceSuperseded
        );
        assert_eq!(workspace_drift.active_correlation(), None);

        let mut presentation_drift = PlanningWorkspaceOperationUiState::default();
        presentation_drift.begin(reset.clone(), 5);
        assert_eq!(
            presentation_drift.settle(&reset, "/workspace", 6),
            PlanningWorkspaceOperationUiSettlement::PresentationSuperseded
        );
        assert_eq!(presentation_drift.active_correlation(), None);
    }
}
