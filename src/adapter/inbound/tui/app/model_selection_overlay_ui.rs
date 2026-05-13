use crate::domain::conversation::{ConversationReasoningEffort, ConversationTurnOptions};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ModelSelectionStep {
    Model,
    Effort,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct ModelSelectionModelOption {
    pub(super) model: &'static str,
    pub(super) detail: &'static str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct ModelSelectionEffortOption {
    pub(super) effort: ConversationReasoningEffort,
    pub(super) detail: &'static str,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ModelSelectionOverlayUiState {
    step: ModelSelectionStep,
    selected_model_index: usize,
    selected_effort_index: usize,
    staged_model_index: usize,
}

pub(super) const MODEL_SELECTION_MODEL_OPTIONS: &[ModelSelectionModelOption] = &[
    ModelSelectionModelOption {
        model: "gpt-5.5",
        detail: "Frontier model for complex coding, research, and real-world work.",
    },
    ModelSelectionModelOption {
        model: "gpt-5.4",
        detail: "Strong model for everyday coding.",
    },
    ModelSelectionModelOption {
        model: "gpt-5.4-mini",
        detail: "Small, fast, and cost-efficient model for simpler coding tasks.",
    },
    ModelSelectionModelOption {
        model: "gpt-5.3-codex",
        detail: "Coding-optimized model.",
    },
    ModelSelectionModelOption {
        model: "gpt-5.3-codex-spark",
        detail: "Ultra-fast coding model.",
    },
    ModelSelectionModelOption {
        model: "gpt-5.2",
        detail: "Optimized for professional work and long-running agents.",
    },
];

pub(super) const MODEL_SELECTION_EFFORT_OPTIONS: &[ModelSelectionEffortOption] = &[
    ModelSelectionEffortOption {
        effort: ConversationReasoningEffort::Low,
        detail: "Fast responses with lighter reasoning.",
    },
    ModelSelectionEffortOption {
        effort: ConversationReasoningEffort::Medium,
        detail: "Balances speed and reasoning depth.",
    },
    ModelSelectionEffortOption {
        effort: ConversationReasoningEffort::High,
        detail: "Greater reasoning depth for complex problems.",
    },
    ModelSelectionEffortOption {
        effort: ConversationReasoningEffort::XHigh,
        detail: "Extra high reasoning for complex problems.",
    },
    ModelSelectionEffortOption {
        effort: ConversationReasoningEffort::Minimal,
        detail: "Minimal reasoning for very direct work.",
    },
    ModelSelectionEffortOption {
        effort: ConversationReasoningEffort::None,
        detail: "No explicit reasoning override.",
    },
];

impl Default for ModelSelectionOverlayUiState {
    fn default() -> Self {
        Self {
            step: ModelSelectionStep::Model,
            selected_model_index: 0,
            selected_effort_index: default_effort_index(),
            staged_model_index: 0,
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
            .unwrap_or(0);
        self.staged_model_index = self.selected_model_index;
        self.selected_effort_index = turn_options
            .reasoning_effort
            .and_then(effort_option_index)
            .unwrap_or_else(default_effort_index);
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
        MODEL_SELECTION_EFFORT_OPTIONS[self.selected_effort_index]
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
        self.staged_model_index = self.selected_model_index;
        self.step = ModelSelectionStep::Effort;
    }

    pub(super) fn return_to_model_selection(&mut self) {
        self.selected_model_index = self.staged_model_index;
        self.step = ModelSelectionStep::Model;
    }

    fn active_option_len(&self) -> usize {
        match self.step {
            ModelSelectionStep::Model => MODEL_SELECTION_MODEL_OPTIONS.len(),
            ModelSelectionStep::Effort => MODEL_SELECTION_EFFORT_OPTIONS.len(),
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
        .position(|option| option.model == model)
}

fn effort_option_index(effort: ConversationReasoningEffort) -> Option<usize> {
    MODEL_SELECTION_EFFORT_OPTIONS
        .iter()
        .position(|option| option.effort == effort)
}

fn default_effort_index() -> usize {
    effort_option_index(ConversationReasoningEffort::Medium).unwrap_or(0)
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
            "gpt-5.4-mini"
        );
        assert_eq!(
            state.selected_effort().effort,
            ConversationReasoningEffort::High
        );
    }

    #[test]
    fn model_selection_advances_to_effort_with_staged_model() {
        let mut state = ModelSelectionOverlayUiState::default();
        state.select_active_index(2);

        state.advance_from_model_selection();

        assert_eq!(state.step(), ModelSelectionStep::Effort);
        assert_eq!(state.staged_model().model, "gpt-5.4-mini");
    }

    #[test]
    fn selection_movement_wraps_within_active_step() {
        let mut state = ModelSelectionOverlayUiState::default();

        state.move_selection(-1);

        assert_eq!(
            state.selected_model_index(),
            MODEL_SELECTION_MODEL_OPTIONS.len() - 1
        );
    }
}
