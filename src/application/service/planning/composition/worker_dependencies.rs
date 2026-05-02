// 학습 주석: directions service는 planning authoring에서 direction catalog를 읽고 고치는 use case입니다.
use super::super::authoring::directions::PlanningDirectionsService;
// 학습 주석: proposal promotion service는 staged proposal을 accepted planning state로 승격하는 authoring path입니다.
use super::super::authoring::proposal_promotion::PlanningProposalPromotionService;
// 학습 주석: worker orchestration service는 planner worker 실행, authority, runtime facade, task repository를
// 연결해 background planning workflow를 제어합니다.
use super::super::worker::orchestration::PlanningWorkerOrchestrationService;
// 학습 주석: feature ports는 worker, workspace, authority, task repository 같은 outbound adapters를 담습니다.
use super::PlanningFeaturePorts;
// 학습 주석: shared services는 directions, prompt, validation, queue, runtime facade처럼 worker/authoring path가
// 재사용하는 application service instance 묶음입니다.
use super::shared_services::PlanningSharedServices;

// 학습 주석: 이 bundle은 worker-facing planning use cases에 필요한 service만 모읍니다. runtime bundle이
// lightweight runtime API를 담당한다면, worker bundle은 background worker와 proposal promotion 흐름을 담당합니다.
pub(super) struct PlanningWorkerUseCaseDependencies {
    // 학습 주석: directions는 worker 흐름에서도 direction catalog를 확인하거나 보강할 때 쓰는 authoring service입니다.
    pub(super) directions: PlanningDirectionsService,
    // 학습 주석: worker_orchestration은 planner worker 실행과 runtime state transition을 조율하는 중심 service입니다.
    pub(super) worker_orchestration: PlanningWorkerOrchestrationService,
    // 학습 주석: proposal_promotion은 worker 또는 operator가 만든 proposal을 accepted planning artifacts로
    // 옮기는 authoring boundary입니다.
    pub(super) proposal_promotion: PlanningProposalPromotionService,
}

// 학습 주석: worker dependency constructor는 composition pass의 마지막 소비자입니다. runtime bundle 생성 후
// 남은 ports/services ownership을 받아, clone을 최소화하면서 worker-facing services를 완성합니다.
impl PlanningWorkerUseCaseDependencies {
    // 학습 주석: 여기서는 ports/services를 value로 받습니다. worker bundle이 composition root에서 남은
    // ownership을 가져가며, 필요한 곳에서만 task_repository처럼 두 service가 공유할 handle을 clone합니다.
    pub(super) fn new(ports: PlanningFeaturePorts, services: PlanningSharedServices) -> Self {
        Self {
            // 학습 주석: directions service는 shared services에서 이미 조립되어 있으므로 worker bundle로 이동합니다.
            directions: services.directions,
            // 학습 주석: worker orchestration은 실제 worker adapter, runtime facade, authority store, task repository를
            // 함께 필요로 합니다. 이 조합이 background planning execution boundary입니다.
            worker_orchestration: PlanningWorkerOrchestrationService::new(
                ports.worker,
                services.runtime_facade,
                ports.authority,
                ports.task_repository.clone(),
            ),
            // 학습 주석: proposal promotion은 workspace에 artifact를 쓰고 prompt/validation/queue/task repository를
            // 함께 갱신합니다. task_repository는 orchestration에도 필요하므로 위에서 clone된 뒤 원본을 여기로 이동합니다.
            proposal_promotion: PlanningProposalPromotionService::with_task_repository(
                ports.workspace,
                services.prompt,
                services.validation,
                services.priority_queue,
                ports.task_repository,
            ),
        }
    }
}
