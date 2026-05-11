// popup surface는 overlay 종류별 builder와 DTO를 한 namespace로 묶는다. shell
// frontend는 이 facade만 import하고, 개별 popup의 layout module 경로에는 의존하지 않는다.
#[path = "popup/base.rs"]
mod base;
#[path = "popup/planning.rs"]
mod planning;
#[path = "popup/queue.rs"]
mod queue;
#[path = "popup/supersession.rs"]
mod supersession;
#[path = "popup/views.rs"]
mod views;

pub(crate) use base::{build_session_overlay_view, build_startup_overlay_view};
pub(crate) use planning::{
    build_planning_draft_editor_overlay_view, build_planning_init_overlay_view,
};
pub(crate) use queue::build_queue_overlay_view;
pub(crate) use supersession::build_supersession_overlay_view;

// builder와 view DTO를 함께 re-export해 popup 호출부가 variant별 module split을
// 몰라도 type과 constructor를 같은 surface에서 다룰 수 있게 한다.
pub(crate) use views::{
    PlanningDraftEditorOverlayView, PlanningInitOverlayView, QueueOverlayView, SessionOverlayView,
    StartupOverlayView, SupersessionOverlayView,
};
