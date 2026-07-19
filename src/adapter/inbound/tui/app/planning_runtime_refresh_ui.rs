use super::PlanningInitRuntimeRefreshIntent;
use crate::core::app::{PlanningDoctorSnapshot, PlanningRuntimeRefreshCorrelation};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum PlanningRuntimeRefreshOperation {
    Init(PlanningInitRuntimeRefreshIntent),
    Doctor,
    ResetRecovery { reset_error: String },
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) enum PlanningRuntimeRefreshUiState {
    #[default]
    Idle,
    Loading {
        correlation: PlanningRuntimeRefreshCorrelation,
        operation: PlanningRuntimeRefreshOperation,
        presentation_revision: u64,
    },
    Ready {
        correlation: PlanningRuntimeRefreshCorrelation,
        operation: PlanningRuntimeRefreshOperation,
        doctor: PlanningDoctorSnapshot,
    },
    Failed {
        correlation: PlanningRuntimeRefreshCorrelation,
        operation: PlanningRuntimeRefreshOperation,
        error: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum PlanningRuntimeRefreshUiCompletion {
    Rejected,
    Superseded {
        operation: PlanningRuntimeRefreshOperation,
    },
    Applied {
        operation: PlanningRuntimeRefreshOperation,
        result: Result<PlanningDoctorSnapshot, String>,
    },
}

impl PlanningRuntimeRefreshUiState {
    pub(super) fn begin(
        &mut self,
        correlation: PlanningRuntimeRefreshCorrelation,
        operation: PlanningRuntimeRefreshOperation,
        presentation_revision: u64,
    ) {
        *self = Self::Loading {
            correlation,
            operation,
            presentation_revision,
        };
    }

    pub(super) fn apply_completion(
        &mut self,
        correlation: PlanningRuntimeRefreshCorrelation,
        presentation_revision: u64,
        result: Result<PlanningDoctorSnapshot, String>,
    ) -> PlanningRuntimeRefreshUiCompletion {
        let Self::Loading {
            correlation: active,
            operation,
            presentation_revision: active_presentation_revision,
        } = self
        else {
            return PlanningRuntimeRefreshUiCompletion::Rejected;
        };
        if active != &correlation {
            return PlanningRuntimeRefreshUiCompletion::Rejected;
        }
        let operation = operation.clone();
        if *active_presentation_revision != presentation_revision {
            *self = Self::Idle;
            return PlanningRuntimeRefreshUiCompletion::Superseded { operation };
        }
        *self = match &result {
            Ok(doctor) => Self::Ready {
                correlation,
                operation: operation.clone(),
                doctor: doctor.clone(),
            },
            Err(error) => Self::Failed {
                correlation,
                operation: operation.clone(),
                error: error.clone(),
            },
        };
        PlanningRuntimeRefreshUiCompletion::Applied { operation, result }
    }

    pub(super) fn rebind(&mut self, correlation: PlanningRuntimeRefreshCorrelation) -> bool {
        let Self::Loading {
            correlation: active,
            ..
        } = self
        else {
            return false;
        };
        if active.workspace_directory != correlation.workspace_directory
            || active.generation > correlation.generation
        {
            return false;
        }
        *active = correlation;
        true
    }

    pub(super) fn cancel(
        &mut self,
        correlation: &PlanningRuntimeRefreshCorrelation,
    ) -> Option<PlanningRuntimeRefreshOperation> {
        let Self::Loading {
            correlation: active,
            operation,
            ..
        } = self
        else {
            return None;
        };
        if active != correlation {
            return None;
        }
        let operation = operation.clone();
        *self = Self::Idle;
        Some(operation)
    }

    pub(super) fn clear_loading(&mut self) -> bool {
        if !matches!(self, Self::Loading { .. }) {
            return false;
        }
        *self = Self::Idle;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::{
        PlanningRuntimeRefreshOperation, PlanningRuntimeRefreshUiCompletion,
        PlanningRuntimeRefreshUiState,
    };
    use crate::adapter::inbound::tui::app::PlanningInitRuntimeRefreshIntent;
    use crate::core::app::{
        PlanningDoctorSnapshot, PlanningRuntimeRefreshCorrelation, PlanningRuntimeRefreshSnapshot,
    };
    use crate::domain::planning::RuntimeProjection;

    fn doctor() -> PlanningDoctorSnapshot {
        PlanningRuntimeRefreshSnapshot::new(RuntimeProjection::uninitialized()).doctor
    }

    #[test]
    fn completion_requires_exact_generation_workspace_and_loading_state() {
        let first = PlanningRuntimeRefreshCorrelation::new(1, "/tmp/a");
        let aba = PlanningRuntimeRefreshCorrelation::new(3, "/tmp/a");
        let wrong_workspace = PlanningRuntimeRefreshCorrelation::new(3, "/tmp/b");
        let mut state = PlanningRuntimeRefreshUiState::default();
        state.begin(aba.clone(), PlanningRuntimeRefreshOperation::Doctor, 5);

        assert!(matches!(
            state.apply_completion(first, 5, Ok(doctor())),
            PlanningRuntimeRefreshUiCompletion::Rejected
        ));
        assert!(matches!(
            state.apply_completion(wrong_workspace, 5, Ok(doctor())),
            PlanningRuntimeRefreshUiCompletion::Rejected
        ));
        assert!(matches!(
            state.apply_completion(aba.clone(), 5, Ok(doctor())),
            PlanningRuntimeRefreshUiCompletion::Applied { .. }
        ));
        assert!(matches!(
            state.apply_completion(aba, 5, Ok(doctor())),
            PlanningRuntimeRefreshUiCompletion::Rejected
        ));
        assert!(matches!(state, PlanningRuntimeRefreshUiState::Ready { .. }));
    }

    #[test]
    fn newer_presentation_revision_supersedes_only_the_exact_loading_operation() {
        let correlation = PlanningRuntimeRefreshCorrelation::new(4, "/tmp/root");
        let mut state = PlanningRuntimeRefreshUiState::default();
        state.begin(
            correlation.clone(),
            PlanningRuntimeRefreshOperation::Doctor,
            8,
        );

        assert!(matches!(
            state.apply_completion(correlation, 9, Err("late".to_string())),
            PlanningRuntimeRefreshUiCompletion::Superseded {
                operation: PlanningRuntimeRefreshOperation::Doctor
            }
        ));
        assert!(matches!(state, PlanningRuntimeRefreshUiState::Idle));
    }

    #[test]
    fn newer_same_workspace_inspection_rebinds_every_operation_and_rejects_stale_success() {
        for operation in [
            PlanningRuntimeRefreshOperation::Init(PlanningInitRuntimeRefreshIntent::Inspect),
            PlanningRuntimeRefreshOperation::Doctor,
            PlanningRuntimeRefreshOperation::ResetRecovery {
                reset_error: "reset failed".to_string(),
            },
        ] {
            let first = PlanningRuntimeRefreshCorrelation::new(1, "/tmp/root");
            let replacement = PlanningRuntimeRefreshCorrelation::new(2, "/tmp/root");
            let mut state = PlanningRuntimeRefreshUiState::default();
            state.begin(first.clone(), operation.clone(), 3);

            assert!(!state.rebind(PlanningRuntimeRefreshCorrelation::new(2, "/tmp/other")));
            assert!(state.rebind(replacement.clone()));
            assert!(matches!(
                state.apply_completion(first, 3, Ok(doctor())),
                PlanningRuntimeRefreshUiCompletion::Rejected
            ));
            assert!(matches!(
                state,
                PlanningRuntimeRefreshUiState::Loading {
                    correlation: ref active,
                    ..
                } if active == &replacement
            ));
            assert!(matches!(
                state.apply_completion(replacement, 3, Ok(doctor())),
                PlanningRuntimeRefreshUiCompletion::Applied {
                    operation: ref applied,
                    ..
                } if applied == &operation
            ));
        }
    }

    #[test]
    fn replacement_rebind_preserves_intent_drift_for_every_operation_and_result() {
        for operation in [
            PlanningRuntimeRefreshOperation::Init(PlanningInitRuntimeRefreshIntent::Inspect),
            PlanningRuntimeRefreshOperation::Doctor,
            PlanningRuntimeRefreshOperation::ResetRecovery {
                reset_error: "reset failed".to_string(),
            },
        ] {
            for result in [Ok(doctor()), Err("inspection failed".to_string())] {
                let replacement = PlanningRuntimeRefreshCorrelation::new(2, "/tmp/root");
                let mut state = PlanningRuntimeRefreshUiState::default();
                state.begin(
                    PlanningRuntimeRefreshCorrelation::new(1, "/tmp/root"),
                    operation.clone(),
                    3,
                );

                assert!(state.rebind(replacement.clone()));
                assert!(matches!(
                    state,
                    PlanningRuntimeRefreshUiState::Loading {
                        presentation_revision: 3,
                        ..
                    }
                ));
                assert_eq!(
                    state.apply_completion(replacement, 4, result),
                    PlanningRuntimeRefreshUiCompletion::Superseded {
                        operation: operation.clone(),
                    }
                );
                assert!(matches!(state, PlanningRuntimeRefreshUiState::Idle));
            }
        }
    }

    #[test]
    fn close_and_cancellation_discard_only_the_exact_loading_operation() {
        let correlation = PlanningRuntimeRefreshCorrelation::new(2, "/tmp/root");
        let mut state = PlanningRuntimeRefreshUiState::default();
        state.begin(
            correlation.clone(),
            PlanningRuntimeRefreshOperation::ResetRecovery {
                reset_error: "reset failed".to_string(),
            },
            2,
        );

        assert!(
            state
                .cancel(&PlanningRuntimeRefreshCorrelation::new(1, "/tmp/root"))
                .is_none()
        );
        assert!(state.clear_loading());
        assert!(matches!(
            state.apply_completion(correlation, 2, Err("late".to_string())),
            PlanningRuntimeRefreshUiCompletion::Rejected
        ));
        assert!(matches!(state, PlanningRuntimeRefreshUiState::Idle));
    }
}
