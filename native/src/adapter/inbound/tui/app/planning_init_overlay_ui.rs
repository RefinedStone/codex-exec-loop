#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum PlanningInitOverlayStep {
    ModeSelection,
    DetailSelection,
    ManualEditor,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum PlanningInitModeSelection {
    Simple,
    Detail,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum PlanningInitDetailSelection {
    Manual,
    LlmAssisted,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct PlanningInitOverlayUiState {
    step: PlanningInitOverlayStep,
    mode_selection: PlanningInitModeSelection,
    detail_selection: PlanningInitDetailSelection,
}

impl Default for PlanningInitOverlayUiState {
    fn default() -> Self {
        Self {
            step: PlanningInitOverlayStep::ModeSelection,
            mode_selection: PlanningInitModeSelection::Simple,
            detail_selection: PlanningInitDetailSelection::Manual,
        }
    }
}

impl PlanningInitOverlayUiState {
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    pub fn step(&self) -> PlanningInitOverlayStep {
        self.step
    }

    pub fn selected_mode(&self) -> PlanningInitModeSelection {
        self.mode_selection
    }

    pub fn selected_detail(&self) -> PlanningInitDetailSelection {
        self.detail_selection
    }

    pub fn select_mode(&mut self, selection: PlanningInitModeSelection) {
        self.mode_selection = selection;
        if selection == PlanningInitModeSelection::Simple {
            self.step = PlanningInitOverlayStep::ModeSelection;
        }
    }

    pub fn move_mode_selection(&mut self, delta: isize) {
        self.mode_selection = match (self.mode_selection, delta.is_negative()) {
            (PlanningInitModeSelection::Simple, false) => PlanningInitModeSelection::Detail,
            (PlanningInitModeSelection::Detail, true) => PlanningInitModeSelection::Simple,
            (selection, _) => selection,
        };
    }

    pub fn open_detail_selection(&mut self) {
        self.mode_selection = PlanningInitModeSelection::Detail;
        self.step = PlanningInitOverlayStep::DetailSelection;
    }

    pub fn open_manual_editor(&mut self) {
        self.mode_selection = PlanningInitModeSelection::Detail;
        self.detail_selection = PlanningInitDetailSelection::Manual;
        self.step = PlanningInitOverlayStep::ManualEditor;
    }

    pub fn return_to_mode_selection(&mut self) {
        self.step = PlanningInitOverlayStep::ModeSelection;
    }

    pub fn select_detail(&mut self, selection: PlanningInitDetailSelection) {
        self.detail_selection = selection;
    }

    pub fn move_detail_selection(&mut self, delta: isize) {
        self.detail_selection = match (self.detail_selection, delta.is_negative()) {
            (PlanningInitDetailSelection::Manual, false) => {
                PlanningInitDetailSelection::LlmAssisted
            }
            (PlanningInitDetailSelection::LlmAssisted, true) => PlanningInitDetailSelection::Manual,
            (selection, _) => selection,
        };
    }
}

#[cfg(test)]
mod tests {
    use super::{
        PlanningInitDetailSelection, PlanningInitModeSelection, PlanningInitOverlayStep,
        PlanningInitOverlayUiState,
    };

    #[test]
    fn default_state_starts_on_simple_mode_selection() {
        let state = PlanningInitOverlayUiState::default();

        assert_eq!(state.step(), PlanningInitOverlayStep::ModeSelection);
        assert_eq!(state.selected_mode(), PlanningInitModeSelection::Simple);
        assert_eq!(state.selected_detail(), PlanningInitDetailSelection::Manual);
    }

    #[test]
    fn opening_detail_selection_keeps_mode_and_step_in_sync() {
        let mut state = PlanningInitOverlayUiState::default();

        state.open_detail_selection();

        assert_eq!(state.step(), PlanningInitOverlayStep::DetailSelection);
        assert_eq!(state.selected_mode(), PlanningInitModeSelection::Detail);
    }

    #[test]
    fn opening_manual_editor_pins_detail_manual_selection() {
        let mut state = PlanningInitOverlayUiState::default();

        state.open_manual_editor();

        assert_eq!(state.step(), PlanningInitOverlayStep::ManualEditor);
        assert_eq!(state.selected_mode(), PlanningInitModeSelection::Detail);
        assert_eq!(state.selected_detail(), PlanningInitDetailSelection::Manual);
    }

    #[test]
    fn reset_restores_default_selections() {
        let mut state = PlanningInitOverlayUiState::default();
        state.open_detail_selection();
        state.select_detail(PlanningInitDetailSelection::LlmAssisted);

        state.reset();

        assert_eq!(state.step(), PlanningInitOverlayStep::ModeSelection);
        assert_eq!(state.selected_mode(), PlanningInitModeSelection::Simple);
        assert_eq!(state.selected_detail(), PlanningInitDetailSelection::Manual);
    }
}
