use std::sync::Arc;

use anyhow::{Result, bail};

use crate::application::port::outbound::planning_task_repository_port::PlanningTaskRepositoryPort;
use crate::application::port::outbound::planning_workspace_port::PlanningWorkspacePort;
use crate::application::service::planning::runtime::validation::PlanningValidationService;
use crate::domain::planning::{PlanningValidationReport, PriorityQueueService};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanningTrackedTaskLedgerApplyResult {
    pub applied_paths: Vec<String>,
    pub validation_report: PlanningValidationReport,
}

impl PlanningTrackedTaskLedgerApplyResult {
    pub fn applied(&self) -> bool {
        !self.applied_paths.is_empty() && self.validation_report.is_valid()
    }
}

#[derive(Clone, Default)]
pub struct PlanningTaskLedgerApplyService;

impl PlanningTaskLedgerApplyService {
    pub fn with_task_repository(
        _planning_workspace_port: Arc<dyn PlanningWorkspacePort>,
        _planning_validation_service: PlanningValidationService,
        _priority_queue_service: PriorityQueueService,
        _planning_task_repository_port: Arc<dyn PlanningTaskRepositoryPort>,
    ) -> Self {
        Self
    }

    pub fn apply_tracked_task_ledger(
        &self,
        _workspace_dir: &str,
    ) -> Result<PlanningTrackedTaskLedgerApplyResult> {
        bail!(
            "tracked task-ledger.json is read-only; update task authority through runtime task intake, admin task management, or planning worker flows"
        )
    }
}
