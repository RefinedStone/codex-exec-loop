use crossterm::event::{self, KeyCode, KeyModifiers};

use super::{
    ConversationIntentEvent, NativeTuiApp, SESSION_PAGE_SIZE, SessionState, ShellChromeEvent,
    ShellOverlay,
};
use crate::domain::session_browser::{
    SessionBrowserPage, SessionBrowserSelection, build_session_browser_page,
};
use crate::domain::session_summary::SessionSummary;

/*
 * Session shell control sits between raw keyboard events and the domain-level session browser
 * projection. The domain module owns filtering, ranking, paging, and stable selection resolution;
 * this controller keeps NativeTuiApp's mutable overlay state in sync and converts the final
 * selected SessionSummary into conversation intents.
 */
impl NativeTuiApp {
    pub(super) fn current_session(&self) -> Option<&SessionSummary> {
        // Always read through the projected page so search/project filters and stale selection ids apply.
        self.current_session_browser_page()
            .and_then(|browser_page| browser_page.selected_session())
    }

    pub(super) fn open_conversation_shell(&mut self) {
        /*
         * Opening a session is routed through ConversationIntentEvent rather than directly mutating
         * conversation_state. That keeps attach/load behavior in conversation lifecycle code, while
         * this controller only supplies the selected catalog row.
         */
        self.dispatch_conversation_intent(ConversationIntentEvent::SessionOpenRequested {
            session: self.current_session().cloned().map(Box::new),
        });
    }

    pub(super) fn move_selection(&mut self, delta: isize) {
        // Domain projection clamps movement and preserves selection by session id across filters/pages.
        let Some(next_selection) = self
            .current_session_browser_page()
            .map(|browser_page| browser_page.selection_after_delta(delta))
        else {
            return;
        };

        self.apply_session_browser_selection(next_selection);
    }

    pub(super) fn jump_to_first_session(&mut self) {
        // Page movement happens in overlay UI state, then selection is recomputed from the page view.
        self.session_overlay_ui_state.jump_to_first_page();
        let next_selection = self
            .current_session_browser_page()
            .map(|browser_page| browser_page.first_selection())
            .unwrap_or(SessionBrowserSelection {
                index: 0,
                session_id: None,
            });
        self.apply_session_browser_selection(next_selection);
    }

    pub(super) fn jump_to_last_session(&mut self) {
        let total_pages = self
            .current_session_browser_page()
            .map(|browser_page| browser_page.projection.total_pages)
            .unwrap_or(0);
        self.session_overlay_ui_state.jump_to_last_page(total_pages);
        let next_selection = self
            .current_session_browser_page()
            .map(|browser_page| browser_page.last_selection())
            .unwrap_or(SessionBrowserSelection {
                index: 0,
                session_id: None,
            });
        self.apply_session_browser_selection(next_selection);
    }

    pub(super) fn clear_session_browser_state(&mut self) {
        // Clear query/filter/page state and then normalize selected_session_id against the fresh page.
        self.selected_session_index = 0;
        self.session_overlay_ui_state.clear_browser_state();
        self.sync_session_browser_selection();
    }

    fn current_session_browser_page(&self) -> Option<SessionBrowserPage<'_>> {
        /*
         * The app stores raw catalog readiness plus UI state separately. Building the page on demand
         * lets renderers and key handlers share one projection that accounts for current workspace,
         * project filter, search query, page index, and selected session id.
         */
        let current_workspace_directory = self.current_workspace_directory();
        if let SessionState::Ready(catalog) = &self.session_state
            && let Some(recent_sessions) = catalog.recent_sessions()
        {
            return Some(build_session_browser_page(
                recent_sessions,
                self.session_overlay_ui_state.browser_state(),
                Some(current_workspace_directory.as_str()),
                self.session_overlay_ui_state.selected_session_id(),
                self.selected_session_index,
            ));
        }
        None
    }

    fn apply_session_browser_selection(&mut self, selection: SessionBrowserSelection) {
        // Store both visible index and stable id: index drives cursor position, id survives resort/filter.
        self.selected_session_index = selection.index;
        self.session_overlay_ui_state
            .set_selected_session_id(selection.session_id);
    }

    fn sync_session_browser_selection(&mut self) {
        /*
         * Search and project-filter changes can remove the previously selected row. Reprojecting
         * here prevents Enter from attaching a stale session after the visible browser has changed.
         */
        let (selected_session_index, selected_session_id) =
            match self.current_session_browser_page() {
                Some(browser_page) => (
                    browser_page.selected_index.unwrap_or(0),
                    browser_page
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
        // Search edit mode is scoped to the Sessions overlay; other overlays reuse '/' differently.
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
        /*
         * Filter options are derived from the current projection because the available project set
         * comes from the catalog itself. After changing filters, selection must be revalidated.
         */
        let Some(browser_page) = self.current_session_browser_page() else {
            return;
        };
        let Some(next_filter) = browser_page.projection.cycled_project_filter(delta) else {
            return;
        };

        self.session_overlay_ui_state
            .set_project_filter(next_filter);
        self.sync_session_browser_selection();
    }

    pub(super) fn move_session_page(&mut self, delta: isize) {
        let Some(browser_page) = self.current_session_browser_page() else {
            return;
        };
        let total_pages = browser_page.projection.total_pages;
        self.session_overlay_ui_state.move_page(delta, total_pages);
        self.sync_session_browser_selection();
    }

    pub(super) fn handle_session_search_query_editor_key(&mut self, key: event::KeyEvent) -> bool {
        /*
         * When the search editor is active, it owns printable characters, backspace, Enter, and
         * cancel keys. Returning true stops the outer overlay handler from also interpreting those
         * keys as navigation commands.
         */
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
        /*
         * This is the Sessions overlay keymap. Commands either refresh the catalog, change browser
         * UI state, or dispatch conversation/startup intents. Unrecognized keys are still consumed
         * while the overlay is active so they do not leak into the conversation prompt.
         */
        if self.shell_overlay != ShellOverlay::Sessions {
            return false;
        }
        match key.code {
            KeyCode::Char('r') if key.modifiers.is_empty() && self.can_open_session_list() => {
                self.dispatch_shell_chrome(ShellChromeEvent::SessionsRequested {
                    limit: SESSION_PAGE_SIZE,
                });
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

    pub(super) fn session_browser_available(&self) -> bool {
        // Render code uses this to decide whether browser-specific affordances should be shown.
        self.current_session_browser_page().is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapter::inbound::tui::app::ConversationState;
    use crate::adapter::inbound::tui::app::test_helpers::test_native_tui_app;
    use crate::domain::recent_sessions::{RecentSessions, SessionCatalog, SessionCatalogTier};
    use crate::domain::session_browser::SessionProjectFilter;

    fn key(code: KeyCode) -> event::KeyEvent {
        event::KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn modified_key(code: KeyCode, modifiers: KeyModifiers) -> event::KeyEvent {
        event::KeyEvent::new(code, modifiers)
    }

    fn session(id: &str, title: &str, cwd: &str) -> SessionSummary {
        SessionSummary {
            id: id.to_string(),
            name: Some(title.to_string()),
            preview: format!("{title} preview"),
            cwd: cwd.to_string(),
            source: "native".to_string(),
            model_provider: "openai".to_string(),
            updated_at_epoch: 1_700_000_000,
            status_type: "ready".to_string(),
            path: format!("{cwd}/{id}.json"),
            git_branch: Some("feature/session-browser".to_string()),
        }
    }

    fn seed_sessions(app: &mut NativeTuiApp, sessions: Vec<SessionSummary>) {
        app.session_state = SessionState::Ready(SessionCatalog::ready(
            SessionCatalogTier::ProviderBackedCatalog,
            RecentSessions {
                items: sessions,
                warnings: Vec::new(),
                next_cursor: None,
            },
        ));
        app.shell_overlay = ShellOverlay::Sessions;
    }

    fn selected_session_id(app: &NativeTuiApp) -> Option<&str> {
        app.current_session().map(|session| session.id.as_str())
    }

    #[test]
    fn session_navigation_reprojects_selection_when_pages_change() {
        let mut app = test_native_tui_app();
        seed_sessions(
            &mut app,
            (0..12)
                .map(|index| {
                    session(
                        &format!("thread-{index:02}"),
                        &format!("Session {index:02}"),
                        "/tmp/root",
                    )
                })
                .collect(),
        );

        assert!(app.session_browser_available());
        assert_eq!(selected_session_id(&app), Some("thread-00"));

        app.jump_to_last_session();

        assert_eq!(app.selected_session_index, 1);
        assert_eq!(selected_session_id(&app), Some("thread-11"));

        app.move_session_page(-1);

        assert_eq!(app.selected_session_index, 1);
        assert_eq!(selected_session_id(&app), Some("thread-01"));

        app.jump_to_first_session();

        assert_eq!(app.selected_session_index, 0);
        assert_eq!(selected_session_id(&app), Some("thread-00"));
    }

    #[test]
    fn search_editor_owns_text_keys_only_in_sessions_overlay() {
        let mut app = test_native_tui_app();
        seed_sessions(
            &mut app,
            vec![
                session("thread-alpha", "Alpha draft", "/tmp/root"),
                session("thread-beta", "Beta bugfix", "/tmp/root"),
            ],
        );

        app.shell_overlay = ShellOverlay::Hidden;
        app.start_session_search_query_edit();
        assert!(!app.is_session_search_query_editing());
        assert!(!app.handle_session_search_query_editor_key(key(KeyCode::Char('b'))));

        app.shell_overlay = ShellOverlay::Sessions;
        assert!(app.handle_session_overlay_key(key(KeyCode::Char('/'))));
        assert!(app.is_session_search_query_editing());

        assert!(app.handle_session_search_query_editor_key(key(KeyCode::Char('b'))));
        assert!(app.handle_session_search_query_editor_key(modified_key(
            KeyCode::Char('E'),
            KeyModifiers::SHIFT,
        )));
        assert!(app.handle_session_search_query_editor_key(key(KeyCode::Backspace)));
        assert_eq!(
            app.session_overlay_ui_state.search_query_editor_buffer(),
            "b"
        );

        assert!(app.handle_session_search_query_editor_key(key(KeyCode::Enter)));

        assert!(!app.is_session_search_query_editing());
        assert_eq!(
            app.session_overlay_ui_state.browser_state().search_query,
            "b"
        );
        assert_eq!(selected_session_id(&app), Some("thread-beta"));

        assert!(app.handle_session_overlay_key(key(KeyCode::Char('/'))));
        assert!(app.handle_session_search_query_editor_key(key(KeyCode::Char('z'))));
        assert!(app.handle_session_search_query_editor_key(key(KeyCode::Esc)));

        assert!(!app.is_session_search_query_editing());
        assert_eq!(
            app.session_overlay_ui_state.browser_state().search_query,
            "b"
        );
        assert_eq!(
            app.session_overlay_ui_state.search_query_editor_buffer(),
            "b"
        );

        assert!(app.handle_session_overlay_key(key(KeyCode::Char('/'))));
        assert!(app.handle_session_search_query_editor_key(modified_key(
            KeyCode::Char('c'),
            KeyModifiers::CONTROL,
        )));

        assert!(!app.is_session_search_query_editing());
        assert_eq!(
            app.session_overlay_ui_state.browser_state().search_query,
            "b"
        );
    }

    #[test]
    fn overlay_keymap_cycles_project_filters_and_clear_resets_browser() {
        let mut app = test_native_tui_app();
        seed_sessions(
            &mut app,
            vec![
                session("thread-root", "Root task", "/tmp/root"),
                session("thread-other", "Other task", "/tmp/other"),
            ],
        );

        app.shell_overlay = ShellOverlay::Hidden;
        assert!(!app.handle_session_overlay_key(key(KeyCode::Tab)));

        app.shell_overlay = ShellOverlay::Sessions;
        assert!(app.handle_session_overlay_key(key(KeyCode::Tab)));
        assert_eq!(
            app.session_overlay_ui_state.browser_state().project_filter,
            SessionProjectFilter::RecentProject {
                workspace_directory: "/tmp/root".to_string(),
            }
        );
        assert_eq!(selected_session_id(&app), Some("thread-root"));

        assert!(app.handle_session_overlay_key(key(KeyCode::BackTab)));
        assert_eq!(
            app.session_overlay_ui_state.browser_state().project_filter,
            SessionProjectFilter::AllProjects,
        );

        app.session_overlay_ui_state.set_search_query("other");
        app.sync_session_browser_selection();
        assert_eq!(selected_session_id(&app), Some("thread-other"));

        assert!(app.handle_session_overlay_key(key(KeyCode::Char('c'))));

        assert_eq!(
            app.session_overlay_ui_state.browser_state().search_query,
            ""
        );
        assert_eq!(
            app.session_overlay_ui_state.browser_state().project_filter,
            SessionProjectFilter::AllProjects,
        );
        assert_eq!(selected_session_id(&app), Some("thread-root"));
    }

    #[test]
    fn open_conversation_shell_routes_selected_session_through_lifecycle() {
        let mut app = test_native_tui_app();
        seed_sessions(
            &mut app,
            vec![
                session("thread-alpha", "Alpha draft", "/tmp/root"),
                session("thread-beta", "Beta bugfix", "/tmp/root"),
            ],
        );
        app.move_selection(1);

        app.open_conversation_shell();

        assert_eq!(app.shell_overlay, ShellOverlay::Hidden);
        assert_eq!(
            app.active_session
                .as_ref()
                .map(|session| session.id.as_str()),
            Some("thread-beta")
        );
        assert!(matches!(app.conversation_state, ConversationState::Loading));
    }
}
