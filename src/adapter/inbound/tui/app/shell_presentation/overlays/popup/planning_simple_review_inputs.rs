use super::super::super::super::{FOOTER_NOTICE_DETAIL_LIMIT, NativeTuiApp, compact_inline_detail};
use super::copy::PlanningSimpleReviewCopy;

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
