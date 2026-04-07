use ratatui::widgets::ListState;

#[derive(Debug, Default)]
pub(super) struct SessionOverlayUiState {
    pub list_state: ListState,
}

impl SessionOverlayUiState {
    pub fn sync_selected_session(&mut self, selected_session_index: Option<usize>) {
        self.list_state.select(selected_session_index);
    }

    pub fn reset(&mut self) {
        self.list_state = ListState::default();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sync_selected_session_preserves_existing_offset() {
        let mut state = SessionOverlayUiState {
            list_state: ListState::default().with_offset(4).with_selected(Some(5)),
        };

        state.sync_selected_session(Some(2));

        assert_eq!(state.list_state.selected(), Some(2));
        assert_eq!(state.list_state.offset(), 4);
    }

    #[test]
    fn sync_selected_session_with_none_clears_offset() {
        let mut state = SessionOverlayUiState {
            list_state: ListState::default().with_offset(4).with_selected(Some(5)),
        };

        state.sync_selected_session(None);

        assert_eq!(state.list_state.selected(), None);
        assert_eq!(state.list_state.offset(), 0);
    }

    #[test]
    fn reset_clears_selection_and_offset() {
        let mut state = SessionOverlayUiState {
            list_state: ListState::default().with_offset(4).with_selected(Some(5)),
        };

        state.reset();

        assert_eq!(state.list_state.selected(), None);
        assert_eq!(state.list_state.offset(), 0);
    }
}
