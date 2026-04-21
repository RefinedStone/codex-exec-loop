use super::super::super::super::super::Line;
use super::super::super::PlanningInitOverlayView;
use super::super::copy::{
    PlanningSimpleReviewCopy, planning_draft_title_line, planning_setup_title_line,
};

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
