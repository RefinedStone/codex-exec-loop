use super::super::super::super::super::planning_draft_editor_ui::PlanningDraftEditorBufferState;
use super::super::super::super::{Color, Line, Modifier, Span, Style};

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
                Style::default().fg(Color::Black).bg(Color::Cyan)
            } else if buffer.is_dirty() {
                Style::default().fg(Color::Yellow)
            } else {
                Style::default().fg(Color::White)
            };
            let marker = if selected { ">> " } else { "   " };
            Line::from(vec![
                Span::styled(marker, style),
                Span::styled(buffer.file_label(), style.add_modifier(Modifier::BOLD)),
                Span::styled(dirty_suffix, style),
            ])
        })
        .collect()
}
