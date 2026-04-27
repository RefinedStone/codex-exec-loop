use std::sync::Arc;

use crate::application::port::outbound::planning_task_repository_port::PlanningTaskRepositoryPort;
use crate::application::port::outbound::planning_workspace_port::PlanningWorkspacePort;
use crate::application::service::priority_queue_service::PriorityQueueService;

use super::super::authoring::bootstrap::PlanningBootstrapService;
use super::super::authoring::directions::PlanningDirectionsService;
use super::super::runtime::prompt::PlanningPromptService;
use super::super::runtime::validation::PlanningValidationService;
use super::PlanningFeaturePorts;
use super::shared_services::PlanningSharedServices;

pub(super) struct PlanningWorkspaceUseCaseDependencies {
    pub(super) workspace: Arc<dyn PlanningWorkspacePort>,
    pub(super) task_repository: Arc<dyn PlanningTaskRepositoryPort>,
    pub(super) bootstrap: PlanningBootstrapService,
    pub(super) validation: PlanningValidationService,
    pub(super) priority_queue: PriorityQueueService,
    pub(super) directions: PlanningDirectionsService,
    pub(super) prompt: PlanningPromptService,
}

impl PlanningWorkspaceUseCaseDependencies {
    pub(super) fn new(ports: &PlanningFeaturePorts, services: &PlanningSharedServices) -> Self {
        Self {
            workspace: ports.workspace.clone(),
            task_repository: ports.task_repository.clone(),
            bootstrap: PlanningBootstrapService::new(),
            validation: services.validation.clone(),
            priority_queue: services.priority_queue.clone(),
            directions: services.directions.clone(),
            prompt: services.prompt.clone(),
        }
    }
}
