use super::super::super::{
    Color, Line, Modifier, NativeTuiApp, Span, Style, TaskIntakeOverlayStep,
};
use super::TaskIntakeOverlayView;

pub(crate) fn build_task_intake_overlay_view(app: &NativeTuiApp) -> TaskIntakeOverlayView {
    let state = &app.task_intake_overlay_ui_state;
    let header_lines = vec![
        Line::from(vec![
            Span::styled(
                "Task Intake",
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(" / runtime planning"),
        ]),
        Line::from("Draft one ready task for the accepted planning queue."),
    ];
    let prompt_lines = if state.prompt_buffer().trim().is_empty() {
        vec![Line::from("prompt: ")]
    } else {
        state
            .prompt_buffer()
            .lines()
            .map(|line| Line::from(format!("prompt: {line}")))
            .collect()
    };
    let preview_lines = state
        .proposal()
        .map(|proposal| {
            let mut lines = proposal
                .preview_lines
                .iter()
                .map(|line| Line::from(line.clone()))
                .collect::<Vec<_>>();
            lines.push(Line::from(format!(
                "task_id: {}",
                proposal.draft.task.id.trim()
            )));
            lines
        })
        .unwrap_or_else(|| vec![Line::from("Preview appears after Enter.")]);

    let mut status_lines = Vec::new();
    if let Some(error) = state.error() {
        status_lines.push(Line::from(vec![
            Span::styled("error: ", Style::default().fg(Color::Red)),
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
        key_lines: build_task_intake_key_lines(state.step()),
    }
}

fn build_task_intake_key_lines(step: TaskIntakeOverlayStep) -> Vec<Line<'static>> {
    match step {
        TaskIntakeOverlayStep::Prompt => {
            vec![Line::from("Enter preview  |  Ctrl+u clear  |  Esc cancel")]
        }
        TaskIntakeOverlayStep::Preview => {
            vec![Line::from("Y commit  |  E edit  |  N/Esc cancel")]
        }
    }
}
