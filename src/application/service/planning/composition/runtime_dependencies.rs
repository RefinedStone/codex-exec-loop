use super::super::runtime::facade::PlanningRuntimeFacadeService;
use super::super::runtime::intake::PlanningTaskIntakeService;
use super::PlanningFeaturePorts;
use super::shared_services::PlanningSharedServices;

pub(super) struct PlanningRuntimeUseCaseDependencies {
    pub(super) runtime_facade: PlanningRuntimeFacadeService,
    pub(super) task_intake: PlanningTaskIntakeService,
}

impl PlanningRuntimeUseCaseDependencies {
    pub(super) fn new(ports: &PlanningFeaturePorts, services: &PlanningSharedServices) -> Self {
        Self {
            runtime_facade: services.runtime_facade.clone(),
            task_intake: PlanningTaskIntakeService::new(
                ports.workspace.clone(),
                ports.task_repository.clone(),
                services.validation.clone(),
                services.priority_queue.clone(),
            ),
        }
    }
}
