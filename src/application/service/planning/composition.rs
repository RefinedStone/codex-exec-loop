/*
 * 학습 주석: composition.rs는 planning subsystem의 composition root다. adapter가 넘긴 outbound
 * port 네 개를 shared service와 role-specific dependency bundle로 나누고, 최종적으로 PlanningFeature
 * public facade를 구성한다. 실제 업무 로직은 service/use-case에 두고, 이 파일은 "어떤 객체가 누구를
 * 의존하는가"라는 조립 정책만 책임진다.
 */
use std::sync::Arc;

// 학습 주석: 하위 dependency 모듈은 build 함수가 긴 생성자 목록으로 흐려지지 않도록 역할별 bundle을 만든다.
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
 * 학습 주석: PlanningFeaturePorts는 composition 바깥에서 들어오는 실제 outbound boundary 묶음이다.
 * Arc trait object로 보관해 workspace/runtime/worker/task-tool use case가 같은 repository/authority를
 * 공유하면서도, 각 builder가 필요한 handle만 clone해 가져갈 수 있게 한다.
 */
pub(super) struct PlanningFeaturePorts {
    // 학습 주석: workspace port는 planning 파일 bootstrap/reset/doctor가 실제 filesystem 또는 storage에 닿는 경계다.
    workspace: Arc<dyn PlanningWorkspacePort>,
    // 학습 주석: task repository는 mutable task authority 저장소로, task tool과 authoring services가 공유한다.
    task_repository: Arc<dyn PlanningTaskRepositoryPort>,
    // 학습 주석: authority port는 runtime snapshot, queue projection, distributor state처럼 accepted DB authority를 읽고 쓴다.
    authority: Arc<dyn PlanningAuthorityPort>,
    // 학습 주석: worker port는 hidden planning worker 실행을 외부 adapter로 넘기는 outbound boundary다.
    worker: Arc<dyn PlanningWorkerPort>,
}

impl PlanningFeaturePorts {
    // 학습 주석: public feature 생성자의 긴 인자 목록을 이름 있는 port bundle로 바꾸는 얇은 조립 입구다.
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
 * 학습 주석: PlanningFeatureComposition은 feature.rs가 내부 service graph를 직접 알지 않게 하는
 * builder다. 새 planning use case가 생기면 이 composition root와 필요한 dependency bundle에만
 * wiring이 추가되고, inbound adapter는 PlanningFeature facade만 계속 사용한다.
 */
pub(super) struct PlanningFeatureComposition {
    ports: PlanningFeaturePorts,
}

impl PlanningFeatureComposition {
    // 학습 주석: composition은 port ownership을 받아 build 시점에 dependency graph를 한 번만 펼친다.
    pub(super) fn new(ports: PlanningFeaturePorts) -> Self {
        Self { ports }
    }

    /*
     * 학습 주석: build는 planning feature의 service graph를 만드는 핵심 순서다. shared service를 먼저
     * 만들고, workspace/runtime/task-tool은 clone 가능한 handle만 받아 생성한다. 마지막 worker bundle은
     * 남은 ports/services를 value로 소비해 composition pass가 끝났음을 드러낸다.
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
            // 학습 주석: workspace facade는 init/reset/doctor/directions maintenance 같은 operator 관리 흐름을 묶는다.
            workspace: PlanningWorkspaceUseCaseBuilder::new(workspace_dependencies).build(),
            // 학습 주석: runtime facade는 TUI/app-server가 읽는 planning snapshot과 queue-driven follow-up 판단을 제공한다.
            runtime: PlanningRuntimeUseCaseBuilder::new(runtime_dependencies).build(),
            // 학습 주석: task_tool facade는 LLM/tool payload를 task repository mutation으로 제한해 적용한다.
            task_tool: task_tool_use_cases,
            // 학습 주석: worker facade는 planning worker prompt/orchestration과 proposal promotion을 담당한다.
            worker: PlanningWorkerUseCaseBuilder::new(worker_dependencies).build(),
        }
    }
}

// 학습 주석: workspace builder는 operator-facing maintenance services를 하나의 workspace use-case facade로 묶는다.
struct PlanningWorkspaceUseCaseBuilder {
    dependencies: PlanningWorkspaceUseCaseDependencies,
}

impl PlanningWorkspaceUseCaseBuilder {
    fn new(dependencies: PlanningWorkspaceUseCaseDependencies) -> Self {
        Self { dependencies }
    }

    /*
     * 학습 주석: init/reset/doctor는 같은 bootstrap/validation/prompt stack을 서로 다른 operator command로
     * 노출한다. 여기서 clone과 move를 정리해 각 service가 필요한 shared helper만 갖도록 하고, facade 밖으로
     * 개별 service 타입이 새지 않게 한다.
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

// 학습 주석: runtime builder는 snapshot/follow-up intake 기능을 runtime facade 하나로 압축한다.
struct PlanningRuntimeUseCaseBuilder {
    dependencies: PlanningRuntimeUseCaseDependencies,
}

impl PlanningRuntimeUseCaseBuilder {
    fn new(dependencies: PlanningRuntimeUseCaseDependencies) -> Self {
        Self { dependencies }
    }

    // 학습 주석: runtime use case는 내부 service를 노출하지 않고 facade 수준의 read와 task intake만 제공한다.
    fn build(self) -> PlanningRuntimeUseCases {
        PlanningRuntimeUseCases::new(
            self.dependencies.runtime_facade,
            self.dependencies.task_intake,
        )
    }
}

// 학습 주석: worker builder는 hidden worker orchestration과 promotion service를 하나의 facade 뒤에 묶는다.
struct PlanningWorkerUseCaseBuilder {
    dependencies: PlanningWorkerUseCaseDependencies,
}

impl PlanningWorkerUseCaseBuilder {
    fn new(dependencies: PlanningWorkerUseCaseDependencies) -> Self {
        Self { dependencies }
    }

    // 학습 주석: worker facade는 direction authoring, worker turn orchestration, proposal promotion을 함께 제공한다.
    fn build(self) -> PlanningWorkerUseCases {
        PlanningWorkerUseCases::new(
            self.dependencies.directions,
            self.dependencies.worker_orchestration,
            self.dependencies.proposal_promotion,
        )
    }
}

// 학습 주석: task-tool builder는 repository와 priority queue만 필요하므로 별도 작은 builder로 둔다.
struct PlanningTaskToolUseCaseBuilder {
    task_tool: PlanningTaskToolService,
}

impl PlanningTaskToolUseCaseBuilder {
    /*
     * 학습 주석: task tool은 worker나 workspace port를 몰라도 된다. composition이 필요한 두 의존성만
     * 골라 주입해 LLM task mutation boundary가 파일 workspace나 worker 실행 경계로 새지 않게 한다.
     */
    fn new(ports: &PlanningFeaturePorts, services: &PlanningSharedServices) -> Self {
        Self {
            task_tool: PlanningTaskToolService::new(
                ports.task_repository.clone(),
                services.priority_queue.clone(),
            ),
        }
    }

    // 학습 주석: use-case wrapper는 adapter 호출자가 안정적인 PlanningTaskToolUseCases 표면만 보게 한다.
    fn build(self) -> PlanningTaskToolUseCases {
        PlanningTaskToolUseCases::new(self.task_tool)
    }
}
