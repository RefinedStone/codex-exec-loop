use super::super::super::{
    AkraTheme, Line, MODEL_SELECTION_MODEL_OPTIONS, ModelSelectionStep, configured_model_index,
    model_selection_effort_option, model_selection_label_at, model_selection_option_at,
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
    pub(crate) configured_model: Option<&'a str>,
    pub(crate) current_model_label: &'a str,
    pub(crate) current_effort_label: &'static str,
}

pub(crate) fn build_model_selection_overlay_view(
    input: ModelSelectionFrameInput<'_>,
) -> ModelSelectionOverlayView {
    let staged_model = model_selection_option_at(input.staged_model_index);
    let staged_model_label =
        model_selection_label_at(input.staged_model_index, input.configured_model);
    let active_model = match input.step {
        ModelSelectionStep::Model => model_selection_option_at(input.selected_model_index),
        ModelSelectionStep::Effort => staged_model,
    };
    let selection_lines = match input.step {
        ModelSelectionStep::Model => {
            build_model_selection_lines(input.selected_model_index, input.configured_model)
        }
        ModelSelectionStep::Effort => {
            build_effort_selection_lines(staged_model, input.selected_effort_index)
        }
    };

    ModelSelectionOverlayView {
        header_lines: vec![
            AkraTheme::title_line("Model setup", ""),
            build_model_selection_step_line(input.step),
            build_provider_line(active_model.provider_label, input.step, &staged_model_label),
        ],
        selection_title: Line::from(match input.step {
            ModelSelectionStep::Model => "Choose model",
            ModelSelectionStep::Effort => "Choose reasoning",
        }),
        selection_lines,
        summary_lines: build_model_selection_summary_lines(input, &staged_model_label),
        key_lines: build_model_selection_key_lines(input.step),
    }
}

fn build_model_selection_lines(
    selected_model_index: usize,
    configured_model: Option<&str>,
) -> Vec<Line<'static>> {
    let mut lines: Vec<_> = MODEL_SELECTION_MODEL_OPTIONS
        .iter()
        .enumerate()
        .map(|(index, option)| {
            overlay_option_line(
                model_selection_shortcut(index),
                option.label,
                option.detail,
                selected_model_index == index,
                false,
            )
        })
        .collect();
    if let Some(model) = configured_model {
        lines.push(overlay_option_line(
            "↑↓",
            &format!("Configured: {model}"),
            "custom",
            selected_model_index == configured_model_index(),
            false,
        ));
    }
    lines
}

fn model_selection_shortcut(index: usize) -> &'static str {
    const SHORTCUTS: [&str; 10] = ["1", "2", "3", "4", "5", "6", "7", "8", "9", "0"];
    SHORTCUTS.get(index).copied().unwrap_or("↑↓")
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
    staged_model_label: &str,
) -> Line<'static> {
    match step {
        ModelSelectionStep::Model => Line::from(vec![
            Span::styled("Provider", AkraTheme::subtle()),
            Span::raw(format!(": {provider_label}")),
        ]),
        ModelSelectionStep::Effort => Line::from(vec![
            Span::styled("Provider", AkraTheme::subtle()),
            Span::raw(format!(": {provider_label}  ·  ")),
            Span::styled(staged_model_label.to_string(), AkraTheme::brand()),
        ]),
    }
}

fn build_model_selection_summary_lines(
    input: ModelSelectionFrameInput<'_>,
    staged_model_label: &str,
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
                model_selection_option_at(input.staged_model_index).supported_efforts
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

fn display_model_label(model: &str) -> String {
    if model == "default" {
        return MODEL_SELECTION_MODEL_OPTIONS
            .iter()
            .find(|option| option.model.is_none())
            .map(|option| option.label.to_string())
            .unwrap_or_else(|| model.to_string());
    }
    MODEL_SELECTION_MODEL_OPTIONS
        .iter()
        .find(|option| option.model == Some(model))
        .map(|option| option.label.to_string())
        .unwrap_or_else(|| format!("Configured: {model}"))
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
            configured_model: None,
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
            configured_model: None,
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

    #[test]
    fn configured_model_is_rendered_as_a_temporary_picker_item() {
        let configured_index = configured_model_index();
        let view = build_model_selection_overlay_view(ModelSelectionFrameInput {
            step: ModelSelectionStep::Model,
            selected_model_index: configured_index,
            selected_effort_index: 0,
            staged_model_index: configured_index,
            configured_model: Some("my-private-model"),
            current_model_label: "my-private-model",
            current_effort_label: "max",
        });

        assert!(
            view.selection_lines
                .iter()
                .any(|line| { line.to_string().contains("Configured: my-private-model") })
        );
        assert!(
            view.summary_lines
                .iter()
                .any(|line| line.to_string().contains("Configured: my-private-model"))
        );
        assert!(view.selection_lines.iter().any(|line| {
            line.to_string()
                .contains("↑↓. Configured: my-private-model")
        }));
        assert!(
            !view
                .selection_lines
                .iter()
                .any(|line| line.to_string().contains("11. Configured"))
        );
    }

    #[test]
    fn tenth_model_uses_the_zero_shortcut_instead_of_an_unreachable_two_digit_label() {
        let view = build_model_selection_overlay_view(ModelSelectionFrameInput {
            step: ModelSelectionStep::Model,
            selected_model_index: 9,
            selected_effort_index: 0,
            staged_model_index: 9,
            configured_model: None,
            current_model_label: "gpt-5.5",
            current_effort_label: "high",
        });

        assert!(
            view.selection_lines
                .iter()
                .any(|line| line.to_string().contains("0. App-server default"))
        );
        assert!(
            !view
                .selection_lines
                .iter()
                .any(|line| line.to_string().contains("10. App-server default"))
        );
    }
}
