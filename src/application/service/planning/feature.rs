/*
 * PlanningFeature는 planning subsystem의 public facade다. inbound adapter는 app-server, DB, filesystem,
 * worker 같은 concrete adapter를 직접 조립하지 않고 Arc<dyn Port> 네 개만 넘긴다. 이 파일은 "어떤
 * boundary를 받아 facade를 만들 수 있는가"를 공개하고, 실제 dependency graph는 composition 모듈에 맡긴다.
 */

// Arc는 trait object 포트를 여러 서비스가 같은 소유권 모델로 나누어 쓰게 해 준다. PlanningFeature는 outbound
// adapter의 구체 타입을 알지 않고 Arc<dyn ...> 경계만 받아 application use case 묶음으로 바꾼다.
use std::sync::Arc;

// authority port는 planning의 authoritative 상태를 읽고 쓰는 outbound 경계다.
use crate::application::port::outbound::planning_authority_port::PlanningAuthorityPort;
// task repository port는 task authority 스냅샷과 큐 관련 데이터를 저장하는 경계다.
use crate::application::port::outbound::planning_task_repository_port::PlanningTaskRepositoryPort;
// worker port는 실제 planning worker 실행을 application service 밖으로 밀어내는 경계다.
use crate::application::port::outbound::planning_worker_port::PlanningWorkerPort;
// workspace port는 파일 기반 planning workspace를 읽고 쓰는 핵심 경계다.
use crate::application::port::outbound::planning_workspace_port::PlanningWorkspacePort;

// feature.rs는 public 생성자만 노출하고, 실제 서비스 그래프 조립은 composition 모듈에 맡긴다. 이렇게 하면 이 파일은
// "어떤 포트를 받는가"만 책임지고, "어떤 서비스가 어떤 순서로 묶이는가"는 PlanningFeatureComposition에 남는다.
use super::composition::{PlanningFeatureComposition, PlanningFeaturePorts};
// use case 묶음들은 adapter가 직접 호출하는 application 표면이다. PlanningFeature는 workspace/runtime/worker/task_tool
// 네 영역을 한 값으로 들고 다니게 해 TUI, CLI, 테스트가 같은 진입 구조를 공유하게 한다.
use super::use_cases::{
    PlanningRuntimeUseCases, PlanningTaskToolUseCases, PlanningWorkerUseCases,
    PlanningWorkspaceUseCases,
};

// Clone이 필요한 이유는 TUI 상태, conversation runtime, 테스트 헬퍼가 같은 planning feature handle을 복제해 보관하기
// 때문이다. 내부 use case들이 clone 가능한 포트/서비스를 들고 있어 feature 전체도 얕게 복제된다.
#[derive(Clone)]
// PlanningFeature는 planning subsystem의 public facade다. 외부 adapter는 내부 service/composition 모듈을 알 필요
// 없이 이 네 필드 중 필요한 use case 묶음만 선택해 호출한다.
pub struct PlanningFeature {
    // workspace는 draft/promote/reset/summary처럼 파일 workspace 자체를 다루는 use case 표면이다.
    pub workspace: PlanningWorkspaceUseCases,
    // runtime은 task intake, follow-up 판단, execution snapshot, reconciliation처럼 턴 실행 경로에서 쓰는 표면이다.
    pub runtime: PlanningRuntimeUseCases,
    // worker는 planning worker dispatch와 queue repair처럼 외부 worker 실행 경계까지 이어지는 표면이다.
    pub worker: PlanningWorkerUseCases,
    // task_tool은 task authority를 도구 호출 형태로 읽고 갱신하는 좁은 표면이다.
    pub task_tool: PlanningTaskToolUseCases,
}

// 이 impl은 PlanningFeature를 만드는 public entrypoint를 모은다. adapter 쪽은 포트만 준비해 넘기고, 내부 서비스
// 조립 방식은 composition 계층에 캡슐화된다.
impl PlanningFeature {
    // from_ports는 완전한 production 구성 경로다. 네 outbound boundary를 모두 받아 PlanningFeaturePorts로 묶은 뒤
    // composition builder에 넘겨 workspace/runtime/worker/task_tool use case를 한 번에 생성한다.
    pub fn from_ports(
        // workspace 포트는 planning 파일과 projection을 다루는 모든 영역의 기반이다.
        planning_workspace_port: Arc<dyn PlanningWorkspacePort>,
        // authority 포트는 authoritative planning 상태를 DB나 다른 저장소에 반영하는 adapter 경계다.
        planning_authority_port: Arc<dyn PlanningAuthorityPort>,
        // task repository 포트는 task queue와 snapshot 관련 저장소 작업을 application service에서 분리한다.
        planning_task_repository_port: Arc<dyn PlanningTaskRepositoryPort>,
        // worker 포트는 실제 worker 실행 방식을 app-server adapter나 noop adapter로 갈아 끼울 수 있게 하는 경계다.
        planning_worker_port: Arc<dyn PlanningWorkerPort>,
    ) -> Self {
        // PlanningFeaturePorts::new는 인자 순서를 composition이 이해하는 이름 있는 묶음으로 바꾼다. 그 다음 build가
        // shared service와 dependency bundle을 만들고 최종 PlanningFeature 구조체를 돌려준다.
        PlanningFeatureComposition::new(PlanningFeaturePorts::new(
            planning_workspace_port,
            planning_task_repository_port,
            planning_authority_port,
            planning_worker_port,
        ))
        .build()
    }
}
