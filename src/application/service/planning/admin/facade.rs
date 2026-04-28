use std::sync::Arc;

use crate::application::port::outbound::planning_authority_port::PlanningAuthorityPort;
use crate::application::port::outbound::planning_task_repository_port::PlanningTaskRepositoryPort;
use crate::application::port::outbound::planning_workspace_port::PlanningWorkspacePort;
use crate::application::service::planning::PlanningServices;
use crate::application::service::planning::runtime::validation::PlanningValidationService;
use crate::domain::planning::PriorityQueueService;

#[derive(Clone)]
pub struct PlanningAdminFacadeService {
    pub(super) workspace_dir: String,
    pub(super) planning: PlanningServices,
    pub(super) planning_workspace_port: Arc<dyn PlanningWorkspacePort>,
    pub(super) planning_authority_port: Arc<dyn PlanningAuthorityPort>,
    pub(super) planning_task_repository_port: Arc<dyn PlanningTaskRepositoryPort>,
    pub(super) planning_validation_service: PlanningValidationService,
    pub(super) priority_queue_service: PriorityQueueService,
}

impl PlanningAdminFacadeService {
    pub fn workspace_dir(&self) -> &str {
        &self.workspace_dir
    }
}
