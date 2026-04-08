use ratatui::widgets::ListState;

use crate::application::service::session_service::{SessionBrowserState, SessionProjectFilter};

#[derive(Debug)]
pub(super) struct SessionOverlayUiState {
    pub list_state: ListState,
    browser_state: SessionBrowserState,
}

impl Default for SessionOverlayUiState {
    fn default() -> Self {
        Self::new(10)
    }
}

impl SessionOverlayUiState {
    pub fn new(page_size: usize) -> Self {
        Self {
            list_state: ListState::default(),
            browser_state: SessionBrowserState::new(page_size),
        }
    }

    pub fn browser_state(&self) -> &SessionBrowserState {
        &self.browser_state
    }

    #[allow(dead_code)]
    pub fn set_search_query(&mut self, search_query: impl Into<String>) {
        self.browser_state.set_search_query(search_query);
        self.list_state = ListState::default();
    }

    #[allow(dead_code)]
    pub fn set_project_filter(&mut self, project_filter: SessionProjectFilter) {
        self.browser_state.set_project_filter(project_filter);
        self.list_state = ListState::default();
    }

    #[allow(dead_code)]
    pub fn move_page(&mut self, delta: isize, total_pages: usize) {
        self.browser_state.move_page(delta, total_pages);
        self.list_state = ListState::default();
    }

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
        let mut state = SessionOverlayUiState::new(10);
        state.list_state = ListState::default().with_offset(4).with_selected(Some(5));

        state.sync_selected_session(Some(2));

        assert_eq!(state.list_state.selected(), Some(2));
        assert_eq!(state.list_state.offset(), 4);
    }

    #[test]
    fn sync_selected_session_with_none_clears_offset() {
        let mut state = SessionOverlayUiState::new(10);
        state.list_state = ListState::default().with_offset(4).with_selected(Some(5));

        state.sync_selected_session(None);

        assert_eq!(state.list_state.selected(), None);
        assert_eq!(state.list_state.offset(), 0);
    }

    #[test]
    fn reset_clears_selection_and_offset_but_keeps_browser_query() {
        let mut state = SessionOverlayUiState::new(10);
        state.list_state = ListState::default().with_offset(4).with_selected(Some(5));
        state.set_search_query("bugfix");

        state.reset();

        assert_eq!(state.list_state.selected(), None);
        assert_eq!(state.list_state.offset(), 0);
        assert_eq!(state.browser_state().search_query, "bugfix");
    }

    #[test]
    fn search_query_resets_list_state_and_page_index() {
        let mut state = SessionOverlayUiState::new(10);
        state.list_state = ListState::default().with_offset(4).with_selected(Some(5));
        state.move_page(1, 4);

        state.set_search_query("bugfix");

        assert_eq!(state.browser_state().search_query, "bugfix");
        assert_eq!(state.browser_state().page_index, 0);
        assert_eq!(state.list_state.selected(), None);
        assert_eq!(state.list_state.offset(), 0);
    }

    #[test]
    fn project_filter_resets_page_and_selection() {
        let mut state = SessionOverlayUiState::new(10);
        state.list_state = ListState::default().with_offset(3).with_selected(Some(4));
        state.move_page(2, 5);

        state.set_project_filter(SessionProjectFilter::RecentProject {
            workspace_directory: "/tmp/root".to_string(),
        });

        assert_eq!(
            state.browser_state().project_filter,
            SessionProjectFilter::RecentProject {
                workspace_directory: "/tmp/root".to_string(),
            }
        );
        assert_eq!(state.browser_state().page_index, 0);
        assert_eq!(state.list_state.selected(), None);
    }

    #[test]
    fn move_page_clamps_and_clears_list_state() {
        let mut state = SessionOverlayUiState::new(10);
        state.list_state = ListState::default().with_offset(2).with_selected(Some(1));

        state.move_page(4, 2);

        assert_eq!(state.browser_state().page_index, 1);
        assert_eq!(state.list_state.selected(), None);
        assert_eq!(state.list_state.offset(), 0);
    }
}
