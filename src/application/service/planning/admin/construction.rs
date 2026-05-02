// Admin facade는 여러 outbound port와 planning feature handle을 오래 보관한다. Arc는 DB/file
// adapter를 facade, PlanningServices, task mutation service가 같은 boundary로 공유하게 하는 소유권 장치다.
use std::sync::Arc;

// Admin construction은 workspace, authority, task repository 포트를 조합한다. Noop 포트는 admin 화면을
// workspace-only 구성으로 띄울 때 필요한 fallback이고, trait object 포트는 실제 adapter가 주입되는 production 경계다.
use crate::application::port::outbound::{
    // Planning authority는 DB-backed authoritative state를 관리한다. 없는 구성에서는 Noop으로 채워 facade shape를 유지한다.
    planning_authority_port::{NoopPlanningAuthorityPort, PlanningAuthorityPort},
    // Task repository는 task authority snapshot과 queue state를 저장한다. workspace-only constructor는 Noop repository를 쓴다.
    planning_task_repository_port::{NoopPlanningTaskRepositoryPort, PlanningTaskRepositoryPort},
    // Workspace port는 admin facade가 직접 파일을 읽고 쓰는 기본 boundary다.
    planning_workspace_port::PlanningWorkspacePort,
};
// PlanningServices는 TUI/CLI와 같은 application use case facade다. admin facade는 별도 관리 기능을 제공하지만,
// workspace/runtime/worker/task_tool 경로는 이 planning facade를 통해 재사용한다.
use crate::application::service::planning::PlanningServices;
// Validation service는 admin mutation과 overview 흐름이 planning 파일/상태를 같은 규칙으로 검사하게 한다.
use crate::application::service::planning::runtime::validation::PlanningValidationService;
// Task mutation service는 admin UI가 task를 생성/수정/삭제할 때 command extraction과 repository commit을 담당한다.
use crate::application::service::planning::task_mutation::PlanningTaskMutationService;
// Priority queue service는 admin task mutation과 planning feature가 task 우선순위를 같은 기준으로 해석하게 하는 도메인 부품이다.
use crate::domain::planning::PriorityQueueService;

// PlanningAdminFacadeService는 admin API/pages가 들고 다니는 상위 service다. 이 파일은 그 구조체를 만드는 생성자만 담당한다.
use super::PlanningAdminFacadeService;

// 이 impl은 admin facade의 construction policy를 모은다. 호출자는 workspace-only, authority-backed,
// fully default 생성자 중 하나를 고르고, 실제 field 조립은 from_planning_with_authority가 한곳에서 수행한다.
impl PlanningAdminFacadeService {
    // from_planning은 이미 만들어진 PlanningServices와 workspace port만으로 admin facade를 구성하는 축약 경로다.
    // authority/task repository가 없는 테스트나 파일 중심 admin 화면에서 사용할 수 있게 Noop boundary를 채워 넣는다.
    pub fn from_planning(
        // workspace_dir은 이 admin facade 인스턴스가 관리할 루트다. Into<String>으로 받아 호출자가 String/&str을 모두 넘길 수 있다.
        workspace_dir: impl Into<String>,
        // planning은 외부에서 이미 조립한 planning use case facade다. admin facade는 이를 소유해 overview/runtime 호출에 재사용한다.
        planning: PlanningServices,
        // planning_workspace_port는 admin CRUD와 file sync가 직접 workspace 파일을 다룰 때 쓰는 같은 file boundary다.
        planning_workspace_port: Arc<dyn PlanningWorkspacePort>,
    ) -> Self {
        // 축약 생성자도 결국 full 생성자를 타게 한다. 이렇게 하면 validation, priority queue, task mutation service 조립
        // 순서가 하나의 함수에만 존재해 admin facade field가 생성자별로 달라지지 않는다.
        Self::from_planning_with_authority(
            workspace_dir,
            planning,
            planning_workspace_port,
            // Noop authority는 DB authority 없이 admin facade를 만들 때 authoritative write를 빈 동작으로 대체한다.
            Arc::new(NoopPlanningAuthorityPort::default()),
            // Noop task repository는 task storage가 없는 구성에서도 task mutation dependency graph를 만족시킨다.
            Arc::new(NoopPlanningTaskRepositoryPort),
        )
    }

    // from_planning_with_authority는 admin facade의 완전한 조립 경로다. production admin API처럼 DB authority와 task
    // repository가 준비된 경우 이 생성자를 통해 모든 boundary를 같은 facade에 연결한다.
    pub fn from_planning_with_authority(
        // workspace_dir은 admin operation이 암묵적으로 대상으로 삼는 루트다. 이후 메서드들은 대부분 이 값을 request에 넣는다.
        workspace_dir: impl Into<String>,
        // planning은 workspace/runtime/worker use case 묶음이다. admin facade는 내부 서비스 대신 이 공개 facade를 통해 기능을 호출한다.
        planning: PlanningServices,
        // workspace port는 admin 문서 CRUD, file sync, overview 진단에서 직접 사용된다.
        planning_workspace_port: Arc<dyn PlanningWorkspacePort>,
        // authority port는 direction/task authority snapshot을 authoritative store에 반영하는 경계다.
        planning_authority_port: Arc<dyn PlanningAuthorityPort>,
        // task repository port는 PlanningTaskMutationService와 admin task views가 같은 task store를 보게 하는 경계다.
        planning_task_repository_port: Arc<dyn PlanningTaskRepositoryPort>,
    ) -> Self {
        // Priority queue service를 먼저 만든다. admin task mutation과 facade field가 같은 priority rule을 공유하도록
        // 아래 task_mutation_service에는 clone을 넘기고 원본은 facade에 보관한다.
        let priority_queue_service = PriorityQueueService::new();
        // Task mutation service는 repository commit과 priority calculation을 함께 필요로 한다. repository Arc clone은
        // 같은 저장소 handle을 서비스 내부 소유권에 맞게 넘기는 것이다.
        let task_mutation_service = PlanningTaskMutationService::new(
            planning_task_repository_port.clone(),
            priority_queue_service.clone(),
        );
        Self {
            // workspace_dir은 Into<String>을 여기서 한 번만 소비해 facade가 독립적인 owned String을 갖게 한다.
            workspace_dir: workspace_dir.into(),
            // planning facade는 overview/runtime/workspace 요청을 admin facade에서 다시 위임할 때 사용된다.
            planning,
            // port field들은 admin 문서 작업과 authority sync 작업이 같은 outbound boundary를 사용하게 한다.
            planning_workspace_port,
            planning_authority_port,
            planning_task_repository_port,
            // validation service는 admin request를 처리할 때 planning runtime validation과 같은 규칙을 적용하기 위한 전용 인스턴스다.
            planning_validation_service: PlanningValidationService::new(),
            // priority_queue_service는 facade field로도 남겨 admin projection/mutation 경로가 동일한 queue 판단을 재사용한다.
            priority_queue_service,
            // task_mutation_service는 admin task CRUD 메서드들이 직접 호출하는 application service다.
            task_mutation_service,
        }
    }

    // new는 가장 가벼운 public constructor다. workspace port만 있으면 PlanningServices를 workspace-only 구성으로 먼저
    // 만들고, 그 결과를 from_planning 축약 경로에 넘겨 admin facade까지 완성한다.
    pub fn new(
        // workspace_dir은 admin facade가 사용할 기본 workspace root다.
        workspace_dir: impl Into<String>,
        // planning_workspace_port는 PlanningServices 생성과 admin facade field 보관에 모두 필요하므로 clone 가능한 Arc로 받는다.
        planning_workspace_port: Arc<dyn PlanningWorkspacePort>,
    ) -> Self {
        // Workspace-only PlanningServices는 authority/task repository/worker를 Noop으로 채우는 경량 feature다. admin
        // facade가 최소 파일 작업만 필요한 상황에서도 같은 public facade shape를 얻게 한다.
        let planning = PlanningServices::from_workspace_port(planning_workspace_port.clone());
        // 최종 조립은 from_planning에 위임해 Noop authority/repository와 공통 admin field 초기화를 재사용한다.
        Self::from_planning(workspace_dir, planning, planning_workspace_port)
    }
}
