#[path = "planning_copy.rs"]
mod copy;
#[path = "planning_inputs.rs"]
mod inputs;
#[path = "planning_projection.rs"]
mod projection;

use super::super::super::{
    ConversationState, FOOTER_NOTICE_DETAIL_LIMIT, NativeTuiApp, PlanningInitOverlayStep,
    compact_inline_detail,
};
use super::{PlanningDraftEditorOverlayView, PlanningInitOverlayView};
use copy::{
    PlanningSimpleReviewCopy, build_detail_selection_overlay_view,
    build_existing_workspace_overlay_view, build_manual_editor_overlay_view,
    build_mode_selection_overlay_view, build_planning_draft_editor_header_lines,
    build_planning_draft_editor_key_lines, build_planning_draft_editor_status_lines,
    build_simple_review_overlay_view,
};
use inputs::{build_existing_workspace_copy, build_planning_draft_editor_status_copy};
use projection::build_planning_draft_editor_projection;

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
            let simple_review = app.planning_init_overlay_ui_state.simple_review();
            let draft_name = simple_review
                .map(|review| review.draft_name().to_string())
                .unwrap_or_else(|| "unknown".to_string());
            let staged_file_count = simple_review
                .map(|review| review.staged_file_count())
                .unwrap_or_default();
            let validation_report = simple_review.map(|review| review.validation_report());
            let validation_ok = validation_report.is_none_or(|report| report.is_valid());
            let first_error = validation_report
                .and_then(|report| report.errors().into_iter().next())
                .map(|issue| {
                    compact_inline_detail(issue.message.as_str(), FOOTER_NOTICE_DETAIL_LIMIT)
                });
            let max_auto_turns_label = app.current_max_auto_turns_label();

            build_simple_review_overlay_view(PlanningSimpleReviewCopy {
                draft_name: &draft_name,
                staged_file_count,
                validation_ok,
                first_error: first_error.as_deref(),
                max_auto_turns_label: &max_auto_turns_label,
                is_turn_budget_editing: app.is_max_auto_turns_editing(),
                turn_budget_buffer: &app.followup_overlay_ui_state.max_auto_turns_editor.buffer,
            })
        }
        PlanningInitOverlayStep::ManualEditor => build_manual_editor_overlay_view(),
    }
}

pub(crate) fn build_planning_draft_editor_overlay_view(
    app: &NativeTuiApp,
    editor_height: u16,
) -> Option<PlanningDraftEditorOverlayView> {
    let buffers = app.planning_draft_editor_ui_state.buffers()?;
    let selected_index = app.planning_draft_editor_ui_state.selected_file_index()?;
    let selected_buffer = app.planning_draft_editor_ui_state.selected_buffer()?;
    let dirty_labels = app.planning_draft_editor_ui_state.dirty_file_labels();
    let validation_report = app.planning_draft_editor_ui_state.validation_report()?;
    let pending_close_risk = app.planning_draft_editor_ui_state.pending_close_risk();
    let close_risk = pending_close_risk.or_else(|| app.planning_draft_editor_ui_state.close_risk());
    let next_action = if !dirty_labels.is_empty() {
        "next action: Ctrl+S re-runs validation, or Ctrl+P saves current edits and promotes if valid"
    } else if validation_report.is_valid() {
        "next action: Ctrl+P promotes this draft into active planning files"
    } else {
        "next action: fix validation errors before promoting this draft"
    };
    let projection = build_planning_draft_editor_projection(
        buffers,
        selected_index,
        selected_buffer,
        editor_height,
    );
    let status_lines =
        build_planning_draft_editor_status_lines(build_planning_draft_editor_status_copy(
            app.planning_draft_editor_ui_state
                .draft_name()
                .unwrap_or("unknown"),
            selected_buffer.active_path(),
            selected_index + 1,
            buffers.len(),
            validation_report,
            selected_buffer.staged_path(),
            &dirty_labels,
            next_action,
            close_risk,
            pending_close_risk.is_some(),
        ));

    Some(PlanningDraftEditorOverlayView {
        header_lines: build_planning_draft_editor_header_lines(
            app.planning_draft_editor_ui_state
                .draft_directory()
                .unwrap_or("unknown"),
        ),
        file_lines: projection.file_lines,
        editor_title: projection.editor_title,
        editor_lines: projection.editor_lines,
        editor_scroll: projection.editor_scroll,
        editor_cursor_offset: projection.editor_cursor_offset,
        status_lines,
        key_lines: build_planning_draft_editor_key_lines(close_risk, pending_close_risk.is_some()),
    })
}
