// 학습 주석: TurnPromptAssemblyService는 최종 턴 프롬프트를 조립하는 공용 서비스입니다. planning 런타임은 계획 컨텍스트를 만든 뒤
// 실제 에이전트에게 넘길 턴 프롬프트까지 이어야 하므로, 이 composition 지점에서 런타임 facade 안으로 함께 주입합니다.
use crate::application::service::turn_prompt_assembly_service::TurnPromptAssemblyService;
// 학습 주석: PriorityQueueService는 planning 작업의 우선순위 계산 규칙을 담은 도메인 서비스입니다. 방향 선택, 프롬프트 생성,
// reconciliation이 같은 정렬 감각을 쓰도록 하나를 만들어 clone으로 나누어 줍니다.
use crate::domain::planning::PriorityQueueService;

// 학습 주석: authoring::directions는 사용자가 어떤 planning 방향을 선택하거나 확정할 때 쓰는 서비스입니다. 여기서는 런타임보다
// 앞선 작성 단계도 같은 workspace/task 저장소와 검증 규칙을 보게 하기 위해 shared services에 포함합니다.
use super::super::authoring::directions::PlanningDirectionsService;
// 학습 주석: reconciliation은 런타임 실행 전후의 계획 상태를 다시 맞추는 repair 서비스입니다. 외부 dependency builder가 직접
// 꺼내 쓰지는 않지만, runtime_facade를 완성하는 내부 부품으로 이 파일에서 조립됩니다.
use super::super::repair::reconciliation::PlanningReconciliationService;
// 학습 주석: runtime facade는 planning 실행에 필요한 prompt, reconciliation, policy, turn prompt assembly를 하나의 진입점으로
// 묶습니다. composition 계층은 세부 서비스를 알고, 호출 계층은 facade만 보게 하는 경계입니다.
use super::super::runtime::facade::PlanningRuntimeFacadeService;
// 학습 주석: runtime policy는 planning 런타임의 실행 판단을 담당합니다. 저장소 포트가 필요 없는 순수 정책 부품이므로 이곳에서
// 새로 만들어 facade에 바로 넘깁니다.
use super::super::runtime::policy::PlanningRuntimePolicyService;
// 학습 주석: prompt 서비스는 workspace와 task repository를 읽어 planning worker가 사용할 컨텍스트를 만듭니다. directions와
// reconciliation이 쓰는 validation/priority_queue와 같은 인스턴스 계열을 받는 것이 중요합니다.
use super::super::runtime::prompt::PlanningPromptService;
// 학습 주석: validation 서비스는 planning 입력과 상태 전환의 기본 규칙을 모읍니다. shared services에서 하나를 만든 뒤 여러
// 서비스에 clone해 주면, 작성/프롬프트/복구 단계가 같은 검증 의미를 공유합니다.
use super::super::runtime::validation::PlanningValidationService;
// 학습 주석: PlanningFeaturePorts는 composition 바깥에서 들어온 실제 outbound 경계 묶음입니다. 이 파일은 포트를 직접 구현하지
// 않고, 이미 준비된 workspace/task repository 포트를 application 서비스들에 연결만 합니다.
use super::PlanningFeaturePorts;

// 학습 주석: PlanningSharedServices는 planning feature 안의 여러 dependency builder가 공통으로 필요로 하는 application/domain
// 서비스 묶음입니다. 포트는 PlanningFeaturePorts가 소유하고, 이 구조체는 그 포트 위에 올라가는 재사용 가능한 서비스만 보관합니다.
pub(super) struct PlanningSharedServices {
    // 학습 주석: validation은 workspace, runtime, worker 의존성 조립에서 같은 규칙을 참조해야 하는 기반 서비스입니다.
    pub(super) validation: PlanningValidationService,
    // 학습 주석: priority_queue는 계획 항목의 중요도 계산을 한곳으로 모읍니다. 여러 상위 서비스가 따로 만들지 않게 여기서 공유합니다.
    pub(super) priority_queue: PriorityQueueService,
    // 학습 주석: directions는 사용자가 검토하거나 선택하는 계획 방향을 만드는 작성 계층 서비스입니다.
    pub(super) directions: PlanningDirectionsService,
    // 학습 주석: prompt는 planning worker 실행 직전에 필요한 작업/워크스페이스 맥락을 텍스트 입력으로 바꾸는 런타임 서비스입니다.
    pub(super) prompt: PlanningPromptService,
    // 학습 주석: runtime_facade는 실제 planning 실행 경로가 세부 prompt/reconciliation/policy 조합을 몰라도 되게 하는 상위 진입점입니다.
    pub(super) runtime_facade: PlanningRuntimeFacadeService,
}

// 학습 주석: 이 impl은 shared service 묶음을 만드는 생성자만 제공합니다. dependency builder들은 구조체 필드를 읽기만 하고,
// 어떤 순서와 조합으로 서비스가 만들어지는지는 이 생성자 안에 고정됩니다.
impl PlanningSharedServices {
    // 학습 주석: new는 외부 포트 묶음을 받아 application 서비스 그래프로 바꿉니다. 반환 타입이 Self인 이유는 호출자가
    // PlanningSharedServices라는 구체 이름을 반복하지 않고도 이 구조체 인스턴스를 받게 하기 위해서입니다.
    pub(super) fn new(ports: &PlanningFeaturePorts) -> Self {
        // 학습 주석: validation은 가장 먼저 만들어집니다. 아래 서비스들이 모두 같은 검증 규칙을 주입받아 planning 상태를
        // 서로 다른 기준으로 해석하지 않게 하는 기준점입니다.
        let validation = PlanningValidationService::new();
        // 학습 주석: priority_queue도 초기에 하나만 만듭니다. clone으로 전달되는 값은 각 서비스가 같은 우선순위 계산 규칙을
        // 독립 필드처럼 들고 있게 해 주며, 규칙 자체를 중복 구성하지 않습니다.
        let priority_queue = PriorityQueueService::new();
        // 학습 주석: directions는 사용자에게 제시할 계획 방향을 만들기 위해 workspace의 현재 상태와 task repository의 작업 정보를
        // 함께 봅니다. ports.*.clone()은 저장소 데이터를 복사한다기보다 공유 포트 핸들을 각 서비스 소유권에 맞게 나눠 주는 흐름입니다.
        let directions = PlanningDirectionsService::new(
            ports.workspace.clone(),
            ports.task_repository.clone(),
            validation.clone(),
            priority_queue.clone(),
        );
        // 학습 주석: prompt는 planning worker가 읽을 실행 프롬프트를 만들기 때문에 workspace, validation, priority_queue,
        // task_repository가 모두 필요합니다. directions와 같은 기반 서비스를 공유해서 "무엇이 중요한 작업인가"의 해석을 맞춥니다.
        let prompt = PlanningPromptService::with_task_repository(
            ports.workspace.clone(),
            validation.clone(),
            priority_queue.clone(),
            ports.task_repository.clone(),
        );
        // 학습 주석: reconciliation은 런타임 중 계획 결과와 현재 작업 상태를 다시 맞추는 repair 부품입니다. shared struct 필드로
        // 노출하지 않고 runtime_facade 안에만 넣는 것은, 현재 다른 composition 조립자가 직접 다룰 필요가 없기 때문입니다.
        let reconciliation = PlanningReconciliationService::with_task_repository(
            ports.workspace.clone(),
            validation.clone(),
            priority_queue.clone(),
            ports.task_repository.clone(),
        );
        // 학습 주석: runtime_facade는 prompt 생성, reconciliation, 실행 정책, 턴 프롬프트 조립을 한 경로로 묶습니다. prompt.clone()을
        // 넘기는 이유는 prompt 서비스 자체도 shared field로 남겨 다른 의존성 조립에서 재사용해야 하기 때문입니다.
        let runtime_facade = PlanningRuntimeFacadeService::new(
            prompt.clone(),
            reconciliation,
            // 학습 주석: policy는 저장소나 workspace 포트를 직접 보지 않는 런타임 판단 규칙이므로 facade 생성 시점에 바로 새로 만듭니다.
            PlanningRuntimePolicyService::new(),
            // 학습 주석: turn prompt assembly는 planning이 만든 컨텍스트를 최종 턴 입력 형태로 연결하는 마지막 조립 단계입니다.
            TurnPromptAssemblyService::new(),
        );

        // 학습 주석: Self에는 다른 dependency builder가 실제로 꺼내 써야 하는 서비스만 담습니다. reconciliation처럼 facade 내부로
        // 흡수된 부품은 여기서 반환하지 않아 composition 표면적을 작게 유지합니다.
        Self {
            validation,
            priority_queue,
            directions,
            prompt,
            runtime_facade,
        }
    }
}
