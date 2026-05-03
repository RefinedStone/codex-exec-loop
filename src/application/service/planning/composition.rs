/*
 * composition.rs is the planning subsystem's application-level composition root.
 * Adapters provide the outbound ports; this file groups them into shared services and role-specific
 * dependency bundles, then exposes only the PlanningFeature facade. Business rules stay in the services
 * and use-case facades, while this module owns the dependency graph and construction order.
 */
use std::sync::Arc;

// Dependency modules keep role-specific wiring out of the top-level build sequence.
mod runtime_dependencies;
mod shared_services;
mod worker_dependencies;
mod workspace_dependencies;

use crate::application::port::outbound::planning_authority_port::PlanningAuthorityPort;
use crate::application::port::outbound::planning_task_repository_port::PlanningTaskRepositoryPort;
use crate::application::port::outbound::planning_worker_port::PlanningWorkerPort;
use crate::application::port::outbound::planning_workspace_port::PlanningWorkspacePort;

use self::runtime_dependencies::PlanningRuntimeUseCaseDependencies;
use self::shared_services::PlanningSharedServices;
use self::worker_dependencies::PlanningWorkerUseCaseDependencies;
use self::workspace_dependencies::PlanningWorkspaceUseCaseDependencies;
use super::authoring::init::PlanningInitService;
use super::feature::PlanningFeature;
use super::repair::doctor::PlanningDoctorService;
use super::repair::reset::PlanningResetService;
use super::task_tool::PlanningTaskToolService;
use super::use_cases::{
    PlanningRuntimeUseCases, PlanningTaskToolUseCases, PlanningWorkerUseCases,
    PlanningWorkspaceUseCases,
};

#[derive(Clone)]
/*
 * PlanningFeaturePorts is the only bundle of concrete outbound boundaries accepted by this composition
 * layer. Each field is an Arc trait object so workspace, runtime, worker, and task-tool use cases share
 * the same storage/worker handles while each builder clones only the dependencies it needs.
 */
pub(super) struct PlanningFeaturePorts {
    // Workspace IO supports bootstrap, reset, doctor, and supporting-file validation flows.
    workspace: Arc<dyn PlanningWorkspacePort>,
    // Task repository owns mutable task authority and commit conflict handling.
    task_repository: Arc<dyn PlanningTaskRepositoryPort>,
    // Authority port backs accepted runtime snapshots, queue projection, and distributor state.
    authority: Arc<dyn PlanningAuthorityPort>,
    // Worker port delegates hidden planning-worker execution to the outbound adapter.
    worker: Arc<dyn PlanningWorkerPort>,
}

impl PlanningFeaturePorts {
    // Constructor turns the public feature inputs into a named bundle before dependency splitting starts.
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

/*
 * PlanningFeatureComposition prevents feature.rs and inbound adapters from knowing the internal service
 * graph. New planning capabilities add wiring here and in the relevant dependency bundle, while callers
 * continue to use the stable PlanningFeature facade.
 */
pub(super) struct PlanningFeatureComposition {
    ports: PlanningFeaturePorts,
}

impl PlanningFeatureComposition {
    // The composition object owns the port bundle until build consumes it into the final feature graph.
    pub(super) fn new(ports: PlanningFeaturePorts) -> Self {
        Self { ports }
    }

    /*
     * build defines the service graph order for the whole planning feature.
     * Shared services are created first, workspace/runtime/task-tool facades receive borrowed cloneable
     * handles, and the worker dependency bundle consumes the remaining ports/services by value to show
     * that no more wiring should happen after the PlanningFeature is assembled.
     */
    pub(super) fn build(self) -> PlanningFeature {
        let services = PlanningSharedServices::new(&self.ports);
        let workspace_dependencies =
            PlanningWorkspaceUseCaseDependencies::new(&self.ports, &services);
        let runtime_dependencies = PlanningRuntimeUseCaseDependencies::new(&self.ports, &services);
        let task_tool_use_cases =
            PlanningTaskToolUseCaseBuilder::new(&self.ports, &services).build();
        let worker_dependencies = PlanningWorkerUseCaseDependencies::new(self.ports, services);
        PlanningFeature {
            // Workspace facade groups operator maintenance flows: init, reset, doctor, and directions.
            workspace: PlanningWorkspaceUseCaseBuilder::new(workspace_dependencies).build(),
            // Runtime facade serves TUI/app-server snapshots and queue-driven follow-up decisions.
            runtime: PlanningRuntimeUseCaseBuilder::new(runtime_dependencies).build(),
            // Task-tool facade confines LLM/tool payloads to task repository mutations.
            task_tool: task_tool_use_cases,
            // Worker facade handles planning-worker prompts, orchestration, and proposal promotion.
            worker: PlanningWorkerUseCaseBuilder::new(worker_dependencies).build(),
        }
    }
}

// Workspace builder folds operator-facing maintenance services into one use-case facade.
struct PlanningWorkspaceUseCaseBuilder {
    dependencies: PlanningWorkspaceUseCaseDependencies,
}

impl PlanningWorkspaceUseCaseBuilder {
    fn new(dependencies: PlanningWorkspaceUseCaseDependencies) -> Self {
        Self { dependencies }
    }

    /*
     * init/reset/doctor expose the same bootstrap, validation, and prompt stack through different
     * operator commands. This builder localizes clone/move decisions so each service gets only its helper
     * set and individual service types do not leak past the workspace facade.
     */
    fn build(self) -> PlanningWorkspaceUseCases {
        PlanningWorkspaceUseCases::new(
            PlanningInitService::with_task_repository(
                self.dependencies.workspace.clone(),
                self.dependencies.bootstrap.clone(),
                self.dependencies.validation.clone(),
                self.dependencies.task_repository.clone(),
                self.dependencies.priority_queue.clone(),
            ),
            PlanningResetService::with_task_repository(
                self.dependencies.workspace.clone(),
                self.dependencies.bootstrap,
                self.dependencies.task_repository.clone(),
                self.dependencies.validation.clone(),
                self.dependencies.priority_queue.clone(),
            ),
            PlanningDoctorService::new(self.dependencies.prompt),
            self.dependencies.directions,
        )
    }
}

// Runtime builder compresses snapshot reads and follow-up intake into one runtime facade.
struct PlanningRuntimeUseCaseBuilder {
    dependencies: PlanningRuntimeUseCaseDependencies,
}

impl PlanningRuntimeUseCaseBuilder {
    fn new(dependencies: PlanningRuntimeUseCaseDependencies) -> Self {
        Self { dependencies }
    }

    // The runtime facade exposes reads and task intake without leaking internal service ownership.
    fn build(self) -> PlanningRuntimeUseCases {
        PlanningRuntimeUseCases::new(
            self.dependencies.runtime_facade,
            self.dependencies.task_intake,
        )
    }
}

// Worker builder hides planning-worker orchestration and proposal promotion behind one facade.
struct PlanningWorkerUseCaseBuilder {
    dependencies: PlanningWorkerUseCaseDependencies,
}

impl PlanningWorkerUseCaseBuilder {
    fn new(dependencies: PlanningWorkerUseCaseDependencies) -> Self {
        Self { dependencies }
    }

    // Direction authoring, worker turn orchestration, and proposal promotion share the worker use-case surface.
    fn build(self) -> PlanningWorkerUseCases {
        PlanningWorkerUseCases::new(
            self.dependencies.directions,
            self.dependencies.worker_orchestration,
            self.dependencies.proposal_promotion,
        )
    }
}

// Task-tool builder stays separate because it only needs repository mutation and queue projection helpers.
struct PlanningTaskToolUseCaseBuilder {
    task_tool: PlanningTaskToolService,
}

impl PlanningTaskToolUseCaseBuilder {
    /*
     * The task tool does not need worker or workspace ports.
     * Injecting only the repository and priority queue prevents the LLM task mutation boundary from
     * reaching into file workspace concerns or worker execution concerns.
     */
    fn new(ports: &PlanningFeaturePorts, services: &PlanningSharedServices) -> Self {
        Self {
            task_tool: PlanningTaskToolService::new(
                ports.task_repository.clone(),
                services.priority_queue.clone(),
            ),
        }
    }

    // The wrapper gives adapters a stable PlanningTaskToolUseCases surface instead of a raw service.
    fn build(self) -> PlanningTaskToolUseCases {
        PlanningTaskToolUseCases::new(self.task_tool)
    }
}
