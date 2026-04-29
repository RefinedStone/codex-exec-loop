use super::super::authoring::directions::PlanningDirectionsService;
use super::super::authoring::proposal_promotion::PlanningProposalPromotionService;
use super::super::worker::orchestration::PlanningWorkerOrchestrationService;
use super::PlanningFeaturePorts;
use super::shared_services::PlanningSharedServices;

pub(super) struct PlanningWorkerUseCaseDependencies {
    pub(super) directions: PlanningDirectionsService,
    pub(super) worker_orchestration: PlanningWorkerOrchestrationService,
    pub(super) proposal_promotion: PlanningProposalPromotionService,
}

impl PlanningWorkerUseCaseDependencies {
    pub(super) fn new(ports: PlanningFeaturePorts, services: PlanningSharedServices) -> Self {
        Self {
            directions: services.directions,
            worker_orchestration: PlanningWorkerOrchestrationService::new(
                ports.worker,
                services.runtime_facade,
                ports.authority,
                ports.task_repository.clone(),
            ),
            proposal_promotion: PlanningProposalPromotionService::with_task_repository(
                ports.workspace,
                services.prompt,
                services.validation,
                services.priority_queue,
                ports.task_repository,
            ),
        }
    }
}
