use super::super::super::super::NativeTuiApp;
use super::super::PlanningDraftEditorOverlayView;
use super::editor_copy::{
    build_planning_draft_editor_header_lines, build_planning_draft_editor_key_lines,
    build_planning_draft_editor_status_lines,
};
use super::inputs::build_planning_draft_editor_status_copy;
use super::projection::build_planning_draft_editor_projection;
use super::runtime::interpret_planning_draft_editor_runtime_state;
use super::session::collect_planning_draft_editor_session_view;

pub(super) fn build_planning_draft_editor_overlay_view_for_app(
    app: &NativeTuiApp,
    editor_height: u16,
) -> Option<PlanningDraftEditorOverlayView> {
    let session = collect_planning_draft_editor_session_view(&app.planning_draft_editor_ui_state)?;
    let runtime_state = interpret_planning_draft_editor_runtime_state(
        app,
        &session.dirty_labels,
        session.validation_report,
    );
    let projection = build_planning_draft_editor_projection(
        session.buffers,
        session.selected_index,
        session.selected_buffer,
        editor_height,
    );
    let status_lines =
        build_planning_draft_editor_status_lines(build_planning_draft_editor_status_copy(
            session.draft_name,
            session.selected_buffer.active_path(),
            session.selected_index + 1,
            session.buffers.len(),
            session.validation_report,
            session.selected_buffer.staged_path(),
            &session.dirty_labels,
            runtime_state.next_action,
            runtime_state.close_risk,
            runtime_state.confirmation_pending,
        ));

    Some(PlanningDraftEditorOverlayView {
        header_lines: build_planning_draft_editor_header_lines(session.draft_directory),
        file_lines: projection.file_lines,
        editor_title: projection.editor_title,
        editor_lines: projection.editor_lines,
        editor_scroll: projection.editor_scroll,
        editor_cursor_offset: projection.editor_cursor_offset,
        status_lines,
        key_lines: build_planning_draft_editor_key_lines(
            runtime_state.close_risk,
            runtime_state.confirmation_pending,
        ),
    })
}
