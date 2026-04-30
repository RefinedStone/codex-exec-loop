use ratatui::widgets::ListState;

use crate::domain::session_browser::{SessionBrowserState, SessionProjectFilter};

#[derive(Debug, Default)]
struct SessionSearchQueryEditorState {
    is_editing: bool,
    buffer: String,
}

#[derive(Debug)]
pub(super) struct SessionOverlayUiState {
    pub list_state: ListState,
    browser_state: SessionBrowserState,
    selected_session_id: Option<String>,
    search_query_editor: SessionSearchQueryEditorState,
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
            selected_session_id: None,
            search_query_editor: SessionSearchQueryEditorState::default(),
        }
    }

    pub fn browser_state(&self) -> &SessionBrowserState {
        &self.browser_state
    }

    pub fn selected_session_id(&self) -> Option<&str> {
        self.selected_session_id.as_deref()
    }

    pub fn set_selected_session_id(&mut self, selected_session_id: Option<String>) {
        self.selected_session_id = selected_session_id;
    }

    pub fn is_search_query_editing(&self) -> bool {
        self.search_query_editor.is_editing
    }

    pub fn search_query_editor_buffer(&self) -> &str {
        &self.search_query_editor.buffer
    }

    pub fn start_search_query_edit(&mut self) {
        self.search_query_editor.is_editing = true;
        self.search_query_editor.buffer = self.browser_state.search_query.clone();
    }

    pub fn save_search_query_edit(&mut self) {
        let next_query = self.search_query_editor.buffer.clone();
        self.set_search_query(next_query);
        self.search_query_editor.is_editing = false;
        self.search_query_editor.buffer = self.browser_state.search_query.clone();
    }

    pub fn cancel_search_query_edit(&mut self) {
        self.search_query_editor.is_editing = false;
        self.search_query_editor.buffer = self.browser_state.search_query.clone();
    }

    pub fn push_search_query_character(&mut self, character: char) {
        self.search_query_editor.buffer.push(character);
    }

    pub fn pop_search_query_character(&mut self) {
        self.search_query_editor.buffer.pop();
    }

    pub fn set_search_query(&mut self, search_query: impl Into<String>) {
        self.browser_state.set_search_query(search_query);
        self.list_state = ListState::default();
    }

    pub fn set_project_filter(&mut self, project_filter: SessionProjectFilter) {
        self.browser_state.set_project_filter(project_filter);
        self.list_state = ListState::default();
    }

    pub fn move_page(&mut self, delta: isize, total_pages: usize) {
        self.browser_state.move_page(delta, total_pages);
        self.list_state = ListState::default();
    }

    pub fn jump_to_first_page(&mut self) {
        self.browser_state.jump_to_first_page();
        self.list_state = ListState::default();
    }

    pub fn jump_to_last_page(&mut self, total_pages: usize) {
        self.browser_state.jump_to_last_page(total_pages);
        self.list_state = ListState::default();
    }

    pub fn clear_browser_state(&mut self) {
        self.browser_state.clear();
        self.list_state = ListState::default();
        self.selected_session_id = None;
        self.search_query_editor.is_editing = false;
        self.search_query_editor.buffer = self.browser_state.search_query.clone();
    }

    pub fn sync_selected_session(&mut self, selected_session_index: Option<usize>) {
        self.list_state.select(selected_session_index);
    }

    pub fn reset(&mut self) {
        self.list_state = ListState::default();
        self.selected_session_id = None;
        self.search_query_editor.is_editing = false;
        self.search_query_editor.buffer = self.browser_state.search_query.clone();
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
    fn reset_clears_selection_editor_and_selected_session_but_keeps_browser_query() {
        let mut state = SessionOverlayUiState::new(10);
        state.list_state = ListState::default().with_offset(4).with_selected(Some(5));
        state.set_search_query("bugfix");
        state.set_selected_session_id(Some("thread-2".to_string()));
        state.start_search_query_edit();
        state.push_search_query_character('x');

        state.reset();

        assert_eq!(state.list_state.selected(), None);
        assert_eq!(state.list_state.offset(), 0);
        assert_eq!(state.selected_session_id(), None);
        assert!(!state.is_search_query_editing());
        assert_eq!(state.search_query_editor_buffer(), "bugfix");
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

    #[test]
    fn clear_browser_state_resets_query_filter_selection_and_editor() {
        let mut state = SessionOverlayUiState::new(10);
        state.set_search_query("docs");
        state.set_project_filter(SessionProjectFilter::RecentProject {
            workspace_directory: "/tmp/root".to_string(),
        });
        state.move_page(3, 5);
        state.set_selected_session_id(Some("thread-2".to_string()));
        state.start_search_query_edit();
        state.push_search_query_character('x');

        state.clear_browser_state();

        assert_eq!(state.browser_state().search_query, "");
        assert_eq!(state.browser_state().page_index, 0);
        assert_eq!(
            state.browser_state().project_filter,
            SessionProjectFilter::AllProjects
        );
        assert_eq!(state.selected_session_id(), None);
        assert!(!state.is_search_query_editing());
        assert_eq!(state.search_query_editor_buffer(), "");
        assert_eq!(state.list_state.selected(), None);
        assert_eq!(state.list_state.offset(), 0);
    }

    #[test]
    fn jump_to_last_page_clamps_and_clears_list_state() {
        let mut state = SessionOverlayUiState::new(10);
        state.list_state = ListState::default().with_offset(2).with_selected(Some(1));

        state.jump_to_last_page(3);

        assert_eq!(state.browser_state().page_index, 2);
        assert_eq!(state.list_state.selected(), None);
        assert_eq!(state.list_state.offset(), 0);
    }

    #[test]
    fn save_search_query_edit_commits_trimmed_query() {
        let mut state = SessionOverlayUiState::new(10);
        state.start_search_query_edit();
        state.push_search_query_character(' ');
        state.push_search_query_character('d');
        state.push_search_query_character('o');
        state.push_search_query_character('c');
        state.push_search_query_character('s');
        state.push_search_query_character(' ');

        state.save_search_query_edit();

        assert!(!state.is_search_query_editing());
        assert_eq!(state.browser_state().search_query, "docs");
        assert_eq!(state.search_query_editor_buffer(), "docs");
    }

    #[test]
    fn cancel_search_query_edit_restores_saved_query() {
        let mut state = SessionOverlayUiState::new(10);
        state.set_search_query("release");
        state.start_search_query_edit();
        state.push_search_query_character(' ');
        state.push_search_query_character('x');

        state.cancel_search_query_edit();

        assert!(!state.is_search_query_editing());
        assert_eq!(state.browser_state().search_query, "release");
        assert_eq!(state.search_query_editor_buffer(), "release");
    }
}
