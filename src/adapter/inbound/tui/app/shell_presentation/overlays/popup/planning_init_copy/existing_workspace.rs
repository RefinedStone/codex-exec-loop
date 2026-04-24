use super::super::super::super::super::{AkraTheme, Line};
use super::super::super::PlanningInitOverlayView;
use super::super::copy::{PlanningExistingWorkspaceCopy, planning_setup_title_line};

pub(super) fn build_existing_workspace_overlay_view(
    copy: PlanningExistingWorkspaceCopy,
) -> PlanningInitOverlayView {
    let mut status_lines = if copy.plan_enabled {
        vec![
            Line::from("Enter opens queue inspection for the existing planning workspace."),
            Line::from("Press D to maintain directions, or O to turn Plan off."),
        ]
    } else {
        vec![
            Line::from("Enter turns Plan on and resumes the existing planning workspace."),
            Line::from("Directions maintenance stays blocked while Plan off."),
        ]
    };
    if let Some(failure_summary) = copy.failure_summary.as_deref() {
        status_lines.push(Line::from(format!("planning failure: {failure_summary}")));
    }

    PlanningInitOverlayView {
        header_lines: vec![
            planning_setup_title_line(" / existing workspace"),
            Line::from(
                "This workspace already has active planning files. Manage the current runtime instead of restaging a bootstrap scaffold.",
            ),
        ],
        summary_lines: vec![
            Line::from(
                "Use :directions only after Plan on. Hidden planner sessions still update task-ledger.json only.",
            ),
            Line::from(
                "Turning Plan off keeps the workspace files on disk and blocks directions maintenance until planning resumes.",
            ),
        ],
        option_lines: vec![
            Line::from(format!("workspace: {}", copy.workspace_directory)),
            Line::from(format!("planning state: {}", copy.plan_state_label)),
            Line::from(format!("queue state: {}", copy.queue_summary)),
            Line::from(format!("queue idle policy: {}", copy.queue_idle_policy)),
        ],
        status_lines,
        key_lines: vec![
            AkraTheme::key_line("Enter opens queue inspection or resumes Plan on."),
            AkraTheme::key_line("Q opens queue inspection. D opens directions maintenance."),
            AkraTheme::key_line("O toggles Plan on or off. Esc/Ctrl+C closes this surface."),
        ],
    }
}
