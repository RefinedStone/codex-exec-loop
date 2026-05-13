use super::super::super::{
    AkraTheme, ConversationTurnOptions, Line, MODEL_SELECTION_EFFORT_OPTIONS,
    MODEL_SELECTION_MODEL_OPTIONS, ModelSelectionStep, NativeTuiApp,
};
use super::super::option_lines::overlay_option_line;
use super::ModelSelectionOverlayView;

pub(crate) fn build_model_selection_overlay_view(app: &NativeTuiApp) -> ModelSelectionOverlayView {
    let state = &app.model_selection_overlay_ui_state;
    let model_lines = MODEL_SELECTION_MODEL_OPTIONS
        .iter()
        .enumerate()
        .map(|(index, option)| {
            let selected = match state.step() {
                ModelSelectionStep::Model => state.selected_model_index() == index,
                ModelSelectionStep::Effort => state.staged_model_index() == index,
            };
            let detail =
                with_current_suffix(option.detail, current_model_label(app) == option.model);
            overlay_option_line(
                &(index + 1).to_string(),
                option.model,
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
            let effort_label = option.effort.label();
            let detail =
                with_current_suffix(option.detail, current_effort_label(app) == effort_label);
            overlay_option_line(
                &(index + 1).to_string(),
                effort_label,
                &detail,
                state.selected_effort_index() == index,
                state.step() == ModelSelectionStep::Model,
            )
        })
        .collect();

    ModelSelectionOverlayView {
        header_lines: vec![
            AkraTheme::title_line("Select Model and Effort", " / inline inspection"),
            Line::from("Choose a model, then choose the think level for future turns."),
        ],
        model_lines,
        effort_lines,
        status_lines: build_model_selection_status_lines(app),
        key_lines: build_model_selection_key_lines(state.step()),
    }
}

fn build_model_selection_status_lines(app: &NativeTuiApp) -> Vec<Line<'static>> {
    let state = &app.model_selection_overlay_ui_state;
    match state.step() {
        ModelSelectionStep::Model => vec![
            Line::from(format!(
                "current: model {}  |  think {}",
                current_model_label(app),
                current_effort_label(app)
            )),
            Line::from("Enter chooses the highlighted model and moves to think level."),
        ],
        ModelSelectionStep::Effort => vec![
            Line::from(format!("selected model: {}", state.staged_model().model)),
            Line::from("Enter applies the highlighted think level with the selected model."),
        ],
    }
}

fn build_model_selection_key_lines(step: ModelSelectionStep) -> Vec<Line<'static>> {
    match step {
        ModelSelectionStep::Model => vec![
            AkraTheme::key_line("Enter/1-6: choose model    j/k or Up/Down: move"),
            AkraTheme::key_line("Esc/Ctrl+C: close"),
        ],
        ModelSelectionStep::Effort => vec![
            AkraTheme::key_line("Enter/1-6: apply    j/k or Up/Down: move"),
            AkraTheme::key_line("Backspace/Left: model    Esc/Ctrl+C: close"),
        ],
    }
}

fn current_model_label(app: &NativeTuiApp) -> &str {
    app.turn_options
        .model
        .as_deref()
        .unwrap_or(ConversationTurnOptions::DEFAULT_MODEL)
}

fn current_effort_label(app: &NativeTuiApp) -> &str {
    app.turn_options
        .reasoning_effort
        .unwrap_or(ConversationTurnOptions::DEFAULT_REASONING_EFFORT)
        .label()
}

fn with_current_suffix(detail: &str, current: bool) -> String {
    if current {
        format!("{detail}  (current)")
    } else {
        detail.to_string()
    }
}
