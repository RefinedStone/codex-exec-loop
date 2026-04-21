#[path = "planning_copy.rs"]
mod copy;
#[path = "planning_inputs.rs"]
mod inputs;
#[path = "planning_projection.rs"]
mod projection;
#[path = "planning_runtime.rs"]
mod runtime;
#[path = "planning_session.rs"]
mod session;

use super::super::super::{ConversationState, NativeTuiApp, PlanningInitOverlayStep};
use super::{PlanningDraftEditorOverlayView, PlanningInitOverlayView};
use copy::{
    build_detail_selection_overlay_view, build_existing_workspace_overlay_view,
    build_manual_editor_overlay_view, build_mode_selection_overlay_view,
    build_planning_draft_editor_header_lines, build_planning_draft_editor_key_lines,
    build_planning_draft_editor_status_lines, build_simple_review_overlay_view,
};
use inputs::{
    build_existing_workspace_copy, build_planning_draft_editor_status_copy,
    build_simple_review_copy,
};
use projection::build_planning_draft_editor_projection;
use runtime::interpret_planning_draft_editor_runtime_state;
use session::collect_planning_draft_editor_session_view;

pub(crate) fn build_planning_init_overlay_view(app: &NativeTuiApp) -> PlanningInitOverlayView {
    match app.planning_init_overlay_ui_state.step() {
        PlanningInitOverlayStep::ExistingWorkspace => {
            let workspace_directory = app.planning_workspace_directory();
            let snapshot = match &app.conversation_state {
                ConversationState::Ready(conversation) => {
                    conversation.planning_runtime_snapshot.clone()
                }
                ConversationState::Loading | ConversationState::Failed(_) => {
                    app.load_planning_runtime_snapshot(&workspace_directory)
                }
            };
            build_existing_workspace_overlay_view(build_existing_workspace_copy(
                &workspace_directory,
                &snapshot,
            ))
        }
        PlanningInitOverlayStep::ModeSelection => {
            build_mode_selection_overlay_view(app.planning_init_overlay_ui_state.selected_mode())
        }
        PlanningInitOverlayStep::DetailSelection => build_detail_selection_overlay_view(
            app.planning_init_overlay_ui_state.selected_detail(),
        ),
        PlanningInitOverlayStep::SimpleReview => {
            build_simple_review_overlay_view(build_simple_review_copy(app))
        }
        PlanningInitOverlayStep::ManualEditor => build_manual_editor_overlay_view(),
    }
}

pub(crate) fn build_planning_draft_editor_overlay_view(
    app: &NativeTuiApp,
    editor_height: u16,
) -> Option<PlanningDraftEditorOverlayView> {
    let session = collect_planning_draft_editor_session_view(&app.planning_draft_editor_ui_state)?;
    let runtime_state = interpret_planning_draft_editor_runtime_state(
        &app.planning_draft_editor_ui_state,
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
