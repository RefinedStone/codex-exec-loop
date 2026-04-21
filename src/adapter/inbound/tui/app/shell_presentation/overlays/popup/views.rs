use super::super::super::Line;
use super::super::OverlayListView;

pub(crate) struct StartupOverlayView {
    pub(crate) header_lines: Vec<Line<'static>>,
    pub(crate) summary_lines: Vec<Line<'static>>,
    pub(crate) check_lines: Vec<Line<'static>>,
    pub(crate) warning_lines: Vec<Line<'static>>,
    pub(crate) key_lines: Vec<Line<'static>>,
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
