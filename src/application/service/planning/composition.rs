/*
 * composition.rs는 planning subsystem의 application-level composition root다. inbound/outbound adapter는 port만
 * 제공하고, 이 파일은 port를 shared service와 role-specific dependency bundle로 나눈 뒤 최종 PlanningFeature
 * facade만 노출한다. business rule은 각 service/use-case facade에 남기고, 여기서는 "어떤 경계를 어떤 순서로
 * 조립하는가"만 책임져 adapter나 feature.rs가 내부 dependency graph를 알지 않게 한다.
 */
use std::sync::Arc;

// dependency module들은 workspace/runtime/worker별 wiring을 top-level build sequence에서 분리한다. composition root는
// 전체 순서만 보여주고, 각 role의 세부 dependency 선택은 전용 파일에 둔다.
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
 * PlanningFeaturePorts는 이 composition layer가 받는 유일한 outbound boundary 묶음이다. 각 필드는 Arc trait
 * object라 workspace/runtime/worker/task-tool use case가 같은 storage/worker handle을 공유하면서도, builder별로
 * 필요한 dependency만 clone해 자기 소유권 모델에 맞게 들고 갈 수 있다.
 */
pub(super) struct PlanningFeaturePorts {
    // workspace I/O는 bootstrap, reset, doctor, supporting-file validation 흐름의 기반 경계다.
    workspace: Arc<dyn PlanningWorkspacePort>,
    // task repository는 mutable task authority와 commit conflict handling을 담당한다.
    task_repository: Arc<dyn PlanningTaskRepositoryPort>,
    // authority port는 accepted runtime snapshot, queue projection, distributor state를 뒷받침한다.
    authority: Arc<dyn PlanningAuthorityPort>,
    // worker port는 hidden planning-worker execution을 outbound adapter로 위임하는 실행 경계다.
    worker: Arc<dyn PlanningWorkerPort>,
}

impl PlanningFeaturePorts {
    // constructor는 public feature input을 이름 있는 bundle로 바꾼다. 이후 dependency splitting 단계에서는 긴
    // positional argument 대신 field name으로 각 role이 필요한 boundary를 선택한다.
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
 * PlanningFeatureComposition은 feature.rs와 inbound adapter가 내부 service graph를 알지 않게 하는 조립 객체다.
 * 새 planning capability가 생기면 이 파일과 해당 dependency bundle에 wiring을 추가하고, caller는 계속 stable
 * PlanningFeature facade만 사용한다.
 */
pub(super) struct PlanningFeatureComposition {
    ports: PlanningFeaturePorts,
}

impl PlanningFeatureComposition {
    // composition object는 build가 최종 feature graph로 소비할 때까지 port bundle을 소유한다. 이 소유권 흐름이
    // 어떤 dependency bundle이 마지막으로 ports/services를 가져가는지 코드상에 드러나게 한다.
    pub(super) fn new(ports: PlanningFeaturePorts) -> Self {
        Self { ports }
    }

    /*
     * build는 planning feature 전체 service graph의 생성 순서를 고정한다. shared services를 먼저 만들고,
     * workspace/runtime/task-tool facade는 borrow된 cloneable handle로 필요한 dependency를 가져간다. worker
     * dependency bundle은 남은 ports/services를 value로 소비해 PlanningFeature 조립 뒤 추가 wiring이 없음을 드러낸다.
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
            // workspace facade는 init/reset/doctor/directions 같은 operator maintenance flow를 묶는다.
            workspace: PlanningWorkspaceUseCaseBuilder::new(workspace_dependencies).build(),
            // runtime facade는 TUI/app-server snapshot과 queue-driven follow-up 판단을 제공한다.
            runtime: PlanningRuntimeUseCaseBuilder::new(runtime_dependencies).build(),
            // task-tool facade는 LLM/tool payload를 task repository mutation 경계 안에 가둔다.
            task_tool: task_tool_use_cases,
            // worker facade는 planning-worker prompt, orchestration, proposal promotion을 담당한다.
            worker: PlanningWorkerUseCaseBuilder::new(worker_dependencies).build(),
        }
    }
}

// workspace builder는 operator-facing maintenance service들을 하나의 use-case facade로 접는다.
struct PlanningWorkspaceUseCaseBuilder {
    dependencies: PlanningWorkspaceUseCaseDependencies,
}

impl PlanningWorkspaceUseCaseBuilder {
    fn new(dependencies: PlanningWorkspaceUseCaseDependencies) -> Self {
        Self { dependencies }
    }

    /*
     * init/reset/doctor는 같은 bootstrap, validation, prompt stack을 서로 다른 operator command로 노출한다.
     * 이 builder는 clone/move 결정을 한곳에 모아 각 service가 필요한 helper set만 받게 하고, 개별 service type이
     * workspace facade 바깥으로 새지 않게 한다.
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

// runtime builder는 snapshot read와 follow-up intake를 하나의 runtime facade로 압축한다.
struct PlanningRuntimeUseCaseBuilder {
    dependencies: PlanningRuntimeUseCaseDependencies,
}

impl PlanningRuntimeUseCaseBuilder {
    fn new(dependencies: PlanningRuntimeUseCaseDependencies) -> Self {
        Self { dependencies }
    }

    // runtime facade는 내부 service ownership을 노출하지 않고 read와 task intake 표면만 제공한다.
    fn build(self) -> PlanningRuntimeUseCases {
        PlanningRuntimeUseCases::new(
            self.dependencies.runtime_facade,
            self.dependencies.task_intake,
        )
    }
}

// worker builder는 planning-worker orchestration과 proposal promotion을 하나의 facade 뒤에 숨긴다.
struct PlanningWorkerUseCaseBuilder {
    dependencies: PlanningWorkerUseCaseDependencies,
}

impl PlanningWorkerUseCaseBuilder {
    fn new(dependencies: PlanningWorkerUseCaseDependencies) -> Self {
        Self { dependencies }
    }

    // direction authoring, worker turn orchestration, proposal promotion은 worker use-case surface를 공유한다.
    fn build(self) -> PlanningWorkerUseCases {
        PlanningWorkerUseCases::new(
            self.dependencies.directions,
            self.dependencies.worker_orchestration,
            self.dependencies.proposal_promotion,
        )
    }
}

// task-tool builder는 repository mutation과 queue projection helper만 필요하므로 별도 builder로 남긴다.
struct PlanningTaskToolUseCaseBuilder {
    task_tool: PlanningTaskToolService,
}

impl PlanningTaskToolUseCaseBuilder {
    /*
     * task tool은 worker port나 workspace port가 필요 없다. repository와 priority queue만 주입하면 LLM task
     * mutation boundary가 file workspace concern이나 worker execution concern으로 확장되는 것을 막을 수 있다.
     */
    fn new(ports: &PlanningFeaturePorts, services: &PlanningSharedServices) -> Self {
        Self {
            task_tool: PlanningTaskToolService::new(
                ports.task_repository.clone(),
                services.priority_queue.clone(),
            ),
        }
    }

    // wrapper는 adapter가 raw service 대신 stable PlanningTaskToolUseCases surface를 사용하게 한다.
    fn build(self) -> PlanningTaskToolUseCases {
        PlanningTaskToolUseCases::new(self.task_tool)
    }
}
