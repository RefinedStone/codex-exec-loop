use std::sync::Arc;

use crate::application::port::outbound::planning_worker_port::{
    PlanningWorkerPort, PlanningWorkerRequest, PlanningWorkerResponse,
};
use crate::application::port::outbound::planning_workspace_port::PlanningWorkspacePort;
use crate::application::service::planning_bootstrap_service::PlanningBootstrapService;
use crate::application::service::planning_init_service::PlanningInitService;
use crate::application::service::planning_prompt_service::PlanningPromptService;
use crate::application::service::planning_proposal_promotion_service::PlanningProposalPromotionService;
use crate::application::service::planning_reconciliation_service::PlanningReconciliationService;
use crate::application::service::planning_runtime_facade_service::PlanningRuntimeFacadeService;
use crate::application::service::planning_runtime_policy_service::PlanningRuntimePolicyService;
use crate::application::service::planning_validation_service::PlanningValidationService;
use crate::application::service::planning_worker_orchestration_service::PlanningWorkerOrchestrationService;
use crate::application::service::priority_queue_service::PriorityQueueService;
use crate::application::service::turn_prompt_assembly_service::TurnPromptAssemblyService;

#[derive(Clone)]
pub struct PlanningServices {
    pub init_service: PlanningInitService,
    pub proposal_promotion: PlanningProposalPromotionService,
    pub runtime_facade: PlanningRuntimeFacadeService,
    pub worker_orchestration: PlanningWorkerOrchestrationService,
}

impl PlanningServices {
    pub fn from_ports(
        planning_workspace_port: Arc<dyn PlanningWorkspacePort>,
        planning_worker_port: Arc<dyn PlanningWorkerPort>,
    ) -> Self {
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
            planning_workspace_port.clone(),
            validation_service.clone(),
            priority_queue_service.clone(),
        );
        let runtime_facade = PlanningRuntimeFacadeService::new(
            planning_prompt_service.clone(),
            planning_reconciliation_service.clone(),
            PlanningRuntimePolicyService::new(),
            TurnPromptAssemblyService::new(),
        );
        let proposal_promotion = PlanningProposalPromotionService::new(
            planning_workspace_port.clone(),
            planning_prompt_service,
            planning_reconciliation_service,
            validation_service,
            priority_queue_service,
        );

        Self {
            init_service,
            proposal_promotion,
            worker_orchestration: PlanningWorkerOrchestrationService::new(
                planning_worker_port,
                runtime_facade.clone(),
            ),
            runtime_facade,
        }
    }

    pub fn from_workspace_port(planning_workspace_port: Arc<dyn PlanningWorkspacePort>) -> Self {
        Self::from_ports(planning_workspace_port, Arc::new(NoopPlanningWorkerPort))
    }
}

struct NoopPlanningWorkerPort;

impl PlanningWorkerPort for NoopPlanningWorkerPort {
    fn run_planning_session(
        &self,
        request: PlanningWorkerRequest,
    ) -> anyhow::Result<PlanningWorkerResponse> {
        Ok(PlanningWorkerResponse {
            operation: request.operation,
            final_agent_message: Some("planner worker disabled".to_string()),
            changed_planning_file_paths: Vec::new(),
        })
    }
}
