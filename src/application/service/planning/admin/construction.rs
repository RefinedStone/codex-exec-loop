// Admin facade는 여러 outbound port와 planning feature handle을 오래 보관한다. Arc는 DB/file
// adapter를 facade, PlanningServices, task mutation service가 같은 boundary로 공유하게 하는 소유권 장치다.
use std::sync::Arc;

// Admin construction은 workspace, authority, task repository 포트를 조합한다.
use crate::application::port::outbound::{
    // Planning authority는 DB-backed authoritative state를 관리한다.
    planning_authority_port::PlanningAuthorityPort,
    // Task repository는 task authority snapshot과 queue state를 저장한다.
    planning_task_repository_port::PlanningTaskRepositoryPort,
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

// 이 impl은 admin facade의 authority-backed construction policy를 모은다.
impl PlanningAdminFacadeService {
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
}
