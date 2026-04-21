use super::Line;

pub(crate) struct OverlayListEntryView {
    pub(crate) lines: Vec<Line<'static>>,
}

pub(crate) struct OverlayListView {
    pub(crate) message_lines: Option<Vec<Line<'static>>>,
    pub(crate) items: Vec<OverlayListEntryView>,
    pub(crate) selected_index: Option<usize>,
}

pub(crate) struct DirectionsMaintenanceOverlayView {
    pub(crate) header_lines: Vec<Line<'static>>,
    pub(crate) summary_lines: Vec<Line<'static>>,
    pub(crate) option_lines: Vec<Line<'static>>,
    pub(crate) status_lines: Vec<Line<'static>>,
    pub(crate) key_lines: Vec<Line<'static>>,
}

#[path = "overlays/base.rs"]
mod base;

#[path = "overlays/directions.rs"]
mod directions;

#[path = "overlays/popup.rs"]
mod popup;

pub(crate) use base::build_startup_banner_lines;
#[cfg(test)]
pub(crate) use base::{
    build_conversation_shell_frame_view, build_conversation_shell_view, build_transcript_panel_view,
};
pub(crate) use directions::build_directions_maintenance_overlay_view;
pub(crate) use popup::{
    AutomationOverlayView, PlanningDraftEditorOverlayView, PlanningInitOverlayView,
    QueueOverlayView, SessionOverlayView, StartupOverlayView, SupersessionOverlayView,
    build_automation_overlay_view, build_planning_draft_editor_overlay_view,
    build_planning_init_overlay_view, build_queue_overlay_view, build_session_overlay_view,
    build_startup_overlay_view, build_supersession_overlay_view,
};
