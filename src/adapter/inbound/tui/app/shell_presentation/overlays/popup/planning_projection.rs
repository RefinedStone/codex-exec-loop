use super::super::super::super::super::planning_draft_editor_ui::PlanningDraftEditorBufferState;
use super::super::super::super::Line;
use super::projection_lines::build_planning_draft_editor_file_lines;

pub(super) struct PlanningDraftEditorProjection {
    pub(super) file_lines: Vec<Line<'static>>,
    pub(super) editor_title: String,
    pub(super) editor_lines: Vec<Line<'static>>,
    pub(super) editor_scroll: u16,
    pub(super) editor_cursor_offset: Option<(u16, u16)>,
}

pub(super) fn build_planning_draft_editor_projection(
    buffers: &[PlanningDraftEditorBufferState],
    selected_index: usize,
    selected_buffer: &PlanningDraftEditorBufferState,
    editor_height: u16,
) -> PlanningDraftEditorProjection {
    let file_lines = build_planning_draft_editor_file_lines(buffers, selected_index);

    let editor_lines = selected_buffer
        .lines()
        .iter()
        .map(|line| Line::from(line.clone()))
        .collect::<Vec<_>>();
    let editor_height = editor_height.max(1) as usize;
    let max_editor_scroll = selected_buffer
        .lines()
        .len()
        .saturating_sub(editor_height)
        .min(u16::MAX as usize) as u16;
    let editor_scroll = selected_buffer.editor_scroll().min(max_editor_scroll);
    let editor_cursor_offset = Some((
        selected_buffer.cursor_column().min(u16::MAX as usize) as u16,
        selected_buffer
            .cursor_line_index()
            .saturating_sub(editor_scroll as usize)
            .min(u16::MAX as usize) as u16,
    ));

    PlanningDraftEditorProjection {
        file_lines,
        editor_title: selected_buffer.file_label(),
        editor_lines,
        editor_scroll,
        editor_cursor_offset,
    }
}
