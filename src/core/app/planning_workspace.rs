#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlanningWorkspaceResetTarget {
    Queue,
    Directions,
    All,
}

impl PlanningWorkspaceResetTarget {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Queue => "queue",
            Self::Directions => "directions",
            Self::All => "all",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanningWorkspaceResetIntent {
    pub workspace_directory: String,
    pub target: PlanningWorkspaceResetTarget,
}

impl PlanningWorkspaceResetIntent {
    pub fn new(
        workspace_directory: impl Into<String>,
        target: PlanningWorkspaceResetTarget,
    ) -> Self {
        Self {
            workspace_directory: workspace_directory.into(),
            target,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanningWorkspaceOperationCorrelation {
    pub generation: u64,
    pub workspace_directory: String,
    pub reset_target: PlanningWorkspaceResetTarget,
}

impl PlanningWorkspaceOperationCorrelation {
    fn from_intent(generation: u64, intent: PlanningWorkspaceResetIntent) -> Self {
        Self {
            generation,
            workspace_directory: intent.workspace_directory,
            reset_target: intent.target,
        }
    }

    fn matches_intent(&self, intent: &PlanningWorkspaceResetIntent) -> bool {
        self.workspace_directory == intent.workspace_directory && self.reset_target == intent.target
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlanningWorkspaceOperationAdmission {
    Started {
        correlation: PlanningWorkspaceOperationCorrelation,
    },
    Coalesced {
        correlation: PlanningWorkspaceOperationCorrelation,
    },
    Busy {
        active_correlation: PlanningWorkspaceOperationCorrelation,
        requested: PlanningWorkspaceResetIntent,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanningWorkspaceResetSnapshot {
    pub target: PlanningWorkspaceResetTarget,
    pub rewritten_paths: Vec<String>,
    pub removed_paths: Vec<String>,
}

#[derive(Debug, Clone)]
pub(crate) struct PlanningWorkspaceOperationCoordinator {
    next_generation: u64,
    active: Option<PlanningWorkspaceOperationCorrelation>,
}

impl PlanningWorkspaceOperationCoordinator {
    pub(crate) fn new() -> Self {
        Self {
            next_generation: 1,
            active: None,
        }
    }

    pub(crate) fn begin_reset(
        &mut self,
        intent: PlanningWorkspaceResetIntent,
    ) -> PlanningWorkspaceOperationAdmission {
        if let Some(active) = self.active.as_ref() {
            return if active.matches_intent(&intent) {
                PlanningWorkspaceOperationAdmission::Coalesced {
                    correlation: active.clone(),
                }
            } else {
                PlanningWorkspaceOperationAdmission::Busy {
                    active_correlation: active.clone(),
                    requested: intent,
                }
            };
        }

        let correlation =
            PlanningWorkspaceOperationCorrelation::from_intent(self.take_generation(), intent);
        self.active = Some(correlation.clone());
        PlanningWorkspaceOperationAdmission::Started { correlation }
    }

    pub(crate) fn accept(&mut self, correlation: &PlanningWorkspaceOperationCorrelation) -> bool {
        if self.active.as_ref() != Some(correlation) {
            return false;
        }
        self.active = None;
        true
    }

    fn take_generation(&mut self) -> u64 {
        let generation = self.next_generation;
        self.next_generation = generation
            .checked_add(1)
            .expect("planning workspace operation generation exhausted");
        generation
    }
}

impl Default for PlanningWorkspaceOperationCoordinator {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reset(
        workspace_directory: &str,
        target: PlanningWorkspaceResetTarget,
    ) -> PlanningWorkspaceResetIntent {
        PlanningWorkspaceResetIntent::new(workspace_directory, target)
    }

    fn started(
        admission: PlanningWorkspaceOperationAdmission,
    ) -> PlanningWorkspaceOperationCorrelation {
        let PlanningWorkspaceOperationAdmission::Started { correlation } = admission else {
            panic!("expected a started planning workspace operation");
        };
        correlation
    }

    #[test]
    fn exact_duplicate_reset_coalesces_without_consuming_a_generation() {
        let mut coordinator = PlanningWorkspaceOperationCoordinator::new();
        let first = started(
            coordinator.begin_reset(reset("/workspace", PlanningWorkspaceResetTarget::Queue)),
        );

        assert_eq!(
            coordinator.begin_reset(reset("/workspace", PlanningWorkspaceResetTarget::Queue)),
            PlanningWorkspaceOperationAdmission::Coalesced {
                correlation: first.clone(),
            }
        );
        assert!(coordinator.accept(&first));

        let second = started(
            coordinator.begin_reset(reset("/workspace", PlanningWorkspaceResetTarget::Queue)),
        );
        assert_eq!(first.generation, 1);
        assert_eq!(second.generation, 2);
    }

    #[test]
    fn a_different_target_or_workspace_is_busy_while_reset_is_active() {
        let mut coordinator = PlanningWorkspaceOperationCoordinator::new();
        let active = started(
            coordinator.begin_reset(reset("/workspace-a", PlanningWorkspaceResetTarget::Queue)),
        );

        for requested in [
            reset("/workspace-a", PlanningWorkspaceResetTarget::All),
            reset("/workspace-b", PlanningWorkspaceResetTarget::Queue),
        ] {
            assert_eq!(
                coordinator.begin_reset(requested.clone()),
                PlanningWorkspaceOperationAdmission::Busy {
                    active_correlation: active.clone(),
                    requested,
                }
            );
        }
    }

    #[test]
    fn only_the_exact_completion_settles_and_old_generation_cannot_win_aba() {
        let mut coordinator = PlanningWorkspaceOperationCoordinator::new();
        let first = started(coordinator.begin_reset(reset(
            "/workspace",
            PlanningWorkspaceResetTarget::Directions,
        )));
        let mut stale_generation = first.clone();
        stale_generation.generation += 1;
        let mut stale_workspace = first.clone();
        stale_workspace.workspace_directory = "/other".to_string();
        let mut stale_target = first.clone();
        stale_target.reset_target = PlanningWorkspaceResetTarget::All;

        assert!(!coordinator.accept(&stale_generation));
        assert!(!coordinator.accept(&stale_workspace));
        assert!(!coordinator.accept(&stale_target));
        assert!(coordinator.accept(&first));
        assert!(!coordinator.accept(&first));

        let second = started(coordinator.begin_reset(reset(
            "/workspace",
            PlanningWorkspaceResetTarget::Directions,
        )));
        assert_ne!(first.generation, second.generation);
        assert!(!coordinator.accept(&first));
        assert!(coordinator.accept(&second));
    }
}
