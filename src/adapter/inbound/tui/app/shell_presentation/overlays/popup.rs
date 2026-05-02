// 학습 주석: base popup module은 session/startup처럼 공통 modal frame에 가까운 popup builders를 담습니다.
#[path = "popup/base.rs"]
mod base;
// 학습 주석: planning popup module은 planning init과 draft editor처럼 planning workflow와 직접 연결된
// modal overlay builders를 담습니다.
#[path = "popup/planning.rs"]
mod planning;
// 학습 주석: queue popup은 background queue/session execution 상태를 list 형태로 보여 주는 overlay입니다.
#[path = "popup/queue.rs"]
mod queue;
// 학습 주석: supersession popup은 현재 작업이 다른 작업으로 대체되는 흐름을 사용자에게 알려 주는
// modal presentation입니다.
#[path = "popup/supersession.rs"]
mod supersession;
// 학습 주석: task_intake popup은 사용자가 새 task 입력을 검토하거나 확정하는 modal overlay입니다.
#[path = "popup/task_intake.rs"]
mod task_intake;
// 학습 주석: views module은 모든 popup builder가 반환하는 view DTO type들을 정의합니다. builder
// module과 DTO module을 분리해 variant별 생성 로직과 공통 surface를 구분합니다.
#[path = "popup/views.rs"]
mod views;

// 학습 주석: session/startup popup builders는 base module의 구현을 popup surface로 올려 보냅니다.
pub(crate) use base::{build_session_overlay_view, build_startup_overlay_view};
// 학습 주석: planning popup builders는 planning workflow 진입점이므로 shell frontend가 이 re-export만
// 의존하게 합니다.
pub(crate) use planning::{
    build_planning_draft_editor_overlay_view, build_planning_init_overlay_view,
};
// 학습 주석: queue popup builder는 queue module 내부 projection을 감추고 생성 함수만 공개합니다.
pub(crate) use queue::build_queue_overlay_view;
// 학습 주석: supersession builder re-export는 shell frontend가 supersession module path를 직접 알지
// 않게 하는 public facade입니다.
pub(crate) use supersession::build_supersession_overlay_view;
// 학습 주석: task intake builder도 popup surface에서 바로 공개해 modal builders를 한 namespace에 모읍니다.
pub(crate) use task_intake::build_task_intake_overlay_view;
// 학습 주석: popup view DTO re-export는 caller가 builder 반환 타입을 같은 popup surface에서 참조하게
// 합니다. 각 view의 field 구조는 views module에 남아 있습니다.
pub(crate) use views::{
    PlanningDraftEditorOverlayView, PlanningInitOverlayView, QueueOverlayView, SessionOverlayView,
    StartupOverlayView, SupersessionOverlayView, TaskIntakeOverlayView,
};
