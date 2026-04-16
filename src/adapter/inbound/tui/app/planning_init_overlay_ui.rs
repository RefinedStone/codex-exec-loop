use crate::application::service::planning::PlanningInitStageResult;
use crate::domain::planning::PlanningValidationReport;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum PlanningInitOverlayStep {
    ModeSelection,
    ExistingWorkspace,
    DetailSelection,
    SimpleReview,
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct PlanningInitSimpleReviewState {
    draft_name: String,
    draft_directory: String,
    staged_file_count: usize,
    validation_report: PlanningValidationReport,
}

impl PlanningInitSimpleReviewState {
    pub fn draft_name(&self) -> &str {
        self.draft_name.as_str()
    }

    pub fn draft_directory(&self) -> &str {
        self.draft_directory.as_str()
    }

    pub fn staged_file_count(&self) -> usize {
        self.staged_file_count
    }

    pub fn validation_report(&self) -> &PlanningValidationReport {
        &self.validation_report
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct PlanningInitOverlayUiState {
    step: PlanningInitOverlayStep,
    mode_selection: PlanningInitModeSelection,
    detail_selection: PlanningInitDetailSelection,
    simple_review: Option<PlanningInitSimpleReviewState>,
}

impl Default for PlanningInitOverlayUiState {
    fn default() -> Self {
        Self {
            step: PlanningInitOverlayStep::ModeSelection,
            mode_selection: PlanningInitModeSelection::Simple,
            detail_selection: PlanningInitDetailSelection::Manual,
            simple_review: None,
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

    pub fn simple_review(&self) -> Option<&PlanningInitSimpleReviewState> {
        self.simple_review.as_ref()
    }

    pub fn open_command_center_mode_selection(&mut self) {
        self.step = PlanningInitOverlayStep::ModeSelection;
        self.simple_review = None;
    }

    pub fn apply_simple_review_validation(&mut self, validation_report: PlanningValidationReport) {
        if let Some(review) = self.simple_review.as_mut() {
            review.validation_report = validation_report;
        }
    }

    pub fn select_mode(&mut self, selection: PlanningInitModeSelection) {
        self.mode_selection = selection;
        if selection == PlanningInitModeSelection::Simple {
            self.step = PlanningInitOverlayStep::ModeSelection;
        }
        self.simple_review = None;
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
        self.simple_review = None;
    }

    pub fn open_existing_workspace(&mut self) {
        self.step = PlanningInitOverlayStep::ExistingWorkspace;
        self.simple_review = None;
    }

    pub fn open_manual_editor(&mut self) {
        self.mode_selection = PlanningInitModeSelection::Detail;
        self.detail_selection = PlanningInitDetailSelection::Manual;
        self.step = PlanningInitOverlayStep::ManualEditor;
        self.simple_review = None;
    }

    pub fn open_simple_editor(&mut self) {
        self.mode_selection = PlanningInitModeSelection::Simple;
        self.detail_selection = PlanningInitDetailSelection::Manual;
        self.step = PlanningInitOverlayStep::ManualEditor;
    }

    pub fn open_simple_review(&mut self, staged: PlanningInitStageResult) {
        self.mode_selection = PlanningInitModeSelection::Simple;
        self.step = PlanningInitOverlayStep::SimpleReview;
        self.simple_review = Some(PlanningInitSimpleReviewState {
            draft_name: staged.draft_name,
            draft_directory: staged.draft_directory,
            staged_file_count: staged.staged_file_count,
            validation_report: staged.validation_report,
        });
    }

    pub fn return_to_mode_selection(&mut self) {
        self.step = PlanningInitOverlayStep::ModeSelection;
        self.simple_review = None;
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
    use crate::application::service::planning::PlanningBootstrapMode;
    use crate::application::service::planning::PlanningInitStageResult;
    use crate::domain::planning::PlanningValidationReport;

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
    fn opening_existing_workspace_switches_overlay_step() {
        let mut state = PlanningInitOverlayUiState::default();

        state.open_existing_workspace();

        assert_eq!(state.step(), PlanningInitOverlayStep::ExistingWorkspace);
        assert!(state.simple_review().is_none());
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
    fn opening_simple_review_tracks_staged_draft_metadata() {
        let mut state = PlanningInitOverlayUiState::default();

        state.open_simple_review(PlanningInitStageResult {
            mode: PlanningBootstrapMode::Simple,
            draft_name: "bootstrap-1".to_string(),
            draft_directory: "/tmp/bootstrap-1".to_string(),
            staged_files: Vec::new(),
            staged_file_count: 4,
            validation_report: PlanningValidationReport::default(),
        });

        assert_eq!(state.step(), PlanningInitOverlayStep::SimpleReview);
        assert_eq!(state.selected_mode(), PlanningInitModeSelection::Simple);
        assert_eq!(
            state.simple_review().map(|review| review.draft_name()),
            Some("bootstrap-1")
        );
        assert_eq!(
            state
                .simple_review()
                .map(|review| review.staged_file_count()),
            Some(4)
        );
    }

    #[test]
    fn reset_restores_default_selections() {
        let mut state = PlanningInitOverlayUiState::default();
        state.open_simple_review(PlanningInitStageResult {
            mode: PlanningBootstrapMode::Simple,
            draft_name: "bootstrap-1".to_string(),
            draft_directory: "/tmp/bootstrap-1".to_string(),
            staged_files: Vec::new(),
            staged_file_count: 4,
            validation_report: PlanningValidationReport::default(),
        });

        state.reset();

        assert_eq!(state.step(), PlanningInitOverlayStep::ModeSelection);
        assert_eq!(state.selected_mode(), PlanningInitModeSelection::Simple);
        assert_eq!(state.selected_detail(), PlanningInitDetailSelection::Manual);
        assert!(state.simple_review().is_none());
    }
}
