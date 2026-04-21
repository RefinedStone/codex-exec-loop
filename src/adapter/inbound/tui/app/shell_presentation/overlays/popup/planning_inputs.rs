use crate::application::service::planning::PlanningRuntimeSnapshot;
use crate::domain::planning::PlanningValidationReport;

use super::super::super::super::super::planning_draft_editor_ui::PlanningDraftEditorCloseRisk;
use super::super::super::super::status_panels::plan_runtime_substate_label;
use super::super::super::super::{FOOTER_NOTICE_DETAIL_LIMIT, NativeTuiApp, compact_inline_detail};
use super::copy::{
    PlanningDraftEditorIssueCopy, PlanningDraftEditorStatusCopy, PlanningExistingWorkspaceCopy,
    PlanningSimpleReviewCopy,
};

pub(super) fn build_existing_workspace_copy(
    workspace_directory: &str,
    snapshot: &PlanningRuntimeSnapshot,
) -> PlanningExistingWorkspaceCopy {
    let plan_state_label = if snapshot.plan_enabled() {
        format!("Plan on / {}", plan_runtime_substate_label(snapshot))
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

    PlanningExistingWorkspaceCopy {
        workspace_directory: workspace_directory.to_string(),
        plan_state_label,
        queue_summary,
        queue_idle_policy: snapshot.queue_idle_policy().label().to_string(),
        failure_summary,
        plan_enabled: snapshot.plan_enabled(),
    }
}

pub(super) fn build_planning_draft_editor_status_copy(
    draft_name: &str,
    active_path: &str,
    selected_file_position: usize,
    file_count: usize,
    validation_report: &PlanningValidationReport,
    staged_path: &str,
    dirty_labels: &[String],
    next_action: &str,
    close_risk: Option<PlanningDraftEditorCloseRisk>,
    confirmation_pending: bool,
) -> PlanningDraftEditorStatusCopy {
    PlanningDraftEditorStatusCopy {
        draft_name: draft_name.to_string(),
        active_path: active_path.to_string(),
        selected_file_position,
        file_count,
        validation_ok: validation_report.is_valid(),
        first_issue: validation_report
            .issues
            .first()
            .map(|issue| PlanningDraftEditorIssueCopy {
                severity: issue.severity,
                detail: compact_inline_detail(&issue.message, FOOTER_NOTICE_DETAIL_LIMIT),
            }),
        staged_path_summary: compact_inline_detail(staged_path, FOOTER_NOTICE_DETAIL_LIMIT),
        dirty_label_summary: if dirty_labels.is_empty() {
            "none".to_string()
        } else {
            compact_inline_detail(&dirty_labels.join(", "), FOOTER_NOTICE_DETAIL_LIMIT)
        },
        has_dirty_labels: !dirty_labels.is_empty(),
        next_action: next_action.to_string(),
        close_risk,
        confirmation_pending,
    }
}

pub(super) fn build_simple_review_copy(app: &NativeTuiApp) -> PlanningSimpleReviewCopy {
    let simple_review = app.planning_init_overlay_ui_state.simple_review();
    let validation_report = simple_review.map(|review| review.validation_report());

    PlanningSimpleReviewCopy {
        draft_name: simple_review
            .map(|review| review.draft_name().to_string())
            .unwrap_or_else(|| "unknown".to_string()),
        staged_file_count: simple_review
            .map(|review| review.staged_file_count())
            .unwrap_or_default(),
        validation_ok: validation_report.is_none_or(|report| report.is_valid()),
        first_error: validation_report
            .and_then(|report| report.errors().into_iter().next())
            .map(|issue| compact_inline_detail(issue.message.as_str(), FOOTER_NOTICE_DETAIL_LIMIT)),
        max_auto_turns_label: app.current_max_auto_turns_label(),
        is_turn_budget_editing: app.is_max_auto_turns_editing(),
        turn_budget_buffer: app
            .followup_overlay_ui_state
            .max_auto_turns_editor
            .buffer
            .clone(),
    }
}
