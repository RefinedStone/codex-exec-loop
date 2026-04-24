use super::super::super::super::super::planning_draft_editor_ui::PlanningDraftEditorBufferState;
use super::super::super::super::{AkraTheme, Line, Modifier, Span, Style};

pub(super) fn build_planning_draft_editor_file_lines(
    buffers: &[PlanningDraftEditorBufferState],
    selected_index: usize,
) -> Vec<Line<'static>> {
    buffers
        .iter()
        .enumerate()
        .map(|(index, buffer)| {
            let selected = index == selected_index;
            let dirty_suffix = if buffer.is_dirty() { " *dirty" } else { "" };
            let style = if selected {
                AkraTheme::selected()
            } else if buffer.is_dirty() {
                AkraTheme::warning()
            } else {
                Style::default()
            };
            let marker = if selected {
                AkraTheme::selected_marker()
            } else {
                "  "
            };
            Line::from(vec![
                Span::styled(marker, style),
                Span::styled(buffer.file_label(), style.add_modifier(Modifier::BOLD)),
                Span::styled(dirty_suffix, style),
            ])
        })
        .collect()
}
