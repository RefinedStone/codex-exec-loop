use anyhow::{Result, bail};

use crate::domain::planning::PlanningValidationReport;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanningTrackedDirectionsApplyResult {
    pub applied_paths: Vec<String>,
    pub validation_report: PlanningValidationReport,
}

impl PlanningTrackedDirectionsApplyResult {
    pub fn applied(&self) -> bool {
        !self.applied_paths.is_empty() && self.validation_report.is_valid()
    }
}

#[derive(Clone, Default)]
pub struct PlanningDirectionsApplyService;

impl PlanningDirectionsApplyService {
    #[cfg(test)]
    #[allow(dead_code)]
    pub fn new(
        _planning_workspace_port: std::sync::Arc<
            dyn crate::application::port::outbound::planning_workspace_port::PlanningWorkspacePort,
        >,
        _planning_validation_service: crate::application::service::planning::runtime::validation::PlanningValidationService,
    ) -> Self {
        Self
    }

    pub fn with_task_repository(
        _planning_workspace_port: std::sync::Arc<
            dyn crate::application::port::outbound::planning_workspace_port::PlanningWorkspacePort,
        >,
        _planning_validation_service: crate::application::service::planning::runtime::validation::PlanningValidationService,
        _priority_queue_service: crate::domain::planning::PriorityQueueService,
        _planning_task_repository_port: std::sync::Arc<
            dyn crate::application::port::outbound::planning_task_repository_port::PlanningTaskRepositoryPort,
        >,
    ) -> Self {
        Self
    }

    pub fn apply_tracked_directions(
        &self,
        _workspace_dir: &str,
    ) -> Result<PlanningTrackedDirectionsApplyResult> {
        bail!(
            "tracked direction file import was removed; edit directions through DB-backed direction management"
        )
    }
}

#[cfg(test)]
mod tests {
    use super::PlanningTrackedDirectionsApplyResult;
    use crate::domain::planning::PlanningValidationReport;

    #[test]
    fn unapplied_result_is_false() {
        let result = PlanningTrackedDirectionsApplyResult {
            applied_paths: Vec::new(),
            validation_report: PlanningValidationReport::default(),
        };
        assert!(!result.applied());
    }
}
