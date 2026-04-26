#[path = "overlays/base.rs"]
mod base;

#[path = "overlays/directions.rs"]
mod directions;

#[path = "overlays/help.rs"]
mod help;

#[path = "overlays/list_projection.rs"]
mod list_projection;

#[path = "overlays/option_lines.rs"]
mod option_lines;

#[path = "overlays/popup.rs"]
mod popup;

#[cfg(test)]
pub(crate) use base::build_conversation_shell_frame_view;
pub(crate) use base::build_startup_banner_lines;
pub(crate) use directions::{
    DirectionsMaintenanceOverlayView, build_directions_maintenance_overlay_view,
};
pub(crate) use help::{HelpOverlayView, build_help_overlay_view};
pub(crate) use list_projection::{OverlayListEntryView, OverlayListView};
pub(crate) use popup::{
    PlanningDraftEditorOverlayView, PlanningInitOverlayView, QueueOverlayView, SessionOverlayView,
    StartupOverlayView, SupersessionOverlayView, TaskIntakeOverlayView,
    build_planning_draft_editor_overlay_view, build_planning_init_overlay_view,
    build_queue_overlay_view, build_session_overlay_view, build_startup_overlay_view,
    build_supersession_overlay_view, build_task_intake_overlay_view,
};
