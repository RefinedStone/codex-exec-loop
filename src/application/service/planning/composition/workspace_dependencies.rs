// workspace use case는 파일 포트, task repository 포트, 여러 application service를 동시에 들고 있어야
// 한다. Arc는 포트 trait object를 builder와 downstream service가 같은 handle로 나누어 갖게 하는 소유권 장치이다.
use std::sync::Arc;

// task repository port는 workspace use case가 task authority snapshot과 queue 상태를 읽거나 저장할 때 쓰는 outbound 경계이다.
use crate::application::port::outbound::planning_task_repository_port::PlanningTaskRepositoryPort;
// workspace port는 draft, active planning 파일, runtime projection 같은 파일 workspace 작업의 실제 I/O 경계이다.
use crate::application::port::outbound::planning_workspace_port::PlanningWorkspacePort;
// priority queue service는 workspace summary와 prompt preparation이 같은 task 우선순위 규칙을 쓰게 하는 도메인 서비스이다.
use crate::domain::planning::PriorityQueueService;

// bootstrap service는 새 planning workspace를 만들 때 기본 파일과 seed 자료를 준비한다. workspace use
// case 표면에서 초기화 명령을 처리하려면 이 서비스가 필요하다.
use super::super::authoring::bootstrap::PlanningBootstrapService;
// directions service는 방향 문서와 queue-idle prompt editor session을 다룬다. workspace use case가
// draft/summary 흐름을 열어 주기 때문에 이 dependency bundle에 포함된다.
use super::super::authoring::directions::PlanningDirectionsService;
// prompt service는 workspace 파일과 task repository를 읽어 runtime projection/prompt context를 만들며, workspace summary와
// runtime preview 사이를 이어 주는 읽기 서비스이다.
use super::super::runtime::prompt::PlanningPromptService;
// validation service는 workspace 파일 내용과 planning 상태 전환을 같은 규칙으로 검증하게 하는 공통 부품이다.
use super::super::runtime::validation::PlanningValidationService;
// PlanningFeaturePorts는 composition 바깥에서 주입된 outbound 경계 묶음이다. 여기서는 workspace use case가 필요한
// workspace/task repository 포트만 꺼낸다.
use super::PlanningFeaturePorts;
// PlanningSharedServices는 validation, priority queue, directions, prompt처럼 여러 use case 묶음이 공유하는 서비스 그래프이다.
use super::shared_services::PlanningSharedServices;

// PlanningWorkspaceUseCaseDependencies는 PlanningWorkspaceUseCases::new로 넘길 재료를 이름 있는 묶음으로 정리한다.
// composition.rs의 builder가 긴 인자 목록을 직접 다루지 않게 하고, workspace 영역의 의존성 표면을 한 파일에서 볼 수 있게 한다.
pub(super) struct PlanningWorkspaceUseCaseDependencies {
    // workspace 포트는 모든 workspace 파일 읽기/쓰기 use case의 기반 I/O handle이다.
    pub(super) workspace: Arc<dyn PlanningWorkspacePort>,
    // task_repository는 workspace summary가 task authority와 queue 정보를 함께 보여줄 수 있게 하는 저장소 경계이다.
    pub(super) task_repository: Arc<dyn PlanningTaskRepositoryPort>,
    // bootstrap은 workspace 생성/초기화 명령 전용 서비스이다. 공유 서비스에 넣지 않고 여기서 새로 만드는 이유는
    // 현재 workspace use case만 bootstrap을 직접 필요로 하기 때문이다.
    pub(super) bootstrap: PlanningBootstrapService,
    // validation은 shared services에서 온 공통 검증 규칙이다. workspace와 runtime이 같은 invalid/ready 판단을 공유한다.
    pub(super) validation: PlanningValidationService,
    // priority_queue는 planning 방향과 task summary를 같은 우선순위 기준으로 정렬하게 한다.
    pub(super) priority_queue: PriorityQueueService,
    // directions는 draft editor, direction summary, queue-idle prompt 작성 흐름을 workspace use case에 연결한다.
    pub(super) directions: PlanningDirectionsService,
    // prompt는 workspace use case에서도 runtime-facing summary나 prompt preview를 만들 때 재사용된다.
    pub(super) prompt: PlanningPromptService,
}

// 이 impl은 ports/shared services를 workspace dependency bundle로 변환하는 유일한 생성 경로이다. 어떤
// 의존성이 새 인스턴스이고 어떤 의존성이 공유 clone인지가 이 함수에 드러난다.
impl PlanningWorkspaceUseCaseDependencies {
    // new는 전체 PlanningFeaturePorts를 받지만 workspace use case에 필요한 경계만 선택한다. 이렇게 하면 worker/runtime
    // 전용 포트가 workspace builder로 새지 않고, composition 계층에서 역할별 의존성 분리가 유지된다.
    pub(super) fn new(ports: &PlanningFeaturePorts, services: &PlanningSharedServices) -> Self {
        Self {
            // Arc clone은 workspace adapter 자체를 복제하는 것이 아니라 같은 outbound boundary handle을 공유한다.
            workspace: ports.workspace.clone(),
            // task repository도 같은 방식으로 공유되어 workspace summary와 runtime/worker 경로가 같은 저장소를 본다.
            task_repository: ports.task_repository.clone(),
            // bootstrap은 상태 없는 작성 서비스라서 workspace dependency 조립 시점에 직접 생성해도 공유 의미가 깨지지 않는다.
            bootstrap: PlanningBootstrapService::new(),
            // validation clone은 runtime/prompt/directions와 같은 검증 정책을 workspace use case에도 주입한다.
            validation: services.validation.clone(),
            // priority_queue clone은 workspace 화면에서 보이는 정렬/대표 task 판단이 worker queue와 어긋나지 않게 한다.
            priority_queue: services.priority_queue.clone(),
            // directions는 shared service에서 이미 workspace/task repository와 공통 검증 규칙으로 조립된 인스턴스를 재사용한다.
            directions: services.directions.clone(),
            // prompt도 shared service 인스턴스를 재사용해 workspace summary와 runtime 실행 프롬프트의 해석을 맞춘다.
            prompt: services.prompt.clone(),
        }
    }
}
