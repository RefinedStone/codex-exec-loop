#[path = "popup/base.rs"]
mod base;
#[path = "popup/planning.rs"]
mod planning;
#[path = "popup/queue.rs"]
mod queue;
#[path = "popup/supersession.rs"]
mod supersession;

pub(crate) use base::{build_session_overlay_view, build_startup_overlay_view};
pub(crate) use planning::{
    build_planning_draft_editor_overlay_view, build_planning_init_overlay_view,
    planning_init_option_line,
};
pub(crate) use queue::{build_automation_overlay_view, build_queue_overlay_view};
pub(crate) use supersession::build_supersession_overlay_view;
