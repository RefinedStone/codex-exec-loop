#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WorkCenterSection {
    Task,
    Agents,
    Terminal,
    Approval,
    Delivery,
}

impl WorkCenterSection {
    pub(crate) const ALL: [Self; 5] = [
        Self::Task,
        Self::Agents,
        Self::Terminal,
        Self::Approval,
        Self::Delivery,
    ];
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) struct WorkCenterOverlayUiState {
    selected_index: usize,
}

impl WorkCenterOverlayUiState {
    pub(crate) fn reset(&mut self) {
        self.selected_index = 0;
    }

    pub(crate) fn selected_section(self) -> WorkCenterSection {
        WorkCenterSection::ALL[self.selected_index.min(WorkCenterSection::ALL.len() - 1)]
    }

    pub(crate) fn move_selection(&mut self, delta: isize) {
        self.selected_index = self
            .selected_index
            .saturating_add_signed(delta)
            .min(WorkCenterSection::ALL.len() - 1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selection_is_bounded_and_resettable() {
        let mut state = WorkCenterOverlayUiState::default();
        state.move_selection(-1);
        assert_eq!(state.selected_section(), WorkCenterSection::Task);

        state.move_selection(99);
        assert_eq!(state.selected_section(), WorkCenterSection::Delivery);

        state.reset();
        assert_eq!(state.selected_section(), WorkCenterSection::Task);
    }
}
