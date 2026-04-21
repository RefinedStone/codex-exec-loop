use super::super::Line;

pub(crate) struct OverlayListEntryView {
    pub(crate) lines: Vec<Line<'static>>,
}

pub(crate) struct OverlayListView {
    pub(crate) message_lines: Option<Vec<Line<'static>>>,
    pub(crate) items: Vec<OverlayListEntryView>,
    pub(crate) selected_index: Option<usize>,
}
