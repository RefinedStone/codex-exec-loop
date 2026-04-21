use super::super::super::super::{Line, PlanningInitDetailSelection, PlanningInitModeSelection};
use super::super::super::option_lines::overlay_option_line;
use super::super::PlanningInitOverlayView;
use super::copy::{
    PlanningExistingWorkspaceCopy, PlanningSimpleReviewCopy, planning_draft_title_line,
    planning_setup_title_line,
};

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
            Line::from("Enter opens queue inspection or resumes Plan on."),
            Line::from("Q opens queue inspection. D opens directions maintenance."),
            Line::from("O toggles Plan on or off. Esc/Ctrl+C closes this surface."),
        ],
    }
}

pub(super) fn build_mode_selection_overlay_view(
    selected_mode: PlanningInitModeSelection,
) -> PlanningInitOverlayView {
    PlanningInitOverlayView {
        header_lines: vec![
            planning_setup_title_line(" / operator inspection"),
            Line::from("Pick the planning entry path before any files are staged."),
        ],
        summary_lines: vec![
            Line::from(
                "Every guided path stages a promotable draft before active planning changes.",
            ),
            Line::from(
                "Simple mode keeps one generic active direction; detail mode prepares richer direction authoring.",
            ),
        ],
        option_lines: vec![
            overlay_option_line(
                "A",
                "simple mode",
                "stage one generic direction and an empty task ledger",
                selected_mode == PlanningInitModeSelection::Simple,
                false,
            ),
            overlay_option_line(
                "B",
                "detail mode",
                "branch into manual or future llm-assisted authoring",
                selected_mode == PlanningInitModeSelection::Detail,
                false,
            ),
        ],
        status_lines: vec![
            Line::from(format!(
                "current selection: {}",
                match selected_mode {
                    PlanningInitModeSelection::Simple => "simple mode",
                    PlanningInitModeSelection::Detail => "detail mode",
                }
            )),
            Line::from("simple mode is the low-ceremony path for planning-aware execution."),
        ],
        key_lines: vec![
            Line::from("A/B or arrows move selection."),
            Line::from("Enter continues. Esc/Ctrl+C cancels."),
        ],
    }
}

pub(super) fn build_detail_selection_overlay_view(
    selected_detail: PlanningInitDetailSelection,
) -> PlanningInitOverlayView {
    PlanningInitOverlayView {
        header_lines: vec![
            planning_setup_title_line(" / operator inspection"),
            Line::from("Current step: choose how detail-mode drafts should be prepared."),
        ],
        summary_lines: vec![
            Line::from("Manual opens the staged draft editor inside the shell."),
            Line::from("LLM-assisted remains visible for the target UX but is still disabled."),
        ],
        option_lines: vec![
            overlay_option_line(
                "A",
                "manual",
                "stage the detail scaffold and keep editing inside the shell",
                selected_detail == PlanningInitDetailSelection::Manual,
                false,
            ),
            overlay_option_line(
                "B",
                "llm-assisted",
                "future guided drafting flow (not supported yet)",
                selected_detail == PlanningInitDetailSelection::LlmAssisted,
                true,
            ),
        ],
        status_lines: vec![
            Line::from(format!(
                "current selection: {}",
                match selected_detail {
                    PlanningInitDetailSelection::Manual => "manual",
                    PlanningInitDetailSelection::LlmAssisted => "llm-assisted (disabled)",
                }
            )),
            Line::from("Enter on manual opens the embedded draft editor."),
        ],
        key_lines: vec![
            Line::from("A/B or arrows move selection."),
            Line::from("Backspace/Left goes back. Enter continues. Esc/Ctrl+C cancels."),
        ],
    }
}

pub(super) fn build_simple_review_overlay_view(
    copy: PlanningSimpleReviewCopy,
) -> PlanningInitOverlayView {
    let mut status_lines = vec![
        Line::from(format!(
            "validation state: {}",
            if copy.validation_ok {
                "ok"
            } else {
                "needs attention"
            }
        )),
        Line::from(format!("turn budget: {}", copy.max_auto_turns_label)),
    ];
    if copy.is_turn_budget_editing {
        status_lines.push(Line::from(format!(
            "current state: editing turn budget / value: {} / controls: Enter saves, Esc/Ctrl+C cancels",
            copy.turn_budget_buffer
        )));
    } else {
        status_lines.push(Line::from(
            "next action: Enter or Ctrl+P promotes the staged simple scaffold.",
        ));
        status_lines.push(Line::from(
            "alternate action: Esc closes this review and leaves the staged draft on disk.",
        ));
        status_lines.push(Line::from(
            "advanced action: D opens detail-mode authoring without promoting the simple scaffold.",
        ));
    }
    if let Some(first_error) = copy.first_error.as_deref() {
        status_lines.push(Line::from(format!("first validation error: {first_error}")));
    }

    PlanningInitOverlayView {
        header_lines: vec![
            planning_setup_title_line(" / operator inspection"),
            Line::from(
                "Simple mode review: promote the lightest planning baseline before you invest in richer authoring.",
            ),
        ],
        summary_lines: vec![
            Line::from(
                "After promote, planning starts with one generic direction and no active queue task yet.",
            ),
            Line::from(
                "The default queue-idle review prompt is already staged so the first reply can justify follow-up work when needed.",
            ),
            Line::from("No active planning files change until you explicitly promote this review."),
        ],
        option_lines: vec![
            Line::from(format!("staged draft: {}", copy.draft_name)),
            Line::from(format!(
                "reviewed artifacts: {} staged planning files",
                copy.staged_file_count
            )),
            Line::from(
                "promote outcome: generic direction catalog, empty task ledger, and default queue-idle review prompt",
            ),
            Line::from(
                "advanced path: press D to branch into detail-mode authoring instead of promoting the simple scaffold",
            ),
        ],
        status_lines,
        key_lines: if copy.is_turn_budget_editing {
            vec![
                Line::from("next action: type the new turn budget directly."),
                Line::from("controls: Enter saves  |  Esc/Ctrl+C cancels  |  Backspace deletes"),
                Line::from("validation: use a whole number greater than 0, or type infinite."),
            ]
        } else {
            vec![
                Line::from("Enter or Ctrl+P promotes the staged scaffold."),
                Line::from(
                    "D opens detail-mode authoring. Ctrl+L edits turn budget. Ctrl+E inspects or edits the draft.",
                ),
                Line::from("Esc/Ctrl+C closes this review."),
            ]
        },
    }
}

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
