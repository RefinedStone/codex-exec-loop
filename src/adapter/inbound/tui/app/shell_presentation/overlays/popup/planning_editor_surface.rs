use super::super::super::super::NativeTuiApp;
use super::super::PlanningDraftEditorOverlayView;
use super::editor_copy::{
    build_planning_draft_editor_header_lines, build_planning_draft_editor_key_lines,
    build_planning_draft_editor_status_lines,
};
use super::editor_inputs::build_planning_draft_editor_status_copy;
use super::projection::build_planning_draft_editor_projection;
use super::runtime::interpret_planning_draft_editor_runtime_state;
use super::session::collect_planning_draft_editor_session_view;

// Manual planning editor surface is the last presentation assembly step before ratatui rendering.
// It reads app state without mutation, joins session/runtime/projection/copy helpers, and returns one frame-stable DTO.
pub(super) fn build_planning_draft_editor_overlay_view_for_app(
    // The shell app is only used as the owner of planning draft editor UI state; editor control stays elsewhere.
    app: &NativeTuiApp,
    // Layout-owned height flows into projection so scroll and cursor math use the same viewport as the renderer.
    editor_height: u16,
) -> Option<PlanningDraftEditorOverlayView> {
    // No session means the editor route is not actually renderable; returning None prevents a half-empty popup.
    let session = collect_planning_draft_editor_session_view(&app.planning_draft_editor_ui_state)?;

    // Runtime interpretation turns raw dirty/validation/confirmation flags into operator-facing decisions.
    // Status copy and key copy must consume the same interpretation so close-risk messaging cannot drift.
    let runtime_state = interpret_planning_draft_editor_runtime_state(
        &app.planning_draft_editor_ui_state,
        &session.dirty_labels,
        session.validation_report,
    );

    // Projection owns visual geometry: file rows, editor text, scroll clamp, and cursor viewport offset.
    // Keeping it separate lets this surface focus on field assembly and runtime/copy joins.
    let projection = build_planning_draft_editor_projection(
        session.buffers,
        session.selected_index,
        session.selected_buffer,
        editor_height,
    );

    // Status copy is the bridge between session metadata and runtime interpretation.
    // The selected file index becomes 1-based here because this is the user-facing copy boundary.
    let status_copy = build_planning_draft_editor_status_copy(
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
    );
    let status_lines = build_planning_draft_editor_status_lines(status_copy);

    // The renderer receives only this DTO, so every field must already reflect the same session snapshot and runtime state.
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
