#[path = "planning_copy.rs"]
mod copy;

use super::super::super::status_panels::plan_runtime_substate_label;
use super::super::super::{
    Color, ConversationState, FOOTER_NOTICE_DETAIL_LIMIT, Line, Modifier, NativeTuiApp,
    PlanningInitOverlayStep, Span, Style, compact_inline_detail,
};
use super::{PlanningDraftEditorOverlayView, PlanningInitOverlayView};
use copy::{
    PlanningDraftEditorIssueCopy, PlanningDraftEditorStatusCopy, PlanningExistingWorkspaceCopy,
    PlanningSimpleReviewCopy, build_detail_selection_overlay_view,
    build_existing_workspace_overlay_view, build_manual_editor_overlay_view,
    build_mode_selection_overlay_view, build_planning_draft_editor_header_lines,
    build_planning_draft_editor_key_lines, build_planning_draft_editor_status_lines,
    build_simple_review_overlay_view,
};

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
            let plan_state_label = if snapshot.plan_enabled() {
                format!("Plan on / {}", plan_runtime_substate_label(&snapshot))
            } else {
                "Plan off".to_string()
            };
            let queue_summary = snapshot
                .queue_summary()
                .map(|summary| compact_inline_detail(summary, FOOTER_NOTICE_DETAIL_LIMIT))
                .unwrap_or_else(|| "queue state unavailable".to_string());
            let failure_summary = snapshot
                .failure_reason()
                .map(|summary| compact_inline_detail(summary, FOOTER_NOTICE_DETAIL_LIMIT));
            build_existing_workspace_overlay_view(PlanningExistingWorkspaceCopy {
                workspace_directory: &workspace_directory,
                plan_state_label: &plan_state_label,
                queue_summary: &queue_summary,
                queue_idle_policy: snapshot.queue_idle_policy().label(),
                failure_summary: failure_summary.as_deref(),
                plan_enabled: snapshot.plan_enabled(),
            })
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

    let file_lines = buffers
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
            let marker = if selected { ">>" } else { "  " };
            Line::from(vec![
                Span::styled(format!("{marker} "), style),
                Span::styled(buffer.file_label(), style.add_modifier(Modifier::BOLD)),
                Span::styled(dirty_suffix, style),
            ])
        })
        .collect::<Vec<_>>();

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

    let status_lines = build_planning_draft_editor_status_lines(PlanningDraftEditorStatusCopy {
        draft_name: app
            .planning_draft_editor_ui_state
            .draft_name()
            .unwrap_or("unknown"),
        active_path: selected_buffer.active_path(),
        selected_file_position: selected_index + 1,
        file_count: buffers.len(),
        validation_ok: validation_report.is_valid(),
        first_issue: validation_report
            .issues
            .first()
            .map(|issue| PlanningDraftEditorIssueCopy {
                severity: issue.severity,
                detail: compact_inline_detail(&issue.message, FOOTER_NOTICE_DETAIL_LIMIT),
            }),
        staged_path_summary: compact_inline_detail(
            selected_buffer.staged_path(),
            FOOTER_NOTICE_DETAIL_LIMIT,
        ),
        dirty_label_summary: if dirty_labels.is_empty() {
            "none".to_string()
        } else {
            compact_inline_detail(&dirty_labels.join(", "), FOOTER_NOTICE_DETAIL_LIMIT)
        },
        has_dirty_labels: !dirty_labels.is_empty(),
        next_action,
        close_risk,
        confirmation_pending: pending_close_risk.is_some(),
    });

    Some(PlanningDraftEditorOverlayView {
        header_lines: build_planning_draft_editor_header_lines(
            app.planning_draft_editor_ui_state
                .draft_directory()
                .unwrap_or("unknown"),
        ),
        file_lines,
        editor_title: selected_buffer.file_label(),
        editor_lines,
        editor_scroll,
        editor_cursor_offset,
        status_lines,
        key_lines: build_planning_draft_editor_key_lines(close_risk, pending_close_risk.is_some()),
    })
}
