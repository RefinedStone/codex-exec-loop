use super::super::super::{
    AkraTheme, Line, MODEL_SELECTION_EFFORT_OPTIONS, MODEL_SELECTION_MODEL_OPTIONS,
    ModelSelectionStep,
};
use super::super::option_lines::overlay_option_line;
use super::ModelSelectionOverlayView;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ModelSelectionFrameInput<'a> {
    pub(crate) step: ModelSelectionStep,
    pub(crate) selected_model_index: usize,
    pub(crate) selected_effort_index: usize,
    pub(crate) staged_model_index: usize,
    pub(crate) staged_model_label: &'static str,
    pub(crate) current_model_label: &'a str,
    pub(crate) current_effort_label: &'static str,
}

pub(crate) fn build_model_selection_overlay_view(
    input: ModelSelectionFrameInput<'_>,
) -> ModelSelectionOverlayView {
    let model_lines = MODEL_SELECTION_MODEL_OPTIONS
        .iter()
        .enumerate()
        .map(|(index, option)| {
            let selected = match input.step {
                ModelSelectionStep::Model => input.selected_model_index == index,
                ModelSelectionStep::Effort => input.staged_model_index == index,
            };
            let detail =
                with_current_suffix(option.detail, input.current_model_label == option.label);
            overlay_option_line(
                &(index + 1).to_string(),
                option.label,
                &detail,
                selected,
                false,
            )
        })
        .collect();
    let effort_lines = MODEL_SELECTION_EFFORT_OPTIONS
        .iter()
        .enumerate()
        .map(|(index, option)| {
            let effort_label = option.label;
            let detail =
                with_current_suffix(option.detail, input.current_effort_label == effort_label);
            overlay_option_line(
                &(index + 1).to_string(),
                effort_label,
                &detail,
                input.selected_effort_index == index,
                input.step == ModelSelectionStep::Model,
            )
        })
        .collect();

    ModelSelectionOverlayView {
        header_lines: vec![
            AkraTheme::title_line("Select Model and Effort", " / focused view"),
            Line::from("Choose a model, then choose the think level for future turns."),
        ],
        model_lines,
        effort_lines,
        status_lines: build_model_selection_status_lines(input),
        key_lines: build_model_selection_key_lines(input.step),
    }
}

fn build_model_selection_status_lines(input: ModelSelectionFrameInput<'_>) -> Vec<Line<'static>> {
    match input.step {
        ModelSelectionStep::Model => vec![
            Line::from(format!(
                "current: model {}  |  think {}",
                input.current_model_label, input.current_effort_label
            )),
            Line::from("Enter chooses the highlighted model and moves to think level."),
        ],
        ModelSelectionStep::Effort => vec![
            Line::from(format!("selected model: {}", input.staged_model_label)),
            Line::from("Enter applies the highlighted think level with the selected model."),
        ],
    }
}

fn build_model_selection_key_lines(step: ModelSelectionStep) -> Vec<Line<'static>> {
    match step {
        ModelSelectionStep::Model => vec![
            AkraTheme::key_line("Enter/1-7: choose model    j/k or Up/Down: move"),
            AkraTheme::key_line("Esc/Ctrl+C: close"),
        ],
        ModelSelectionStep::Effort => vec![
            AkraTheme::key_line("Enter/1-7: apply    j/k or Up/Down: move"),
            AkraTheme::key_line("Backspace/Left: model    Esc/Ctrl+C: close"),
        ],
    }
}

fn with_current_suffix(detail: &str, current: bool) -> String {
    if current {
        format!("{detail}  (current)")
    } else {
        detail.to_string()
    }
}
