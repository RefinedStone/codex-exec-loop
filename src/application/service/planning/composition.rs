use std::sync::Arc;

mod shared_services;

use crate::application::port::outbound::planning_authority_port::PlanningAuthorityPort;
use crate::application::port::outbound::planning_task_repository_port::PlanningTaskRepositoryPort;
use crate::application::port::outbound::planning_worker_port::PlanningWorkerPort;
use crate::application::port::outbound::planning_workspace_port::PlanningWorkspacePort;

use self::shared_services::PlanningSharedServices;
use super::authoring::bootstrap::PlanningBootstrapService;
use super::authoring::directions_apply::PlanningDirectionsApplyService;
use super::authoring::init::PlanningInitService;
use super::authoring::proposal_promotion::PlanningProposalPromotionService;
use super::authoring::task_ledger_apply::PlanningTaskLedgerApplyService;
use super::feature::PlanningFeature;
use super::repair::doctor::PlanningDoctorService;
use super::repair::reset::PlanningResetService;
use super::runtime::intake::PlanningTaskIntakeService;
use super::use_cases::{
    PlanningRuntimeUseCases, PlanningWorkerUseCases, PlanningWorkspaceUseCases,
};
use super::worker::orchestration::PlanningWorkerOrchestrationService;

#[derive(Clone)]
pub(super) struct PlanningFeaturePorts {
    workspace: Arc<dyn PlanningWorkspacePort>,
    task_repository: Arc<dyn PlanningTaskRepositoryPort>,
    authority: Arc<dyn PlanningAuthorityPort>,
    worker: Arc<dyn PlanningWorkerPort>,
}

impl PlanningFeaturePorts {
    pub(super) fn new(
        workspace: Arc<dyn PlanningWorkspacePort>,
        task_repository: Arc<dyn PlanningTaskRepositoryPort>,
        authority: Arc<dyn PlanningAuthorityPort>,
        worker: Arc<dyn PlanningWorkerPort>,
    ) -> Self {
        Self {
            workspace,
            task_repository,
            authority,
            worker,
        }
    }
}

pub(super) struct PlanningFeatureComposition {
    ports: PlanningFeaturePorts,
}

impl PlanningFeatureComposition {
    pub(super) fn new(ports: PlanningFeaturePorts) -> Self {
        Self { ports }
    }

    pub(super) fn build(self) -> PlanningFeature {
        let services = PlanningSharedServices::new(&self.ports);
        PlanningFeature {
            workspace: PlanningWorkspaceUseCaseBuilder::new(&self.ports, &services).build(),
            runtime: PlanningRuntimeUseCaseBuilder::new(&self.ports, &services).build(),
            worker: PlanningWorkerUseCaseBuilder::new(self.ports, services).build(),
        }
    }
}

struct PlanningWorkspaceUseCaseBuilder<'a> {
    ports: &'a PlanningFeaturePorts,
    services: &'a PlanningSharedServices,
}

impl<'a> PlanningWorkspaceUseCaseBuilder<'a> {
    fn new(ports: &'a PlanningFeaturePorts, services: &'a PlanningSharedServices) -> Self {
        Self { ports, services }
    }

    fn build(&self) -> PlanningWorkspaceUseCases {
        PlanningWorkspaceUseCases::new(
            PlanningInitService::with_task_repository(
                self.ports.workspace.clone(),
                PlanningBootstrapService::new(),
                self.services.validation.clone(),
                self.ports.task_repository.clone(),
                self.services.priority_queue.clone(),
            ),
            PlanningResetService::with_task_repository(
                self.ports.workspace.clone(),
                PlanningBootstrapService::new(),
                self.ports.task_repository.clone(),
                self.services.validation.clone(),
                self.services.priority_queue.clone(),
            ),
            PlanningDoctorService::new(self.services.prompt.clone()),
            self.services.directions.clone(),
            PlanningDirectionsApplyService::with_task_repository(
                self.ports.workspace.clone(),
                self.services.validation.clone(),
                self.services.priority_queue.clone(),
                self.ports.task_repository.clone(),
            ),
            PlanningTaskLedgerApplyService::with_task_repository(
                self.ports.workspace.clone(),
                self.services.validation.clone(),
                self.services.priority_queue.clone(),
                self.ports.task_repository.clone(),
            ),
        )
    }
}

struct PlanningRuntimeUseCaseBuilder<'a> {
    ports: &'a PlanningFeaturePorts,
    services: &'a PlanningSharedServices,
}

impl<'a> PlanningRuntimeUseCaseBuilder<'a> {
    fn new(ports: &'a PlanningFeaturePorts, services: &'a PlanningSharedServices) -> Self {
        Self { ports, services }
    }

    fn build(&self) -> PlanningRuntimeUseCases {
        PlanningRuntimeUseCases::new(
            self.services.runtime_facade.clone(),
            PlanningTaskIntakeService::new(
                self.ports.workspace.clone(),
                self.ports.task_repository.clone(),
                self.services.validation.clone(),
                self.services.priority_queue.clone(),
            ),
        )
    }
}

struct PlanningWorkerUseCaseBuilder {
    ports: PlanningFeaturePorts,
    services: PlanningSharedServices,
}

impl PlanningWorkerUseCaseBuilder {
    fn new(ports: PlanningFeaturePorts, services: PlanningSharedServices) -> Self {
        Self { ports, services }
    }

    fn build(self) -> PlanningWorkerUseCases {
        PlanningWorkerUseCases::new(
            self.services.directions,
            PlanningWorkerOrchestrationService::new(
                self.ports.worker,
                self.services.runtime_facade,
                self.ports.authority,
            ),
            PlanningProposalPromotionService::with_task_repository(
                self.ports.workspace,
                self.services.prompt,
                self.services.validation,
                self.services.priority_queue,
                self.ports.task_repository,
            ),
        )
    }
}
