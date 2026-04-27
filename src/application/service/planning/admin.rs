use std::sync::Arc;

mod construction;
mod crud;
mod documents;
mod draft_session;
mod file_sync;
mod overview;
mod projection;
mod reset;
mod surface;

use crate::application::port::outbound::planning_authority_port::PlanningAuthorityPort;
use crate::application::port::outbound::planning_task_repository_port::PlanningTaskRepositoryPort;
use crate::application::port::outbound::planning_workspace_port::PlanningWorkspacePort;
use crate::application::service::planning::PlanningServices;
use crate::application::service::planning::runtime::validation::PlanningValidationService;
use crate::application::service::priority_queue_service::PriorityQueueService;

pub use self::surface::*;

#[derive(Clone)]
pub struct PlanningAdminFacadeService {
    workspace_dir: String,
    planning: PlanningServices,
    planning_workspace_port: Arc<dyn PlanningWorkspacePort>,
    planning_authority_port: Arc<dyn PlanningAuthorityPort>,
    planning_task_repository_port: Arc<dyn PlanningTaskRepositoryPort>,
    planning_validation_service: PlanningValidationService,
    priority_queue_service: PriorityQueueService,
}

impl PlanningAdminFacadeService {
    pub fn workspace_dir(&self) -> &str {
        &self.workspace_dir
    }
}
