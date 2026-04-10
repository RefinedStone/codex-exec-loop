use std::sync::Arc;

use crate::application::port::outbound::planning_workspace_port::PlanningWorkspacePort;
use crate::application::service::planning_bootstrap_service::PlanningBootstrapService;
use crate::application::service::planning_init_service::PlanningInitService;
use crate::application::service::planning_prompt_service::PlanningPromptService;
use crate::application::service::planning_reconciliation_service::PlanningReconciliationService;
use crate::application::service::planning_runtime_facade_service::PlanningRuntimeFacadeService;
use crate::application::service::planning_runtime_policy_service::PlanningRuntimePolicyService;
use crate::application::service::planning_validation_service::PlanningValidationService;
use crate::application::service::priority_queue_service::PriorityQueueService;
use crate::application::service::turn_prompt_assembly_service::TurnPromptAssemblyService;

#[derive(Clone)]
pub struct PlanningServices {
    pub init_service: PlanningInitService,
    pub runtime_facade: PlanningRuntimeFacadeService,
}

impl PlanningServices {
    pub fn from_workspace_port(planning_workspace_port: Arc<dyn PlanningWorkspacePort>) -> Self {
        let validation_service = PlanningValidationService::new();
        let priority_queue_service = PriorityQueueService::new();
        let init_service = PlanningInitService::new(
            planning_workspace_port.clone(),
            PlanningBootstrapService::new(),
            validation_service.clone(),
        );
        let planning_prompt_service = PlanningPromptService::new(
            planning_workspace_port.clone(),
            validation_service.clone(),
            priority_queue_service.clone(),
        );
        let planning_reconciliation_service = PlanningReconciliationService::new(
            planning_workspace_port,
            validation_service,
            priority_queue_service,
        );

        Self {
            init_service,
            runtime_facade: PlanningRuntimeFacadeService::new(
                planning_prompt_service,
                planning_reconciliation_service,
                PlanningRuntimePolicyService::new(),
                TurnPromptAssemblyService::new(),
            ),
        }
    }
}
