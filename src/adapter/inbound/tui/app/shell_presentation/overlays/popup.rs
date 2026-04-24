#[path = "popup/base.rs"]
mod base;
#[path = "popup/planning.rs"]
mod planning;
#[path = "popup/queue.rs"]
mod queue;
#[path = "popup/supersession.rs"]
mod supersession;
#[path = "popup/task_intake.rs"]
mod task_intake;
#[path = "popup/views.rs"]
mod views;

pub(crate) use base::{build_session_overlay_view, build_startup_overlay_view};
pub(crate) use planning::{
    build_planning_draft_editor_overlay_view, build_planning_init_overlay_view,
};
pub(crate) use queue::{build_automation_overlay_view, build_queue_overlay_view};
pub(crate) use supersession::build_supersession_overlay_view;
pub(crate) use task_intake::build_task_intake_overlay_view;
pub(crate) use views::{
    AutomationOverlayView, PlanningDraftEditorOverlayView, PlanningInitOverlayView,
    QueueOverlayView, SessionOverlayView, StartupOverlayView, SupersessionOverlayView,
    TaskIntakeOverlayView,
};
