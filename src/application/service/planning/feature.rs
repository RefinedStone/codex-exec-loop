use std::sync::Arc;

use crate::application::port::outbound::planning_authority_port::PlanningAuthorityPort;
use crate::application::port::outbound::planning_task_repository_port::{
    NoopPlanningTaskRepositoryPort, PlanningTaskRepositoryPort,
};
use crate::application::port::outbound::planning_worker_port::PlanningWorkerPort;
use crate::application::port::outbound::planning_workspace_port::PlanningWorkspacePort;

use super::authoring::bootstrap::PlanningBootstrapService;
use super::authoring::directions::PlanningDirectionsService;
use super::authoring::directions_apply::PlanningDirectionsApplyService;
use super::authoring::init::PlanningInitService;
use super::authoring::proposal_promotion::PlanningProposalPromotionService;
use super::authoring::task_ledger_apply::PlanningTaskLedgerApplyService;
use super::noop_ports::{NoopPlanningAuthorityPort, NoopPlanningWorkerPort};
use super::repair::doctor::PlanningDoctorService;
use super::repair::reconciliation::PlanningReconciliationService;
use super::repair::reset::PlanningResetService;
use super::runtime::facade::PlanningRuntimeFacadeService;
use super::runtime::intake::PlanningTaskIntakeService;
use super::runtime::policy::PlanningRuntimePolicyService;
use super::runtime::prompt::PlanningPromptService;
use super::runtime::validation::PlanningValidationService;
use super::use_cases::{
    PlanningRuntimeUseCases, PlanningWorkerUseCases, PlanningWorkspaceUseCases,
};
use super::worker::orchestration::PlanningWorkerOrchestrationService;
use crate::application::service::priority_queue_service::PriorityQueueService;
use crate::application::service::turn_prompt_assembly_service::TurnPromptAssemblyService;

#[derive(Clone)]
pub struct PlanningFeature {
    pub workspace: PlanningWorkspaceUseCases,
    pub runtime: PlanningRuntimeUseCases,
    pub worker: PlanningWorkerUseCases,
}

impl PlanningFeature {
    pub fn from_ports(
        planning_workspace_port: Arc<dyn PlanningWorkspacePort>,
        planning_authority_port: Arc<dyn PlanningAuthorityPort>,
        planning_task_repository_port: Arc<dyn PlanningTaskRepositoryPort>,
        planning_worker_port: Arc<dyn PlanningWorkerPort>,
    ) -> Self {
        let validation_service = PlanningValidationService::new();
        let priority_queue_service = PriorityQueueService::new();
        let init_service = PlanningInitService::with_task_repository(
            planning_workspace_port.clone(),
            PlanningBootstrapService::new(),
            validation_service.clone(),
            planning_task_repository_port.clone(),
            priority_queue_service.clone(),
        );
        let reset_service = PlanningResetService::with_task_repository(
            planning_workspace_port.clone(),
            PlanningBootstrapService::new(),
            planning_task_repository_port.clone(),
            validation_service.clone(),
            priority_queue_service.clone(),
        );
        let directions_service = PlanningDirectionsService::new(
            planning_workspace_port.clone(),
            validation_service.clone(),
        );
        let directions_apply_service = PlanningDirectionsApplyService::with_task_repository(
            planning_workspace_port.clone(),
            validation_service.clone(),
            priority_queue_service.clone(),
            planning_task_repository_port.clone(),
        );
        let task_ledger_apply_service = PlanningTaskLedgerApplyService::with_task_repository(
            planning_workspace_port.clone(),
            validation_service.clone(),
            priority_queue_service.clone(),
            planning_task_repository_port.clone(),
        );
        let planning_prompt_service = PlanningPromptService::with_task_repository(
            planning_workspace_port.clone(),
            validation_service.clone(),
            priority_queue_service.clone(),
            planning_task_repository_port.clone(),
        );
        let doctor_service = PlanningDoctorService::new(planning_prompt_service.clone());
        let planning_reconciliation_service = PlanningReconciliationService::with_task_repository(
            planning_workspace_port.clone(),
            validation_service.clone(),
            priority_queue_service.clone(),
            planning_task_repository_port.clone(),
        );
        let runtime_facade = PlanningRuntimeFacadeService::new(
            planning_prompt_service.clone(),
            planning_reconciliation_service.clone(),
            PlanningRuntimePolicyService::new(),
            TurnPromptAssemblyService::new(),
        );
        let task_intake = PlanningTaskIntakeService::new(
            planning_workspace_port.clone(),
            planning_task_repository_port.clone(),
            validation_service.clone(),
            priority_queue_service.clone(),
        );
        let proposal_promotion = PlanningProposalPromotionService::with_task_repository(
            planning_workspace_port,
            planning_prompt_service,
            validation_service,
            priority_queue_service,
            planning_task_repository_port,
        );

        Self {
            workspace: PlanningWorkspaceUseCases::new(
                init_service,
                reset_service,
                doctor_service,
                directions_service.clone(),
                directions_apply_service,
                task_ledger_apply_service,
            ),
            runtime: PlanningRuntimeUseCases::new(runtime_facade.clone(), task_intake),
            worker: PlanningWorkerUseCases::new(
                directions_service,
                PlanningWorkerOrchestrationService::new(
                    planning_worker_port,
                    runtime_facade,
                    planning_authority_port,
                ),
                proposal_promotion,
            ),
        }
    }

    pub fn from_workspace_port(planning_workspace_port: Arc<dyn PlanningWorkspacePort>) -> Self {
        Self::from_ports(
            planning_workspace_port,
            Arc::new(NoopPlanningAuthorityPort::default()),
            Arc::new(NoopPlanningTaskRepositoryPort),
            Arc::new(NoopPlanningWorkerPort),
        )
    }
}
