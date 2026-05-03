// Task intake popup is a pure projection of controller-owned modal state.
// The controller talks to planning runtime; this layer only turns the prompt/proposal/result snapshot into shared popup copy.
use super::super::super::{AkraTheme, Line, NativeTuiApp, Span, TaskIntakeOverlayStep};
use super::TaskIntakeOverlayView;

// Build the single view DTO used by both popup rendering and inline inspection rendering.
pub(crate) fn build_task_intake_overlay_view(app: &NativeTuiApp) -> TaskIntakeOverlayView {
    // Rendering reads a stable modal snapshot; all mutation belongs to shell_controller key handling and runtime callbacks.
    let state = &app.task_intake_overlay_ui_state;
    let header_lines = vec![
        AkraTheme::title_line("Task Intake", " / runtime planning"),
        Line::from("Draft one ready task for the accepted planning queue."),
    ];
    // Echo the raw prompt as the user typed it; preview generation may trim it, but the modal should preserve editing shape.
    let prompt_lines = if state.prompt_buffer().trim().is_empty() {
        vec![Line::from("prompt: ")]
    } else {
        state
            .prompt_buffer()
            .lines()
            // Prefix every row so multiline prompt text keeps its identity after layout wrapping.
            .map(|line| Line::from(format!("prompt: {line}")))
            .collect()
    };
    // Preview area reflects the planning runtime proposal without reinterpreting its generated summary lines.
    let preview_lines = state
        .proposal()
        .map(|proposal| {
            let mut lines = proposal
                .preview_lines
                .iter()
                .map(|line| Line::from(line.clone()))
                .collect::<Vec<_>>();
            // The draft id is commit identity, so show it separately from human preview text.
            lines.push(Line::from(format!(
                "task_id: {}",
                proposal.draft.task.id.trim()
            )));
            lines
        })
        .unwrap_or_else(|| vec![Line::from("Preview appears after Enter.")]);

    // Status priority is action-result first, state second: errors and committed revisions should not be hidden by step copy.
    let mut status_lines = Vec::new();
    if let Some(error) = state.error() {
        status_lines.push(Line::from(vec![
            // Style only the severity label so service error text remains literal and easy to copy.
            Span::styled("error: ", AkraTheme::danger()),
            Span::raw(error.to_string()),
        ]));
    } else if let Some(result) = state.commit_result() {
        status_lines.push(Line::from(format!(
            "accepted: {} / revision {}",
            result.committed_task_id, result.committed_planning_revision
        )));
    } else {
        status_lines.push(Line::from(match state.step() {
            TaskIntakeOverlayStep::Prompt => "status: editing prompt",
            TaskIntakeOverlayStep::Preview => "status: preview ready",
        }));
    }

    TaskIntakeOverlayView {
        header_lines,
        prompt_lines,
        preview_lines,
        status_lines,
        // Key copy is generated from the same step enum as key handling, keeping visible commands and controller branches aligned.
        key_lines: build_task_intake_key_lines(state.step()),
    }
}

// Translate the modal state machine into the exact command vocabulary advertised to the operator.
fn build_task_intake_key_lines(step: TaskIntakeOverlayStep) -> Vec<Line<'static>> {
    match step {
        // Prompt mode edits raw text and can only ask the runtime to prepare a preview.
        TaskIntakeOverlayStep::Prompt => {
            vec![AkraTheme::key_line(
                "Enter preview  |  Ctrl+u clear  |  Esc cancel",
            )]
        }
        // Preview mode has a concrete proposal, so keys switch to commit/edit/cancel decisions.
        TaskIntakeOverlayStep::Preview => {
            vec![AkraTheme::key_line("Y commit  |  E edit  |  N/Esc cancel")]
        }
    }
}
