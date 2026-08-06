use super::super::super::{
    AkraTheme, Line, MODEL_SELECTION_MODEL_OPTIONS, ModelSelectionStep,
    model_selection_effort_option,
};
use super::super::option_lines::overlay_option_line;
use super::ModelSelectionOverlayView;
use ratatui::text::Span;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ModelSelectionFrameInput<'a> {
    pub(crate) step: ModelSelectionStep,
    pub(crate) selected_model_index: usize,
    pub(crate) selected_effort_index: usize,
    pub(crate) staged_model_index: usize,
    pub(crate) current_model_label: &'a str,
    pub(crate) current_effort_label: &'static str,
}

pub(crate) fn build_model_selection_overlay_view(
    input: ModelSelectionFrameInput<'_>,
) -> ModelSelectionOverlayView {
    let staged_model = MODEL_SELECTION_MODEL_OPTIONS[input.staged_model_index];
    let active_model = match input.step {
        ModelSelectionStep::Model => MODEL_SELECTION_MODEL_OPTIONS[input.selected_model_index],
        ModelSelectionStep::Effort => staged_model,
    };
    let selection_lines = match input.step {
        ModelSelectionStep::Model => build_model_selection_lines(input.selected_model_index),
        ModelSelectionStep::Effort => {
            build_effort_selection_lines(staged_model, input.selected_effort_index)
        }
    };

    ModelSelectionOverlayView {
        header_lines: vec![
            AkraTheme::title_line("Model setup", ""),
            build_model_selection_step_line(input.step),
            build_provider_line(active_model.provider_label, input.step, staged_model.label),
        ],
        selection_title: Line::from(match input.step {
            ModelSelectionStep::Model => "Choose model",
            ModelSelectionStep::Effort => "Choose reasoning",
        }),
        selection_lines,
        summary_lines: build_model_selection_summary_lines(input, staged_model.label),
        key_lines: build_model_selection_key_lines(input.step),
    }
}

fn build_model_selection_lines(selected_model_index: usize) -> Vec<Line<'static>> {
    MODEL_SELECTION_MODEL_OPTIONS
        .iter()
        .enumerate()
        .map(|(index, option)| {
            overlay_option_line(
                &(index + 1).to_string(),
                option.label,
                option.detail,
                selected_model_index == index,
                false,
            )
        })
        .collect()
}

fn build_effort_selection_lines(
    model: super::super::super::ModelSelectionModelOption,
    selected_effort_index: usize,
) -> Vec<Line<'static>> {
    model
        .supported_efforts
        .iter()
        .enumerate()
        .map(|(index, effort)| {
            let option = model_selection_effort_option(*effort);
            let detail = if option.effort == model.recommended_effort {
                "recommended"
            } else {
                ""
            };
            overlay_option_line(
                &(index + 1).to_string(),
                option.label,
                detail,
                selected_effort_index == index,
                false,
            )
        })
        .collect()
}

fn build_model_selection_step_line(step: ModelSelectionStep) -> Line<'static> {
    match step {
        ModelSelectionStep::Model => Line::from(vec![
            Span::styled("1  Model", AkraTheme::brand()),
            Span::styled("   →   ", AkraTheme::subtle()),
            Span::styled("2  Reasoning", AkraTheme::subtle()),
        ]),
        ModelSelectionStep::Effort => Line::from(vec![
            Span::styled("1  Model", AkraTheme::subtle()),
            Span::styled("   →   ", AkraTheme::subtle()),
            Span::styled("2  Reasoning", AkraTheme::brand()),
        ]),
    }
}

fn build_provider_line(
    provider_label: &'static str,
    step: ModelSelectionStep,
    staged_model_label: &'static str,
) -> Line<'static> {
    match step {
        ModelSelectionStep::Model => Line::from(vec![
            Span::styled("Provider", AkraTheme::subtle()),
            Span::raw(format!(": {provider_label}")),
        ]),
        ModelSelectionStep::Effort => Line::from(vec![
            Span::styled("Provider", AkraTheme::subtle()),
            Span::raw(format!(": {provider_label}  ·  ")),
            Span::styled(staged_model_label, AkraTheme::brand()),
        ]),
    }
}

fn build_model_selection_summary_lines(
    input: ModelSelectionFrameInput<'_>,
    staged_model_label: &'static str,
) -> Vec<Line<'static>> {
    match input.step {
        ModelSelectionStep::Model => vec![Line::styled(
            format!(
                "Current  {} · {}",
                display_model_label(input.current_model_label),
                input.current_effort_label
            ),
            AkraTheme::muted(),
        )],
        ModelSelectionStep::Effort => {
            let selected_effort = model_selection_effort_option(
                MODEL_SELECTION_MODEL_OPTIONS[input.staged_model_index].supported_efforts
                    [input.selected_effort_index],
            );
            vec![Line::styled(
                format!(
                    "Selection  {staged_model_label} · {}",
                    selected_effort.label
                ),
                AkraTheme::muted(),
            )]
        }
    }
}

fn display_model_label(model: &str) -> &str {
    if model == "default" {
        return MODEL_SELECTION_MODEL_OPTIONS
            .iter()
            .find(|option| option.model.is_none())
            .map(|option| option.label)
            .unwrap_or(model);
    }
    MODEL_SELECTION_MODEL_OPTIONS
        .iter()
        .find(|option| option.model == Some(model))
        .map(|option| option.label)
        .unwrap_or(model)
}

fn build_model_selection_key_lines(step: ModelSelectionStep) -> Vec<Line<'static>> {
    match step {
        ModelSelectionStep::Model => vec![AkraTheme::key_line(
            "Enter choose  ·  ↑↓ / j k move  ·  Esc close",
        )],
        ModelSelectionStep::Effort => vec![AkraTheme::key_line(
            "Enter apply  ·  ← / Backspace model  ·  ↑↓ / j k move  ·  Esc close",
        )],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn model_step_is_compact_and_identifies_the_provider() {
        let view = build_model_selection_overlay_view(ModelSelectionFrameInput {
            step: ModelSelectionStep::Model,
            selected_model_index: 0,
            selected_effort_index: 2,
            staged_model_index: 0,
            current_model_label: "gpt-5.5",
            current_effort_label: "high",
        });

        assert!(
            view.header_lines
                .iter()
                .any(|line| line.to_string().contains("Provider: OpenAI"))
        );
        assert!(
            view.selection_lines
                .iter()
                .any(|line| line.to_string().contains("GPT-5.6 Sol"))
        );
        assert!(
            !view
                .selection_lines
                .iter()
                .any(|line| line.to_string().contains("Frontier model for complex"))
        );
    }

    #[test]
    fn gpt_5_6_reasoning_step_shows_the_recommendation_and_max_only() {
        let view = build_model_selection_overlay_view(ModelSelectionFrameInput {
            step: ModelSelectionStep::Effort,
            selected_model_index: 0,
            selected_effort_index: 2,
            staged_model_index: 0,
            current_model_label: "gpt-5.5",
            current_effort_label: "high",
        });

        assert_eq!(view.selection_title.to_string(), "Choose reasoning");
        assert!(
            view.selection_lines
                .iter()
                .any(|line| line.to_string().contains("max"))
        );
        assert!(
            view.selection_lines
                .iter()
                .any(|line| line.to_string().contains("recommended"))
        );
        assert!(
            !view
                .selection_lines
                .iter()
                .any(|line| line.to_string().contains("minimal"))
        );
    }
}
