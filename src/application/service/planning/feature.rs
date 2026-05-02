// 학습 주석: Arc는 trait object 포트를 여러 서비스가 같은 소유권 모델로 나누어 쓰게 해 줍니다. PlanningFeature는 outbound
// adapter의 구체 타입을 알지 않고 Arc<dyn ...> 경계만 받아 application use case 묶음으로 바꿉니다.
use std::sync::Arc;

// 학습 주석: authority port는 planning의 authoritative 상태를 읽고 쓰는 outbound 경계입니다. Noop 구현은 workspace-only
// 구성에서도 feature가 만들어지게 하는 기본값이고, 실제 저장소가 연결되면 dyn PlanningAuthorityPort로 들어옵니다.
use crate::application::port::outbound::planning_authority_port::{
    NoopPlanningAuthorityPort, PlanningAuthorityPort,
};
// 학습 주석: task repository port는 task authority 스냅샷과 큐 관련 데이터를 저장하는 경계입니다. from_workspace_port 경로는
// 이 저장소가 없는 호출자를 위해 NoopPlanningTaskRepositoryPort를 꽂아 feature 표면을 유지합니다.
use crate::application::port::outbound::planning_task_repository_port::{
    NoopPlanningTaskRepositoryPort, PlanningTaskRepositoryPort,
};
// 학습 주석: worker port는 실제 planning worker 실행을 application service 밖으로 밀어내는 경계입니다. Noop worker는 테스트나
// 제한된 TUI 흐름에서 worker 실행 없이도 workspace/runtime use case를 구성할 수 있게 합니다.
use crate::application::port::outbound::planning_worker_port::{
    NoopPlanningWorkerPort, PlanningWorkerPort,
};
// 학습 주석: workspace port는 파일 기반 planning workspace를 읽고 쓰는 핵심 경계입니다. from_workspace_port가 이 포트만 요구하는
// 이유는 기존 호출자들이 최소한의 workspace 기능만으로 PlanningFeature를 만들 수 있어야 하기 때문입니다.
use crate::application::port::outbound::planning_workspace_port::PlanningWorkspacePort;

// 학습 주석: feature.rs는 public 생성자만 노출하고, 실제 서비스 그래프 조립은 composition 모듈에 맡깁니다. 이렇게 하면 이 파일은
// "어떤 포트를 받는가"만 책임지고, "어떤 서비스가 어떤 순서로 묶이는가"는 PlanningFeatureComposition에 남습니다.
use super::composition::{PlanningFeatureComposition, PlanningFeaturePorts};
// 학습 주석: use case 묶음들은 adapter가 직접 호출하는 application 표면입니다. PlanningFeature는 workspace/runtime/worker/task_tool
// 네 영역을 한 값으로 들고 다니게 해 TUI, CLI, 테스트가 같은 진입 구조를 공유하게 합니다.
use super::use_cases::{
    PlanningRuntimeUseCases, PlanningTaskToolUseCases, PlanningWorkerUseCases,
    PlanningWorkspaceUseCases,
};

// 학습 주석: Clone이 필요한 이유는 TUI 상태, conversation runtime, 테스트 헬퍼가 같은 planning feature handle을 복제해 보관하기
// 때문입니다. 내부 use case들이 clone 가능한 포트/서비스를 들고 있어 feature 전체도 얕게 복제됩니다.
#[derive(Clone)]
// 학습 주석: PlanningFeature는 planning subsystem의 public facade입니다. 외부 adapter는 내부 service/composition 모듈을 알 필요
// 없이 이 네 필드 중 필요한 use case 묶음만 선택해 호출합니다.
pub struct PlanningFeature {
    // 학습 주석: workspace는 draft/promote/reset/summary처럼 파일 workspace 자체를 다루는 use case 표면입니다.
    pub workspace: PlanningWorkspaceUseCases,
    // 학습 주석: runtime은 task intake, follow-up 판단, execution snapshot, reconciliation처럼 턴 실행 경로에서 쓰는 표면입니다.
    pub runtime: PlanningRuntimeUseCases,
    // 학습 주석: worker는 planning worker dispatch와 queue repair처럼 외부 worker 실행 경계까지 이어지는 표면입니다.
    pub worker: PlanningWorkerUseCases,
    // 학습 주석: task_tool은 task authority를 도구 호출 형태로 읽고 갱신하는 좁은 표면입니다.
    pub task_tool: PlanningTaskToolUseCases,
}

// 학습 주석: 이 impl은 PlanningFeature를 만드는 public entrypoint를 모읍니다. adapter 쪽은 포트만 준비해 넘기고, 내부 서비스
// 조립 방식은 composition 계층에 캡슐화됩니다.
impl PlanningFeature {
    // 학습 주석: from_ports는 완전한 production 구성 경로입니다. 네 outbound boundary를 모두 받아 PlanningFeaturePorts로 묶은 뒤
    // composition builder에 넘겨 workspace/runtime/worker/task_tool use case를 한 번에 생성합니다.
    pub fn from_ports(
        // 학습 주석: workspace 포트는 planning 파일과 projection을 다루는 모든 영역의 기반입니다.
        planning_workspace_port: Arc<dyn PlanningWorkspacePort>,
        // 학습 주석: authority 포트는 authoritative planning 상태를 DB나 다른 저장소에 반영하는 adapter 경계입니다.
        planning_authority_port: Arc<dyn PlanningAuthorityPort>,
        // 학습 주석: task repository 포트는 task queue와 snapshot 관련 저장소 작업을 application service에서 분리합니다.
        planning_task_repository_port: Arc<dyn PlanningTaskRepositoryPort>,
        // 학습 주석: worker 포트는 실제 worker 실행 방식을 app-server adapter나 noop adapter로 갈아 끼울 수 있게 하는 경계입니다.
        planning_worker_port: Arc<dyn PlanningWorkerPort>,
    ) -> Self {
        // 학습 주석: PlanningFeaturePorts::new는 인자 순서를 composition이 이해하는 이름 있는 묶음으로 바꿉니다. 그 다음 build가
        // shared service와 dependency bundle을 만들고 최종 PlanningFeature 구조체를 돌려줍니다.
        PlanningFeatureComposition::new(PlanningFeaturePorts::new(
            planning_workspace_port,
            planning_task_repository_port,
            planning_authority_port,
            planning_worker_port,
        ))
        .build()
    }

    // 학습 주석: from_workspace_port는 예전 호출자와 가벼운 테스트를 위한 축약 생성자입니다. workspace 포트만 실제 구현으로 받고,
    // authority/task repository/worker는 noop으로 채워 feature의 public shape를 그대로 유지합니다.
    pub fn from_workspace_port(planning_workspace_port: Arc<dyn PlanningWorkspacePort>) -> Self {
        // 학습 주석: 아래 호출은 축약 경로도 결국 from_ports를 타게 만듭니다. 그래서 production 경로와 noop 경로가 서로 다른
        // 조립 로직을 갖지 않고, composition 변경이 한곳에만 반영됩니다.
        Self::from_ports(
            planning_workspace_port,
            // 학습 주석: Noop authority는 DB-backed authority가 없어도 관련 호출이 기본 동작으로 끝나게 하는 안전한 대체 포트입니다.
            Arc::new(NoopPlanningAuthorityPort::default()),
            // 학습 주석: Noop task repository는 task 저장소가 없는 구성에서 repository 의존성을 만족시키는 빈 구현입니다.
            Arc::new(NoopPlanningTaskRepositoryPort),
            // 학습 주석: Noop worker는 worker 실행 요청을 실제 app-server 호출로 보내지 않는 테스트/축약 구성용 구현입니다.
            Arc::new(NoopPlanningWorkerPort),
        )
    }
}
