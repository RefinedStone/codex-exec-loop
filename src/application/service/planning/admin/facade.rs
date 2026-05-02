// admin facade는 여러 outbound port와 services를 clone 가능한 handle로 공유하므로 Arc를 사용한다.
use std::sync::Arc;

// authority port는 accepted planning authority DB/state를 읽고 쓰는 admin boundary이다.
use crate::application::port::outbound::planning_authority_port::PlanningAuthorityPort;
// task repository port는 task ledger mutation과 admin task views에 필요한 persistence boundary이다.
use crate::application::port::outbound::planning_task_repository_port::PlanningTaskRepositoryPort;
// workspace port는 draft/session/supporting files 같은 filesystem planning artifacts를 다룬다.
use crate::application::port::outbound::planning_workspace_port::PlanningWorkspacePort;
// PlanningServices는 runtime/authoring/control use cases를 모은 broader planning service surface이다.
use crate::application::service::planning::PlanningServices;
// validation service는 admin draft mutation 후 accepted planning state가 유효한지 다시 확인할 때 쓴다.
use crate::application::service::planning::runtime::validation::PlanningValidationService;
// task mutation service는 admin task action을 task repository/priority queue update로 연결한다.
use crate::application::service::planning::task_mutation::PlanningTaskMutationService;
// priority queue service는 admin에서 task priority/order를 계산하고 갱신하는 domain service이다.
use crate::domain::planning::PriorityQueueService;

// PlanningAdminFacadeService는 inbound admin API가 planning subsystem을 호출할 때 쓰는 application facade이다.
// 하위 impl 파일들이 CRUD, draft session, reset, overview, document mutation을 이 같은 struct에 메서드로 붙인다.
#[derive(Clone)]
pub struct PlanningAdminFacadeService {
    // workspace_dir은 admin page와 reset/draft operations가 대상으로 삼는 repo/workspace root이다.
    pub(super) workspace_dir: String,
    // planning은 broader planning service bundle이다. admin facade가 runtime/control summary를
    // 다시 조립하지 않고 기존 use-case surface를 호출하게 한다.
    pub(super) planning: PlanningServices,
    // workspace port는 admin draft load/save와 supporting file sync에서 사용된다.
    pub(super) planning_workspace_port: Arc<dyn PlanningWorkspacePort>,
    // authority port는 reset/overview/direction mutation이 accepted authority state를 다룰 때 사용한다.
    pub(super) planning_authority_port: Arc<dyn PlanningAuthorityPort>,
    // task repository port는 admin task ledger view와 mutation path의 persistence handle이다.
    pub(super) planning_task_repository_port: Arc<dyn PlanningTaskRepositoryPort>,
    // validation service는 admin edits가 accepted planning state로 promotion될 수 있는지 확인한다.
    pub(super) planning_validation_service: PlanningValidationService,
    // priority queue service는 task ordering/queue summaries를 domain rule에 맞춰 계산한다.
    pub(super) priority_queue_service: PriorityQueueService,
    // task mutation service는 admin task action을 validated repository update로 감싸는 application service이다.
    pub(super) task_mutation_service: PlanningTaskMutationService,
}

// 이 impl에는 facade의 작은 identity/accessor만 둔다. 큰 admin use cases는 파일별 impl로 나뉘어
// 같은 struct에 붙어 있어, facade struct가 admin subsystem의 shared dependency container 역할을 한다.
impl PlanningAdminFacadeService {
    // workspace_dir accessor는 inbound adapter가 admin page context를 표시하거나 route를 만들 때
    // facade 내부 String ownership을 노출하지 않고 &str view만 제공하게 한다.
    pub fn workspace_dir(&self) -> &str {
        &self.workspace_dir
    }
}
