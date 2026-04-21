use super::super::super::super::super::super::Line;
use super::super::super::PlanningInitOverlayView;
use super::super::super::copy::planning_draft_title_line;

pub(super) fn build_manual_editor_overlay_view() -> PlanningInitOverlayView {
    PlanningInitOverlayView {
        header_lines: vec![
            planning_draft_title_line(" / operator inspection"),
            Line::from("Edit the staged planning draft and save to re-run validation."),
        ],
        summary_lines: vec![Line::from(
            "This state renders through the dedicated planning draft editor view.",
        )],
        option_lines: vec![Line::from(
            "next action: Tab switches files. Ctrl+S saves and re-runs validation.",
        )],
        status_lines: vec![Line::from(
            "current state: editing the staged planning draft",
        )],
        key_lines: vec![Line::from("Esc/Ctrl+C closes this surface.")],
    }
}
