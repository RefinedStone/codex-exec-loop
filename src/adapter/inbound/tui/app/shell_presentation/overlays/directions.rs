#[path = "directions_copy.rs"]
mod copy;
#[path = "directions_projection.rs"]
mod projection;

use super::super::{
    DirectionsMaintenanceOverlayStep, Line, NativeTuiApp, QUEUE_INSPECTION_NOTE_DETAIL_LIMIT,
    compact_whitespace_detail,
};
use copy::{
    build_detail_doc_confirm_overlay_view, build_detail_doc_selection_overlay_view,
    build_manual_editor_overlay_view, build_overview_overlay_view,
};
use projection::build_detail_doc_selection_projection;

pub(crate) struct DirectionsMaintenanceOverlayView {
    pub(crate) header_lines: Vec<Line<'static>>,
    pub(crate) summary_lines: Vec<Line<'static>>,
    pub(crate) option_lines: Vec<Line<'static>>,
    pub(crate) status_lines: Vec<Line<'static>>,
    pub(crate) key_lines: Vec<Line<'static>>,
}

pub(crate) fn build_directions_maintenance_overlay_view(
    app: &NativeTuiApp,
) -> DirectionsMaintenanceOverlayView {
    match app.directions_maintenance_overlay_ui_state.step() {
        DirectionsMaintenanceOverlayStep::Overview => {
            let summary = app.directions_maintenance_overlay_ui_state.summary();
            let missing_doc_count = summary
                .map(|summary| summary.missing_detail_doc_count)
                .unwrap_or_default();
            let broken_doc_count = summary
                .map(|summary| summary.broken_detail_doc_count)
                .unwrap_or_default();
            let total_direction_count =
                summary.map(|summary| summary.directions.len()).unwrap_or(0);
            let queue_idle_policy = summary
                .map(|summary| summary.queue_idle_policy.label().to_string())
                .unwrap_or_else(|| "unknown".to_string());
            let queue_idle_prompt = summary
                .and_then(|summary| summary.queue_idle_prompt_path.as_deref())
                .map(|path| compact_whitespace_detail(path, QUEUE_INSPECTION_NOTE_DETAIL_LIMIT))
                .unwrap_or_else(|| "<none>".to_string());
            let queue_idle_prompt_status = summary
                .map(|summary| summary.queue_idle_prompt_status.label())
                .unwrap_or("unknown");
            let parse_error_summary = summary
                .and_then(|summary| summary.parse_error.as_deref())
                .map(|error| compact_whitespace_detail(error, QUEUE_INSPECTION_NOTE_DETAIL_LIMIT));

            build_overview_overlay_view(
                missing_doc_count,
                broken_doc_count,
                total_direction_count,
                &queue_idle_policy,
                queue_idle_prompt_status,
                &queue_idle_prompt,
                parse_error_summary.as_deref(),
            )
        }
        DirectionsMaintenanceOverlayStep::DetailDocSelection => {
            let actionable_directions = app
                .directions_maintenance_overlay_ui_state
                .actionable_detail_doc_directions();
            let selected_direction = app
                .directions_maintenance_overlay_ui_state
                .selected_actionable_detail_doc_direction();
            let projection = build_detail_doc_selection_projection(
                actionable_directions.as_slice(),
                selected_direction.map(|direction| direction.id.as_str()),
            );

            build_detail_doc_selection_overlay_view(
                projection.option_lines,
                projection.selected_direction_title.as_deref(),
            )
        }
        DirectionsMaintenanceOverlayStep::DetailDocConfirm => {
            let pending = app
                .directions_maintenance_overlay_ui_state
                .pending_detail_doc_creation();
            let direction_id = pending
                .map(|pending| pending.direction_id())
                .unwrap_or("unknown");
            let direction_title = pending
                .map(|pending| pending.direction_title())
                .unwrap_or("unknown");

            build_detail_doc_confirm_overlay_view(
                direction_title,
                direction_id,
                app.directions_maintenance_overlay_ui_state
                    .detail_doc_confirm_choice(),
            )
        }
        DirectionsMaintenanceOverlayStep::ManualEditor => build_manual_editor_overlay_view(),
    }
}
