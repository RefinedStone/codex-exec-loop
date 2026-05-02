// runtime facade는 planning runtime 상태 조회/제어 use case를 묶은 service이다. runtime dependency
// bundle은 이 facade를 그대로 노출해 runtime-facing use case wrapper가 호출하게 한다.
use super::super::runtime::facade::PlanningRuntimeFacadeService;
// task intake service는 외부 입력을 planning task ledger와 priority queue에 반영하는 runtime path이다.
// ports와 shared validation/queue services를 조립해 새 instance로 만든다.
use super::super::runtime::intake::PlanningTaskIntakeService;
// feature ports는 workspace, repository 같은 outbound boundary adapter들을 담는 composition input이다.
use super::PlanningFeaturePorts;
// shared services는 validation, runtime facade, priority queue처럼 여러 use case가 공유하는 application
// service instance 묶음이다.
use super::shared_services::PlanningSharedServices;

// 이 struct는 runtime-facing planning use cases에 필요한 service만 추려 담는 내부 dependency bundle이다.
// composition root가 큰 ports/services 묶음을 하위 facade가 소비하기 쉬운 단위로 나눈다.
pub(super) struct PlanningRuntimeUseCaseDependencies {
    // runtime_facade는 runtime state 조회/명령을 담당하는 이미 조립된 shared service이다.
    pub(super) runtime_facade: PlanningRuntimeFacadeService,
    // task_intake는 inbound runtime 요청을 workspace/task repository/queue update로 변환한다.
    pub(super) task_intake: PlanningTaskIntakeService,
}

// constructor는 composition root의 wiring policy를 캡슐화한다. runtime use case는 원본 ports/services
// 전체를 보지 않고 이 bundle만 받아 필요한 기능을 갖게 된다.
impl PlanningRuntimeUseCaseDependencies {
    // runtime dependency는 worker dependency와 달리 ports/services를 borrowed reference로 받는다. 같은
    // composition pass에서 worker bundle도 만들어야 하므로 여기서는 clone 가능한 service/port handle만 복제한다.
    pub(super) fn new(ports: &PlanningFeaturePorts, services: &PlanningSharedServices) -> Self {
        Self {
            // runtime facade는 shared service로 이미 완성되어 있고 runtime/worker 양쪽에서 참조될 수
            // 있어 clone된 handle을 runtime bundle에 넣는다.
            runtime_facade: services.runtime_facade.clone(),
            // task intake는 runtime 요청을 실제 planning state mutation으로 연결하므로 workspace,
            // task repository, validation, priority queue 네 경계를 명시적으로 주입한다.
            task_intake: PlanningTaskIntakeService::new(
                ports.workspace.clone(),
                ports.task_repository.clone(),
                services.validation.clone(),
                services.priority_queue.clone(),
            ),
        }
    }
}
