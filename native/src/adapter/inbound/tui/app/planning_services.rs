use std::sync::Arc;

use crate::application::port::outbound::planning_workspace_port::PlanningWorkspacePort;
use crate::application::service::planning_bootstrap_service::PlanningBootstrapService;
use crate::application::service::planning_init_service::PlanningInitService;
use crate::application::service::planning_prompt_service::{
    PlanningPromptService, PlanningRuntimeSnapshot,
};
use crate::application::service::planning_reconciliation_service::PlanningReconciliationService;
use crate::application::service::planning_runtime_policy_service::PlanningRuntimePolicyService;
use crate::application::service::planning_validation_service::PlanningValidationService;
use crate::application::service::priority_queue_service::PriorityQueueService;
use crate::application::service::turn_prompt_assembly_service::TurnPromptAssemblyService;

pub(super) struct PlanningServices {
    pub(super) init_service: PlanningInitService,
    pub(super) prompt_service: PlanningPromptService,
    pub(super) reconciliation_service: PlanningReconciliationService,
    pub(super) policy_service: PlanningRuntimePolicyService,
    pub(super) turn_prompt_assembly_service: TurnPromptAssemblyService,
}

impl PlanningServices {
    pub(super) fn from_workspace_port(
        planning_workspace_port: Arc<dyn PlanningWorkspacePort>,
    ) -> Self {
        let validation_service = PlanningValidationService::new();
        let priority_queue_service = PriorityQueueService::new();
        Self {
            init_service: PlanningInitService::new(
                planning_workspace_port.clone(),
                PlanningBootstrapService::new(),
                validation_service.clone(),
            ),
            prompt_service: PlanningPromptService::new(
                planning_workspace_port.clone(),
                validation_service.clone(),
                priority_queue_service.clone(),
            ),
            reconciliation_service: PlanningReconciliationService::new(
                planning_workspace_port,
                validation_service,
                priority_queue_service,
            ),
            policy_service: PlanningRuntimePolicyService::new(),
            turn_prompt_assembly_service: TurnPromptAssemblyService::new(),
        }
    }

    pub(super) fn load_runtime_snapshot(
        &self,
        workspace_directory: &str,
    ) -> PlanningRuntimeSnapshot {
        self.prompt_service
            .load_runtime_snapshot(workspace_directory)
            .unwrap_or_else(|error| planning_runtime_snapshot_load_failed(error.to_string()))
    }
}

pub(super) fn planning_runtime_snapshot_load_failed(error: String) -> PlanningRuntimeSnapshot {
    PlanningRuntimeSnapshot::invalid(format!("failed to load planning workspace: {error}"))
}
