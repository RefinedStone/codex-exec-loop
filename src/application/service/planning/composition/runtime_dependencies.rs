// 학습 주석: runtime facade는 planning runtime 상태 조회/제어 use case를 묶은 service입니다. runtime
// dependency bundle은 이 facade를 그대로 노출해 runtime-facing use case wrapper가 호출하게 합니다.
use super::super::runtime::facade::PlanningRuntimeFacadeService;
// 학습 주석: task intake service는 외부 입력을 planning task ledger와 priority queue에 반영하는 runtime
// path입니다. ports와 shared validation/queue services를 조립해 새 instance로 만듭니다.
use super::super::runtime::intake::PlanningTaskIntakeService;
// 학습 주석: feature ports는 workspace, repository 같은 outbound boundary adapter들을 담는 composition input입니다.
use super::PlanningFeaturePorts;
// 학습 주석: shared services는 validation, runtime facade, priority queue처럼 여러 use case가 공유하는
// application service instance 묶음입니다.
use super::shared_services::PlanningSharedServices;

// 학습 주석: 이 struct는 runtime-facing planning use cases에 필요한 service만 추려 담는 내부 dependency
// bundle입니다. composition root가 큰 ports/services 묶음을 하위 facade가 소비하기 쉬운 단위로 나눕니다.
pub(super) struct PlanningRuntimeUseCaseDependencies {
    // 학습 주석: runtime_facade는 runtime state 조회/명령을 담당하는 이미 조립된 shared service입니다.
    pub(super) runtime_facade: PlanningRuntimeFacadeService,
    // 학습 주석: task_intake는 inbound runtime 요청을 workspace/task repository/queue update로 변환합니다.
    pub(super) task_intake: PlanningTaskIntakeService,
}

// 학습 주석: constructor는 composition root의 wiring policy를 캡슐화합니다. runtime use case는 원본
// ports/services 전체를 보지 않고 이 bundle만 받아 필요한 기능을 갖게 됩니다.
impl PlanningRuntimeUseCaseDependencies {
    // 학습 주석: runtime dependency는 worker dependency와 달리 ports/services를 borrowed reference로 받습니다.
    // 같은 composition pass에서 worker bundle도 만들어야 하므로 여기서는 clone 가능한 service/port handle만 복제합니다.
    pub(super) fn new(ports: &PlanningFeaturePorts, services: &PlanningSharedServices) -> Self {
        Self {
            // 학습 주석: runtime facade는 shared service로 이미 완성되어 있고 runtime/worker 양쪽에서 참조될 수
            // 있어 clone된 handle을 runtime bundle에 넣습니다.
            runtime_facade: services.runtime_facade.clone(),
            // 학습 주석: task intake는 runtime 요청을 실제 planning state mutation으로 연결하므로 workspace,
            // task repository, validation, priority queue 네 경계를 명시적으로 주입합니다.
            task_intake: PlanningTaskIntakeService::new(
                ports.workspace.clone(),
                ports.task_repository.clone(),
                services.validation.clone(),
                services.priority_queue.clone(),
            ),
        }
    }
}
