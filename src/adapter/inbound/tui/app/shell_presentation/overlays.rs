use super::Line;

pub(crate) struct StartupOverlayView {
    pub(crate) header_lines: Vec<Line<'static>>,
    pub(crate) summary_lines: Vec<Line<'static>>,
    pub(crate) check_lines: Vec<Line<'static>>,
    pub(crate) warning_lines: Vec<Line<'static>>,
    pub(crate) key_lines: Vec<Line<'static>>,
}

pub(crate) struct OverlayListEntryView {
    pub(crate) lines: Vec<Line<'static>>,
}

pub(crate) struct OverlayListView {
    pub(crate) message_lines: Option<Vec<Line<'static>>>,
    pub(crate) items: Vec<OverlayListEntryView>,
    pub(crate) selected_index: Option<usize>,
}

pub(crate) struct SessionOverlayView {
    pub(crate) header_lines: Vec<Line<'static>>,
    pub(crate) list_view: OverlayListView,
    pub(crate) detail_lines: Vec<Line<'static>>,
    pub(crate) warning_lines: Vec<Line<'static>>,
    pub(crate) key_lines: Vec<Line<'static>>,
}

pub(crate) struct SupersessionOverlayView {
    pub(crate) header_lines: Vec<Line<'static>>,
    pub(crate) summary_lines: Vec<Line<'static>>,
    pub(crate) capability_lines: Vec<Line<'static>>,
    pub(crate) pool_lines: Vec<Line<'static>>,
    pub(crate) roster_lines: Vec<Line<'static>>,
    pub(crate) detail_lines: Vec<Line<'static>>,
    pub(crate) distributor_lines: Vec<Line<'static>>,
    pub(crate) key_lines: Vec<Line<'static>>,
}

pub(crate) struct AutomationOverlayView {
    pub(crate) header_lines: Vec<Line<'static>>,
    pub(crate) list_view: OverlayListView,
    pub(crate) preview_lines: Vec<Line<'static>>,
    pub(crate) status_lines: Vec<Line<'static>>,
    pub(crate) key_lines: Vec<Line<'static>>,
}

pub(crate) struct QueueOverlayView {
    pub(crate) header_lines: Vec<Line<'static>>,
    pub(crate) summary_lines: Vec<Line<'static>>,
    pub(crate) queue_lines: Vec<Line<'static>>,
    pub(crate) proposal_lines: Vec<Line<'static>>,
    pub(crate) note_lines: Vec<Line<'static>>,
    pub(crate) key_lines: Vec<Line<'static>>,
}

pub(crate) struct PlanningInitOverlayView {
    pub(crate) header_lines: Vec<Line<'static>>,
    pub(crate) summary_lines: Vec<Line<'static>>,
    pub(crate) option_lines: Vec<Line<'static>>,
    pub(crate) status_lines: Vec<Line<'static>>,
    pub(crate) key_lines: Vec<Line<'static>>,
}

pub(crate) struct DirectionsMaintenanceOverlayView {
    pub(crate) header_lines: Vec<Line<'static>>,
    pub(crate) summary_lines: Vec<Line<'static>>,
    pub(crate) option_lines: Vec<Line<'static>>,
    pub(crate) status_lines: Vec<Line<'static>>,
    pub(crate) key_lines: Vec<Line<'static>>,
}

pub(crate) struct PlanningDraftEditorOverlayView {
    pub(crate) header_lines: Vec<Line<'static>>,
    pub(crate) file_lines: Vec<Line<'static>>,
    pub(crate) editor_title: String,
    pub(crate) editor_lines: Vec<Line<'static>>,
    pub(crate) editor_scroll: u16,
    pub(crate) editor_cursor_offset: Option<(u16, u16)>,
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
    build_automation_overlay_view, build_planning_draft_editor_overlay_view,
    build_planning_init_overlay_view, build_queue_overlay_view, build_session_overlay_view,
    build_startup_overlay_view, build_supersession_overlay_view,
};
