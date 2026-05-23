#[path = "directions_copy.rs"]
// Copy owns the operator-facing wording; this router only decides which state facts each step exposes.
mod copy;
#[path = "directions_projection.rs"]
// Selection projection turns service summaries into stable rows before copy/layout styling is applied.
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

// Renderer contract for the directions maintenance overlay.
// The inline and popup renderers can lay out every step with the same panel slots because this DTO keeps
// chrome, summary, actions, diagnostics, and key guidance separated.
pub(crate) struct DirectionsMaintenanceOverlayView {
    // Header identifies the maintenance surface and anchors it in the shared Akra overlay chrome.
    pub(crate) header_lines: Vec<Line<'static>>,
    // Summary carries the current service snapshot: counts in overview, selected target context elsewhere.
    pub(crate) summary_lines: Vec<Line<'static>>,
    // Options are the active rows the controller also accepts keys for: overview actions or selectable directions.
    pub(crate) option_lines: Vec<Line<'static>>,
    // Status contains diagnostics that should not replace actions, such as parse errors or confirmation context.
    pub(crate) status_lines: Vec<Line<'static>>,
    // Key lines mirror the same step that shell_controller routes to handle_directions_overlay_key.
    pub(crate) key_lines: Vec<Line<'static>>,
}

// Top-level presentation boundary for directions maintenance.
// Controller code mutates DirectionsMaintenanceOverlayUiState from key events; this function reads that
// state and the last application-service summary, then lowers them into renderer-ready lines without
// starting file IO or changing the selected action.
pub(crate) fn build_directions_maintenance_overlay_view(
    app: &NativeTuiApp,
) -> DirectionsMaintenanceOverlayView {
    // The step is the shared state-machine axis between rendering and key handling.
    match app.directions_maintenance_overlay_ui_state.step() {
        // Overview summarizes the service's authority scan and queue-idle prompt health.
        DirectionsMaintenanceOverlayStep::Overview => {
            let summary = app.directions_maintenance_overlay_ui_state.summary();
            // Missing and broken counts are the operator's quick signal for whether detail-doc repair is needed.
            let missing_doc_count = summary
                .map(|summary| summary.missing_detail_doc_count)
                .unwrap_or_default();
            let broken_doc_count = summary
                .map(|summary| summary.broken_detail_doc_count)
                .unwrap_or_default();
            // Total count keeps the problem counts anchored to the size of the direction authority set.
            let total_direction_count =
                summary.map(|summary| summary.directions.len()).unwrap_or(0);
            // Queue-idle policy lives in the same maintenance surface because prompt recovery depends on directions.
            let queue_idle_policy = summary
                .map(|summary| summary.queue_idle_policy.label().to_string())
                .unwrap_or_else(|| "unknown".to_string());
            // Paths and parse errors use the queue inspection compaction limit so popup width stays predictable.
            let queue_idle_prompt = summary
                .and_then(|summary| summary.queue_idle_prompt_path.as_deref())
                .map(|path| compact_whitespace_detail(path, QUEUE_INSPECTION_NOTE_DETAIL_LIMIT))
                .unwrap_or_else(|| "<none>".to_string());
            // The service has already classified prompt health; presentation only forwards the label.
            let queue_idle_prompt_status = summary
                .map(|summary| summary.queue_idle_prompt_status.label())
                .unwrap_or("unknown");
            // Parse errors stay in status lines so they explain blocked actions without hiding the action menu.
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
        // DetailDocSelection presents only actionable directions, matching the controller's selection movement.
        DirectionsMaintenanceOverlayStep::DetailDocSelection => {
            let actionable_directions = app
                .directions_maintenance_overlay_ui_state
                .actionable_detail_doc_directions();
            let selected_direction = app
                .directions_maintenance_overlay_ui_state
                .selected_actionable_detail_doc_direction();
            // Projection keeps filtered-list cursor rules out of copy text and out of renderer layout.
            let projection = build_detail_doc_selection_projection(
                actionable_directions.as_slice(),
                selected_direction,
            );

            build_detail_doc_selection_overlay_view(
                projection.option_lines,
                projection.selected_direction_title.as_deref(),
            )
        }
        // DetailDocConfirm renders the pending target snapshot captured before any editor/service action starts.
        DirectionsMaintenanceOverlayStep::DetailDocConfirm => {
            let pending = app
                .directions_maintenance_overlay_ui_state
                .pending_detail_doc_creation();
            // Title confirms the human target; id is the stable key the controller will pass to the editor flow.
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
                    // The choice controls both highlighted copy and the Enter behavior in the controller.
                    .detail_doc_confirm_choice(),
            )
        }
        // ManualEditor is rendered by the draft editor path; this static view is only a fallback contract.
        DirectionsMaintenanceOverlayStep::ManualEditor => build_manual_editor_overlay_view(),
    }
}

#[cfg(test)]
mod tests {
    use super::super::super::{AkraTheme, DetailDocConfirmChoice};
    use super::*;
    use crate::application::service::planning::{
        DirectionsMaintenanceDirectionSummary, DirectionsSupportingFileStatus,
    };

    #[test]
    fn overview_copy_reports_parse_error_and_disables_unsafe_detail_repair() {
        let view = build_overview_overlay_view(
            1,
            2,
            4,
            "review",
            "broken",
            ".codex-exec-loop/planning/prompts/queue-idle.md",
            Some("line 3: invalid direction"),
        );

        assert!(
            section_text(&view.status_lines).contains("4 total / 1 missing docs / 2 broken docs")
        );
        assert!(section_text(&view.status_lines).contains("directions parse error"));
        assert!(section_text(&view.option_lines).contains("repair detail docs"));
        assert_eq!(view.option_lines[1].spans[0].style, AkraTheme::subtle());
        assert_eq!(view.option_lines[2].spans[0].style, AkraTheme::subtle());
    }

    #[test]
    fn detail_doc_projection_marks_selected_direction_by_stable_id() {
        let missing = direction_summary(
            "dir-alpha",
            "Alpha",
            None,
            DirectionsSupportingFileStatus::MissingMapping,
        );
        let broken = direction_summary(
            "dir-beta",
            "Beta",
            Some(".codex-exec-loop/planning/directions/dir-beta.md"),
            DirectionsSupportingFileStatus::BrokenMapping,
        );

        let projection = build_detail_doc_selection_projection(&[&missing, &broken], Some(&broken));

        assert_eq!(projection.selected_direction_title.as_deref(), Some("Beta"));
        assert!(
            line_text(&projection.option_lines[0])
                .contains("id=dir-alpha / status=unset / path=<unset>")
        );
        assert!(line_text(&projection.option_lines[1]).contains(
            "id=dir-beta / status=broken / path=.codex-exec-loop/planning/directions/dir-beta.md"
        ));
        assert_eq!(
            projection.option_lines[1].spans[0].style,
            AkraTheme::selected()
        );

        let empty_projection = build_detail_doc_selection_projection(&[], None);
        assert!(
            line_text(&empty_projection.option_lines[0])
                .contains("Every direction already has a healthy detail-doc mapping")
        );
        assert_eq!(empty_projection.selected_direction_title, None);
    }

    #[test]
    fn detail_doc_copy_keeps_selection_confirm_and_editor_steps_distinct() {
        let selection =
            build_detail_doc_selection_overlay_view(vec![Line::from("Dir row")], Some("Alpha"));
        assert!(section_text(&selection.status_lines).contains("selected: Alpha"));
        assert!(section_text(&selection.summary_lines).contains("detail_doc_path"));

        let confirm = build_detail_doc_confirm_overlay_view(
            "Alpha",
            "dir-alpha",
            DetailDocConfirmChoice::Yes,
        );
        assert!(section_text(&confirm.summary_lines).contains("direction: Alpha"));
        assert!(
            section_text(&confirm.summary_lines)
                .contains(".codex-exec-loop/planning/directions/dir-alpha.md")
        );
        assert_eq!(
            confirm.option_lines[0].spans[0].style,
            AkraTheme::selected()
        );

        let manual = build_manual_editor_overlay_view();
        assert!(section_text(&manual.header_lines).contains("staged editor"));
        assert!(section_text(&manual.option_lines).contains("Ctrl+S to save + validate"));
    }

    fn direction_summary(
        id: &str,
        title: &str,
        path: Option<&str>,
        status: DirectionsSupportingFileStatus,
    ) -> DirectionsMaintenanceDirectionSummary {
        DirectionsMaintenanceDirectionSummary {
            id: id.to_string(),
            title: title.to_string(),
            detail_doc_path: path.map(str::to_string),
            detail_doc_status: status,
        }
    }

    fn section_text(lines: &[Line<'_>]) -> String {
        lines.iter().map(line_text).collect::<Vec<_>>().join("\n")
    }

    fn line_text(line: &Line<'_>) -> String {
        line.spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect::<String>()
    }
}
