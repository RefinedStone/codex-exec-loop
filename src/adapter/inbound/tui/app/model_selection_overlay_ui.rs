use crate::domain::conversation::{ConversationReasoningEffort, ConversationTurnOptions};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ModelSelectionStep {
    Model,
    Effort,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct ModelSelectionModelOption {
    pub(super) label: &'static str,
    pub(super) model: Option<&'static str>,
    pub(super) provider_label: &'static str,
    pub(super) detail: &'static str,
    pub(super) supported_efforts: &'static [Option<ConversationReasoningEffort>],
    pub(super) recommended_effort: Option<ConversationReasoningEffort>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct ModelSelectionEffortOption {
    pub(super) label: &'static str,
    pub(super) effort: Option<ConversationReasoningEffort>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ModelSelectionOverlayUiState {
    step: ModelSelectionStep,
    selected_model_index: usize,
    // This index is always relative to the staged model's supported effort list.
    selected_effort_index: usize,
    staged_model_index: usize,
}

const OPENAI_PROVIDER_LABEL: &str = "OpenAI";

// GPT-5.6 no longer exposes `minimal`; `max` is available instead. Keeping the
// capability on each catalog entry makes a future provider catalog a data-source
// replacement rather than another picker flow.
const GPT_5_6_REASONING_EFFORTS: &[Option<ConversationReasoningEffort>] = &[
    Some(ConversationReasoningEffort::Low),
    Some(ConversationReasoningEffort::Medium),
    Some(ConversationReasoningEffort::High),
    Some(ConversationReasoningEffort::XHigh),
    Some(ConversationReasoningEffort::Max),
    Some(ConversationReasoningEffort::None),
];

const LEGACY_REASONING_EFFORTS: &[Option<ConversationReasoningEffort>] = &[
    Some(ConversationReasoningEffort::Low),
    Some(ConversationReasoningEffort::Medium),
    Some(ConversationReasoningEffort::High),
    Some(ConversationReasoningEffort::XHigh),
    Some(ConversationReasoningEffort::Minimal),
    Some(ConversationReasoningEffort::None),
];

const APP_SERVER_DEFAULT_REASONING_EFFORTS: &[Option<ConversationReasoningEffort>] = &[
    Some(ConversationReasoningEffort::Low),
    Some(ConversationReasoningEffort::Medium),
    Some(ConversationReasoningEffort::High),
    Some(ConversationReasoningEffort::XHigh),
    Some(ConversationReasoningEffort::Minimal),
    Some(ConversationReasoningEffort::None),
    None,
];

pub(super) const MODEL_SELECTION_MODEL_OPTIONS: &[ModelSelectionModelOption] = &[
    ModelSelectionModelOption {
        label: "GPT-5.6 Sol",
        model: Some("gpt-5.6-sol"),
        provider_label: OPENAI_PROVIDER_LABEL,
        detail: "frontier",
        supported_efforts: GPT_5_6_REASONING_EFFORTS,
        recommended_effort: Some(ConversationReasoningEffort::Medium),
    },
    ModelSelectionModelOption {
        label: "GPT-5.6 Terra",
        model: Some("gpt-5.6-terra"),
        provider_label: OPENAI_PROVIDER_LABEL,
        detail: "balanced",
        supported_efforts: GPT_5_6_REASONING_EFFORTS,
        recommended_effort: Some(ConversationReasoningEffort::Max),
    },
    ModelSelectionModelOption {
        label: "GPT-5.6 Luna",
        model: Some("gpt-5.6-luna"),
        provider_label: OPENAI_PROVIDER_LABEL,
        detail: "fast",
        supported_efforts: GPT_5_6_REASONING_EFFORTS,
        recommended_effort: Some(ConversationReasoningEffort::Max),
    },
    ModelSelectionModelOption {
        label: "GPT-5.5",
        model: Some("gpt-5.5"),
        provider_label: OPENAI_PROVIDER_LABEL,
        detail: "",
        supported_efforts: LEGACY_REASONING_EFFORTS,
        recommended_effort: Some(ConversationReasoningEffort::High),
    },
    ModelSelectionModelOption {
        label: "GPT-5.4",
        model: Some("gpt-5.4"),
        provider_label: OPENAI_PROVIDER_LABEL,
        detail: "",
        supported_efforts: LEGACY_REASONING_EFFORTS,
        recommended_effort: Some(ConversationReasoningEffort::High),
    },
    ModelSelectionModelOption {
        label: "GPT-5.4 mini",
        model: Some("gpt-5.4-mini"),
        provider_label: OPENAI_PROVIDER_LABEL,
        detail: "",
        supported_efforts: LEGACY_REASONING_EFFORTS,
        recommended_effort: Some(ConversationReasoningEffort::Medium),
    },
    ModelSelectionModelOption {
        label: "GPT-5.3 Codex",
        model: Some("gpt-5.3-codex"),
        provider_label: OPENAI_PROVIDER_LABEL,
        detail: "",
        supported_efforts: LEGACY_REASONING_EFFORTS,
        recommended_effort: Some(ConversationReasoningEffort::High),
    },
    ModelSelectionModelOption {
        label: "GPT-5.3 Codex Spark",
        model: Some("gpt-5.3-codex-spark"),
        provider_label: OPENAI_PROVIDER_LABEL,
        detail: "",
        supported_efforts: LEGACY_REASONING_EFFORTS,
        recommended_effort: Some(ConversationReasoningEffort::Low),
    },
    ModelSelectionModelOption {
        label: "GPT-5.2",
        model: Some("gpt-5.2"),
        provider_label: OPENAI_PROVIDER_LABEL,
        detail: "",
        supported_efforts: LEGACY_REASONING_EFFORTS,
        recommended_effort: Some(ConversationReasoningEffort::High),
    },
    ModelSelectionModelOption {
        label: "App-server default",
        model: None,
        provider_label: OPENAI_PROVIDER_LABEL,
        detail: "",
        supported_efforts: APP_SERVER_DEFAULT_REASONING_EFFORTS,
        recommended_effort: None,
    },
];

pub(super) fn model_selection_effort_option(
    effort: Option<ConversationReasoningEffort>,
) -> ModelSelectionEffortOption {
    match effort {
        Some(effort) => ModelSelectionEffortOption {
            label: effort.label(),
            effort: Some(effort),
        },
        None => ModelSelectionEffortOption {
            label: "default",
            effort: None,
        },
    }
}

impl Default for ModelSelectionOverlayUiState {
    fn default() -> Self {
        let staged_model_index = project_default_model_index();
        let staged_model = MODEL_SELECTION_MODEL_OPTIONS[staged_model_index];
        Self {
            step: ModelSelectionStep::Model,
            selected_model_index: staged_model_index,
            selected_effort_index: effort_option_index(
                staged_model,
                Some(ConversationTurnOptions::DEFAULT_REASONING_EFFORT),
            )
            .unwrap_or_else(|| recommended_effort_index(staged_model)),
            staged_model_index,
        }
    }
}

impl ModelSelectionOverlayUiState {
    pub(super) fn reset_from_turn_options(&mut self, turn_options: &ConversationTurnOptions) {
        self.step = ModelSelectionStep::Model;
        self.selected_model_index = turn_options
            .model
            .as_deref()
            .and_then(model_option_index)
            .unwrap_or_else(default_model_index);
        self.staged_model_index = self.selected_model_index;
        let staged_model = self.staged_model();
        self.selected_effort_index =
            effort_option_index(staged_model, turn_options.reasoning_effort)
                .unwrap_or_else(|| recommended_effort_index(staged_model));
    }

    pub(super) fn step(&self) -> ModelSelectionStep {
        self.step
    }

    pub(super) fn selected_model_index(&self) -> usize {
        self.selected_model_index
    }

    pub(super) fn selected_effort_index(&self) -> usize {
        self.selected_effort_index
    }

    pub(super) fn staged_model_index(&self) -> usize {
        self.staged_model_index
    }

    pub(super) fn staged_model(&self) -> ModelSelectionModelOption {
        MODEL_SELECTION_MODEL_OPTIONS[self.staged_model_index]
    }

    pub(super) fn selected_effort(&self) -> ModelSelectionEffortOption {
        model_selection_effort_option(
            self.staged_model().supported_efforts[self.selected_effort_index],
        )
    }

    pub(super) fn move_selection(&mut self, delta: isize) {
        let len = self.active_option_len();
        if len == 0 {
            return;
        }
        let current = self.active_selected_index() as isize;
        let next = (current + delta).rem_euclid(len as isize) as usize;
        self.set_active_selected_index(next);
    }

    pub(super) fn select_active_index(&mut self, index: usize) -> bool {
        if index >= self.active_option_len() {
            return false;
        }
        self.set_active_selected_index(index);
        true
    }

    pub(super) fn advance_from_model_selection(&mut self) {
        let selected_effort = self.selected_effort().effort;
        let model_changed = self.selected_model_index != self.staged_model_index;
        self.staged_model_index = self.selected_model_index;
        let staged_model = self.staged_model();
        self.selected_effort_index = if model_changed {
            recommended_effort_index(staged_model)
        } else {
            effort_option_index(staged_model, selected_effort)
                .unwrap_or_else(|| recommended_effort_index(staged_model))
        };
        self.step = ModelSelectionStep::Effort;
    }

    pub(super) fn return_to_model_selection(&mut self) {
        self.selected_model_index = self.staged_model_index;
        self.step = ModelSelectionStep::Model;
    }

    fn active_option_len(&self) -> usize {
        match self.step {
            ModelSelectionStep::Model => MODEL_SELECTION_MODEL_OPTIONS.len(),
            ModelSelectionStep::Effort => self.staged_model().supported_efforts.len(),
        }
    }

    fn active_selected_index(&self) -> usize {
        match self.step {
            ModelSelectionStep::Model => self.selected_model_index,
            ModelSelectionStep::Effort => self.selected_effort_index,
        }
    }

    fn set_active_selected_index(&mut self, index: usize) {
        match self.step {
            ModelSelectionStep::Model => self.selected_model_index = index,
            ModelSelectionStep::Effort => self.selected_effort_index = index,
        }
    }
}

fn model_option_index(model: &str) -> Option<usize> {
    MODEL_SELECTION_MODEL_OPTIONS
        .iter()
        .position(|option| option.model == Some(model))
}

fn effort_option_index(
    model: ModelSelectionModelOption,
    effort: Option<ConversationReasoningEffort>,
) -> Option<usize> {
    model
        .supported_efforts
        .iter()
        .position(|candidate| *candidate == effort)
}

fn default_model_index() -> usize {
    MODEL_SELECTION_MODEL_OPTIONS
        .iter()
        .position(|option| option.model.is_none())
        .unwrap_or(0)
}

fn project_default_model_index() -> usize {
    model_option_index(ConversationTurnOptions::DEFAULT_MODEL).unwrap_or(0)
}

fn recommended_effort_index(model: ModelSelectionModelOption) -> usize {
    effort_option_index(model, model.recommended_effort).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reset_selects_current_turn_options_when_available() {
        let mut state = ModelSelectionOverlayUiState::default();
        state.reset_from_turn_options(&ConversationTurnOptions {
            model: Some("gpt-5.4-mini".to_string()),
            reasoning_effort: Some(ConversationReasoningEffort::High),
        });

        assert_eq!(state.step(), ModelSelectionStep::Model);
        assert_eq!(
            MODEL_SELECTION_MODEL_OPTIONS[state.selected_model_index()].model,
            Some("gpt-5.4-mini")
        );
        assert_eq!(
            state.selected_effort().effort,
            Some(ConversationReasoningEffort::High)
        );
    }

    #[test]
    fn reset_uses_a_models_recommendation_when_current_effort_is_unsupported() {
        let mut state = ModelSelectionOverlayUiState::default();
        state.reset_from_turn_options(&ConversationTurnOptions {
            model: Some("gpt-5.6-sol".to_string()),
            reasoning_effort: Some(ConversationReasoningEffort::Minimal),
        });

        assert_eq!(state.staged_model().model, Some("gpt-5.6-sol"));
        assert_eq!(
            state.selected_effort().effort,
            Some(ConversationReasoningEffort::Medium)
        );
    }

    #[test]
    fn gpt_5_6_efforts_include_max_without_minimal() {
        let sol = MODEL_SELECTION_MODEL_OPTIONS[model_option_index("gpt-5.6-sol").unwrap()];

        assert!(
            sol.supported_efforts
                .contains(&Some(ConversationReasoningEffort::Max))
        );
        assert!(
            !sol.supported_efforts
                .contains(&Some(ConversationReasoningEffort::Minimal))
        );
    }

    #[test]
    fn reset_selects_app_server_defaults_when_turn_options_are_unset() {
        let mut state = ModelSelectionOverlayUiState::default();
        state.reset_from_turn_options(&ConversationTurnOptions::app_server_default());

        assert_eq!(
            MODEL_SELECTION_MODEL_OPTIONS[state.selected_model_index()].model,
            None
        );
        assert_eq!(state.selected_effort().effort, None);
    }

    #[test]
    fn default_state_selects_project_defaults() {
        let state = ModelSelectionOverlayUiState::default();

        assert_eq!(
            MODEL_SELECTION_MODEL_OPTIONS[state.selected_model_index()].model,
            Some(ConversationTurnOptions::DEFAULT_MODEL)
        );
        assert_eq!(
            state.selected_effort().effort,
            Some(ConversationTurnOptions::DEFAULT_REASONING_EFFORT)
        );
    }

    #[test]
    fn model_selection_advances_to_effort_with_staged_model() {
        let mut state = ModelSelectionOverlayUiState::default();
        let sol_index = model_option_index("gpt-5.6-sol").unwrap();
        state.select_active_index(sol_index);

        state.advance_from_model_selection();

        assert_eq!(state.step(), ModelSelectionStep::Effort);
        assert_eq!(state.staged_model().model, Some("gpt-5.6-sol"));
        assert_eq!(
            state.selected_effort().effort,
            Some(ConversationReasoningEffort::Medium)
        );
    }

    #[test]
    fn changing_models_selects_their_configured_recommended_effort() {
        let mut state = ModelSelectionOverlayUiState::default();
        for (model, expected_effort) in [
            ("gpt-5.6-sol", ConversationReasoningEffort::Medium),
            ("gpt-5.6-terra", ConversationReasoningEffort::Max),
            ("gpt-5.6-luna", ConversationReasoningEffort::Max),
        ] {
            let model_index = model_option_index(model).unwrap();
            assert!(state.select_active_index(model_index));

            state.advance_from_model_selection();

            assert_eq!(state.staged_model().model, Some(model));
            assert_eq!(state.selected_effort().effort, Some(expected_effort));
            state.return_to_model_selection();
        }
    }

    #[test]
    fn selection_movement_wraps_within_active_step() {
        let mut state = ModelSelectionOverlayUiState::default();
        assert!(state.select_active_index(0));

        state.move_selection(-1);

        assert_eq!(
            state.selected_model_index(),
            MODEL_SELECTION_MODEL_OPTIONS.len() - 1
        );
    }
}
