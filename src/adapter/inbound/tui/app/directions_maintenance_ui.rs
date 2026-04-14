use crate::application::service::planning::{
    DirectionsMaintenanceDirectionSummary, DirectionsMaintenanceSummary,
    DirectionsSupportingFileStatus,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(super) enum DirectionsMaintenanceOverlayStep {
    #[default]
    Overview,
    DetailDocSelection,
    DetailDocConfirm,
    ManualEditor,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(super) enum DetailDocConfirmChoice {
    #[default]
    Yes,
    No,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct PendingDetailDocCreation {
    direction_id: String,
    direction_title: String,
}

impl PendingDetailDocCreation {
    pub fn direction_id(&self) -> &str {
        self.direction_id.as_str()
    }

    pub fn direction_title(&self) -> &str {
        self.direction_title.as_str()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(super) struct DirectionsMaintenanceOverlayUiState {
    step: DirectionsMaintenanceOverlayStep,
    summary: Option<DirectionsMaintenanceSummary>,
    selected_missing_detail_doc_index: usize,
    pending_detail_doc_creation: Option<PendingDetailDocCreation>,
    detail_doc_confirm_choice: DetailDocConfirmChoice,
}

impl DirectionsMaintenanceOverlayUiState {
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    pub fn open_summary(&mut self, summary: DirectionsMaintenanceSummary) {
        self.summary = Some(summary);
        self.step = DirectionsMaintenanceOverlayStep::Overview;
        self.selected_missing_detail_doc_index = 0;
        self.pending_detail_doc_creation = None;
        self.detail_doc_confirm_choice = DetailDocConfirmChoice::Yes;
    }

    pub fn step(&self) -> DirectionsMaintenanceOverlayStep {
        self.step
    }

    pub fn summary(&self) -> Option<&DirectionsMaintenanceSummary> {
        self.summary.as_ref()
    }

    pub fn actionable_detail_doc_directions(&self) -> Vec<&DirectionsMaintenanceDirectionSummary> {
        self.summary
            .as_ref()
            .map(|summary| {
                summary
                    .directions
                    .iter()
                    .filter(|direction| {
                        direction.detail_doc_status != DirectionsSupportingFileStatus::Ready
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    pub fn selected_actionable_detail_doc_direction(
        &self,
    ) -> Option<&DirectionsMaintenanceDirectionSummary> {
        let directions = self.actionable_detail_doc_directions();
        directions
            .get(
                self.selected_missing_detail_doc_index
                    .min(directions.len().saturating_sub(1)),
            )
            .copied()
    }

    pub fn open_detail_doc_selection(&mut self) {
        self.step = DirectionsMaintenanceOverlayStep::DetailDocSelection;
        self.selected_missing_detail_doc_index = 0;
        self.pending_detail_doc_creation = None;
        self.detail_doc_confirm_choice = DetailDocConfirmChoice::Yes;
    }

    pub fn return_to_overview(&mut self) {
        self.step = DirectionsMaintenanceOverlayStep::Overview;
        self.pending_detail_doc_creation = None;
        self.detail_doc_confirm_choice = DetailDocConfirmChoice::Yes;
    }

    pub fn move_missing_detail_doc_selection(&mut self, delta: isize) {
        let directions = self.actionable_detail_doc_directions();
        if directions.is_empty() {
            self.selected_missing_detail_doc_index = 0;
            return;
        }
        let max_index = directions.len().saturating_sub(1) as isize;
        let next_index =
            (self.selected_missing_detail_doc_index as isize + delta).clamp(0, max_index);
        self.selected_missing_detail_doc_index = next_index as usize;
    }

    pub fn open_detail_doc_confirm(&mut self) {
        let Some(direction) = self.selected_actionable_detail_doc_direction() else {
            return;
        };
        self.pending_detail_doc_creation = Some(PendingDetailDocCreation {
            direction_id: direction.id.clone(),
            direction_title: direction.title.clone(),
        });
        self.detail_doc_confirm_choice = DetailDocConfirmChoice::Yes;
        self.step = DirectionsMaintenanceOverlayStep::DetailDocConfirm;
    }

    pub fn pending_detail_doc_creation(&self) -> Option<&PendingDetailDocCreation> {
        self.pending_detail_doc_creation.as_ref()
    }

    pub fn detail_doc_confirm_choice(&self) -> DetailDocConfirmChoice {
        self.detail_doc_confirm_choice
    }

    pub fn move_detail_doc_confirm_choice(&mut self, delta: isize) {
        self.detail_doc_confirm_choice = match (self.detail_doc_confirm_choice, delta.is_negative())
        {
            (DetailDocConfirmChoice::Yes, false) => DetailDocConfirmChoice::No,
            (DetailDocConfirmChoice::No, true) => DetailDocConfirmChoice::Yes,
            (choice, _) => choice,
        };
    }

    pub fn open_manual_editor(&mut self) {
        self.step = DirectionsMaintenanceOverlayStep::ManualEditor;
    }
}
