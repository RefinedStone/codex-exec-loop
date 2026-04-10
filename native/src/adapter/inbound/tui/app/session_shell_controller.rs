use super::session_browser::{
    SessionBrowserSelection, SessionBrowserView, build_session_browser_view,
};
use super::*;
use crate::application::service::session_service::project_recent_sessions;

impl NativeTuiApp {
    pub(super) fn current_session(&self) -> Option<&SessionSummary> {
        self.current_session_browser_view()
            .and_then(|browser_view| browser_view.selected_session())
    }

    pub(super) fn open_conversation_shell(&mut self) {
        self.dispatch_conversation_intent(ConversationIntentEvent::SessionOpenRequested {
            session: self.current_session().cloned(),
        });
    }

    pub(super) fn move_selection(&mut self, delta: isize) {
        let Some(next_selection) = self
            .current_session_browser_view()
            .map(|browser_view| browser_view.selection_after_delta(delta))
        else {
            return;
        };

        self.apply_session_browser_selection(next_selection);
    }

    pub(super) fn jump_to_first_session(&mut self) {
        self.session_overlay_ui_state.jump_to_first_page();
        let next_selection = self
            .current_session_browser_view()
            .map(|browser_view| browser_view.first_selection())
            .unwrap_or(SessionBrowserSelection {
                index: 0,
                session_id: None,
            });
        self.apply_session_browser_selection(next_selection);
    }

    pub(super) fn jump_to_last_session(&mut self) {
        let total_pages = self
            .current_session_browser_view()
            .map(|browser_view| browser_view.projection.total_pages)
            .unwrap_or(0);
        self.session_overlay_ui_state.jump_to_last_page(total_pages);
        let next_selection = self
            .current_session_browser_view()
            .map(|browser_view| browser_view.last_selection())
            .unwrap_or(SessionBrowserSelection {
                index: 0,
                session_id: None,
            });
        self.apply_session_browser_selection(next_selection);
    }

    pub(super) fn clear_session_browser_state(&mut self) {
        self.selected_session_index = 0;
        self.session_overlay_ui_state.clear_browser_state();
        self.sync_session_browser_selection();
    }

    fn current_session_browser_view(&self) -> Option<SessionBrowserView<'_>> {
        let current_workspace_directory = self.current_workspace_directory();
        match &self.session_state {
            SessionState::Ready(recent_sessions) => Some(build_session_browser_view(
                recent_sessions,
                self.session_overlay_ui_state.browser_state(),
                Some(current_workspace_directory.as_str()),
                self.session_overlay_ui_state.selected_session_id(),
                self.selected_session_index,
            )),
            _ => None,
        }
    }

    fn apply_session_browser_selection(&mut self, selection: SessionBrowserSelection) {
        self.selected_session_index = selection.index;
        self.session_overlay_ui_state
            .set_selected_session_id(selection.session_id);
    }

    fn sync_session_browser_selection(&mut self) {
        let (selected_session_index, selected_session_id) =
            match self.current_session_browser_view() {
                Some(browser_view) => (
                    browser_view.selected_index.unwrap_or(0),
                    browser_view
                        .selected_session()
                        .map(|session| session.id.clone()),
                ),
                None => (0, None),
            };

        self.selected_session_index = selected_session_index;
        self.session_overlay_ui_state
            .set_selected_session_id(selected_session_id);
    }

    pub(super) fn is_session_search_query_editing(&self) -> bool {
        self.session_overlay_ui_state.is_search_query_editing()
    }

    pub(super) fn start_session_search_query_edit(&mut self) {
        if self.shell_overlay != ShellOverlay::Sessions {
            return;
        }

        self.session_overlay_ui_state.start_search_query_edit();
    }

    pub(super) fn save_session_search_query_edit(&mut self) {
        if !self.is_session_search_query_editing() {
            return;
        }

        self.session_overlay_ui_state.save_search_query_edit();
        self.sync_session_browser_selection();
    }

    pub(super) fn cancel_session_search_query_edit(&mut self) {
        if !self.is_session_search_query_editing() {
            return;
        }

        self.session_overlay_ui_state.cancel_search_query_edit();
    }

    pub(super) fn push_session_search_query_character(&mut self, character: char) {
        self.session_overlay_ui_state
            .push_search_query_character(character);
    }

    pub(super) fn pop_session_search_query_character(&mut self) {
        self.session_overlay_ui_state.pop_search_query_character();
    }

    pub(super) fn cycle_session_project_filter(&mut self, delta: isize) {
        let SessionState::Ready(recent_sessions) = &self.session_state else {
            return;
        };

        let projection = project_recent_sessions(
            recent_sessions,
            self.session_overlay_ui_state.browser_state(),
            Some(self.current_workspace_directory().as_str()),
        );
        let Some(next_filter) = projection.cycled_project_filter(delta) else {
            return;
        };

        self.session_overlay_ui_state
            .set_project_filter(next_filter);
        self.sync_session_browser_selection();
    }

    pub(super) fn move_session_page(&mut self, delta: isize) {
        let SessionState::Ready(recent_sessions) = &self.session_state else {
            return;
        };

        let total_pages = project_recent_sessions(
            recent_sessions,
            self.session_overlay_ui_state.browser_state(),
            Some(self.current_workspace_directory().as_str()),
        )
        .total_pages;
        self.session_overlay_ui_state.move_page(delta, total_pages);
        self.sync_session_browser_selection();
    }

    pub(super) fn handle_session_search_query_editor_key(&mut self, key: event::KeyEvent) -> bool {
        if self.shell_overlay != ShellOverlay::Sessions || !self.is_session_search_query_editing() {
            return false;
        }

        match key.code {
            KeyCode::Enter if key.modifiers.is_empty() => self.save_session_search_query_edit(),
            KeyCode::Esc => self.cancel_session_search_query_edit(),
            KeyCode::Char('c') if key.modifiers == KeyModifiers::CONTROL => {
                self.cancel_session_search_query_edit()
            }
            KeyCode::Backspace => self.pop_session_search_query_character(),
            KeyCode::Char(character)
                if key.modifiers == KeyModifiers::NONE || key.modifiers == KeyModifiers::SHIFT =>
            {
                self.push_session_search_query_character(character);
            }
            _ => {}
        }

        true
    }

    pub(super) fn handle_session_overlay_key(&mut self, key: event::KeyEvent) -> bool {
        if self.shell_overlay != ShellOverlay::Sessions {
            return false;
        }

        match key.code {
            KeyCode::Char('r') if key.modifiers.is_empty() => {
                if self.can_open_session_list() {
                    self.dispatch_shell_chrome(ShellChromeEvent::SessionsRequested {
                        limit: SESSION_PAGE_SIZE,
                    });
                }
            }
            KeyCode::Char('n') if key.modifiers.is_empty() => {
                self.open_new_conversation_shell();
            }
            KeyCode::Char('c') if key.modifiers.is_empty() => self.clear_session_browser_state(),
            KeyCode::Char('/') if key.modifiers.is_empty() => {
                self.start_session_search_query_edit()
            }
            KeyCode::Tab if key.modifiers.is_empty() => self.cycle_session_project_filter(1),
            KeyCode::BackTab => self.cycle_session_project_filter(-1),
            KeyCode::Home if key.modifiers.is_empty() => self.jump_to_first_session(),
            KeyCode::End if key.modifiers.is_empty() => self.jump_to_last_session(),
            KeyCode::Char('g') if key.modifiers.is_empty() => self.jump_to_first_session(),
            KeyCode::Char('G') if key.modifiers == KeyModifiers::SHIFT => {
                self.jump_to_last_session()
            }
            KeyCode::PageUp if key.modifiers.is_empty() => self.move_session_page(-1),
            KeyCode::PageDown if key.modifiers.is_empty() => self.move_session_page(1),
            KeyCode::Char('[') if key.modifiers.is_empty() => self.move_session_page(-1),
            KeyCode::Char(']') if key.modifiers.is_empty() => self.move_session_page(1),
            KeyCode::Up | KeyCode::Char('k') if key.modifiers.is_empty() => self.move_selection(-1),
            KeyCode::Down | KeyCode::Char('j') if key.modifiers.is_empty() => {
                self.move_selection(1)
            }
            KeyCode::Enter if key.modifiers.is_empty() => self.open_conversation_shell(),
            KeyCode::Char('d') if key.modifiers == KeyModifiers::CONTROL => {
                self.show_startup_overlay()
            }
            _ => {}
        }
        true
    }
}
