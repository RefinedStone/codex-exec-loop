use std::sync::Arc;

use crate::application::port::outbound::planning_authority_port::{
    NoopPlanningAuthorityPort, PlanningAuthorityPort,
};
use crate::application::port::outbound::planning_task_repository_port::{
    NoopPlanningTaskRepositoryPort, PlanningTaskRepositoryPort,
};
use crate::application::port::outbound::planning_worker_port::{
    NoopPlanningWorkerPort, PlanningWorkerPort,
};
use crate::application::port::outbound::planning_workspace_port::PlanningWorkspacePort;

use super::composition::{PlanningFeatureComposition, PlanningFeaturePorts};
use super::use_cases::{
    PlanningRuntimeUseCases, PlanningTaskToolUseCases, PlanningWorkerUseCases,
    PlanningWorkspaceUseCases,
};

#[derive(Clone)]
pub struct PlanningFeature {
    pub workspace: PlanningWorkspaceUseCases,
    pub runtime: PlanningRuntimeUseCases,
    pub worker: PlanningWorkerUseCases,
    pub task_tool: PlanningTaskToolUseCases,
}

impl PlanningFeature {
    pub fn from_ports(
        planning_workspace_port: Arc<dyn PlanningWorkspacePort>,
        planning_authority_port: Arc<dyn PlanningAuthorityPort>,
        planning_task_repository_port: Arc<dyn PlanningTaskRepositoryPort>,
        planning_worker_port: Arc<dyn PlanningWorkerPort>,
    ) -> Self {
        PlanningFeatureComposition::new(PlanningFeaturePorts::new(
            planning_workspace_port,
            planning_task_repository_port,
            planning_authority_port,
            planning_worker_port,
        ))
        .build()
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
