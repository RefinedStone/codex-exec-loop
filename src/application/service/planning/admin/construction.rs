use std::sync::Arc;

use crate::application::port::outbound::{
    planning_authority_port::{NoopPlanningAuthorityPort, PlanningAuthorityPort},
    planning_task_repository_port::{NoopPlanningTaskRepositoryPort, PlanningTaskRepositoryPort},
    planning_workspace_port::PlanningWorkspacePort,
};
use crate::application::service::planning::PlanningServices;
use crate::application::service::planning::runtime::validation::PlanningValidationService;
use crate::application::service::planning::task_mutation::PlanningTaskMutationService;
use crate::domain::planning::PriorityQueueService;

use super::PlanningAdminFacadeService;

impl PlanningAdminFacadeService {
    pub fn from_planning(
        workspace_dir: impl Into<String>,
        planning: PlanningServices,
        planning_workspace_port: Arc<dyn PlanningWorkspacePort>,
    ) -> Self {
        Self::from_planning_with_authority(
            workspace_dir,
            planning,
            planning_workspace_port,
            Arc::new(NoopPlanningAuthorityPort::default()),
            Arc::new(NoopPlanningTaskRepositoryPort),
        )
    }

    pub fn from_planning_with_authority(
        workspace_dir: impl Into<String>,
        planning: PlanningServices,
        planning_workspace_port: Arc<dyn PlanningWorkspacePort>,
        planning_authority_port: Arc<dyn PlanningAuthorityPort>,
        planning_task_repository_port: Arc<dyn PlanningTaskRepositoryPort>,
    ) -> Self {
        let priority_queue_service = PriorityQueueService::new();
        let task_mutation_service = PlanningTaskMutationService::new(
            planning_task_repository_port.clone(),
            priority_queue_service.clone(),
        );
        Self {
            workspace_dir: workspace_dir.into(),
            planning,
            planning_workspace_port,
            planning_authority_port,
            planning_task_repository_port,
            planning_validation_service: PlanningValidationService::new(),
            priority_queue_service,
            task_mutation_service,
        }
    }

    pub fn new(
        workspace_dir: impl Into<String>,
        planning_workspace_port: Arc<dyn PlanningWorkspacePort>,
    ) -> Self {
        let planning = PlanningServices::from_workspace_port(planning_workspace_port.clone());
        Self::from_planning(workspace_dir, planning, planning_workspace_port)
    }
}
