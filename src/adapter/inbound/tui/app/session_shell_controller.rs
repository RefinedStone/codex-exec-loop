use crossterm::event::{self, KeyCode, KeyModifiers};

use super::{
    ConversationInputEvent, ConversationIntentEvent, ConversationRuntimeEvent, NativeTuiApp,
    SESSION_PAGE_SIZE, SessionState, ShellChromeEvent, ShellOverlay,
};
use crate::core::app::{
    AppCommand, SessionCatalogSnapshot, SessionRenameAcceptedSnapshot, SessionRenameCorrelation,
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

    pub(super) fn is_session_rename_editing(&self) -> bool {
        self.session_overlay_ui_state.is_rename_editing()
    }

    pub(super) fn start_session_rename_edit(&mut self) {
        if self.shell_overlay != ShellOverlay::Sessions {
            return;
        }
        let Some(session) = self.current_session() else {
            self.dispatch_conversation_input(ConversationInputEvent::StatusMessageShown {
                status_text: self.tui_language.session_rename_select_status().to_string(),
            });
            return;
        };
        let thread_id = session.id.clone();
        let name = session
            .name
            .as_deref()
            .map(str::trim)
            .filter(|name| !name.is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| session.title());
        self.session_overlay_ui_state
            .start_rename_edit(thread_id, name);
    }

    pub(super) fn handle_session_rename_editor_key(&mut self, key: event::KeyEvent) -> bool {
        if self.shell_overlay != ShellOverlay::Sessions || !self.is_session_rename_editing() {
            return false;
        }
        match key.code {
            KeyCode::Enter if key.modifiers.is_empty() => self.submit_session_rename(),
            KeyCode::Esc => self
                .session_overlay_ui_state
                .cancel_rename_edit(self.tui_language),
            KeyCode::Char('c') if key.modifiers == KeyModifiers::CONTROL => self
                .session_overlay_ui_state
                .cancel_rename_edit(self.tui_language),
            KeyCode::Backspace => self.session_overlay_ui_state.pop_rename_character(),
            KeyCode::Char(character)
                if key.modifiers == KeyModifiers::NONE || key.modifiers == KeyModifiers::SHIFT =>
            {
                self.session_overlay_ui_state
                    .push_rename_character(character);
            }
            _ => {}
        }
        true
    }

    pub(super) fn handle_session_rename_paste(&mut self, text: &str) -> bool {
        if self.shell_overlay != ShellOverlay::Sessions || !self.is_session_rename_editing() {
            return false;
        }
        self.session_overlay_ui_state.push_rename_text(text);
        true
    }

    fn submit_session_rename(&mut self) {
        let Some(request) = self
            .session_overlay_ui_state
            .prepare_rename_request(self.tui_language)
        else {
            if let Some(feedback) = self
                .session_overlay_ui_state
                .rename_editor_feedback()
                .map(str::to_string)
            {
                self.dispatch_conversation_input(ConversationInputEvent::StatusMessageShown {
                    status_text: feedback,
                });
            }
            return;
        };

        self.dispatch_conversation_input(ConversationInputEvent::StatusMessageShown {
            status_text: self
                .tui_language
                .session_rename_started_status(&request.name),
        });
        self.dispatch_core_command(AppCommand::RenameSession(request));
    }

    pub(super) fn apply_session_rename_completion(
        &mut self,
        correlation: SessionRenameCorrelation,
        result: Result<SessionRenameAcceptedSnapshot, String>,
    ) {
        if !self
            .session_overlay_ui_state
            .pending_rename_matches(&correlation.request)
        {
            return;
        }

        match result {
            Ok(accepted) => {
                self.apply_session_catalog_projection(accepted.session_catalog);
                if let Some(stream_snapshot) = accepted.turn_stream {
                    self.dispatch_conversation_runtime(
                        ConversationRuntimeEvent::StreamSnapshotApplied(stream_snapshot),
                    );
                }
                self.session_overlay_ui_state.finish_rename_success();
                self.session_overlay_ui_state
                    .set_selected_session_id(Some(correlation.request.thread_id.clone()));
                self.dispatch_conversation_input(ConversationInputEvent::StatusMessageShown {
                    status_text: self
                        .tui_language
                        .session_renamed_status(&correlation.request.name),
                });
            }
            Err(reason) => {
                self.session_overlay_ui_state
                    .finish_rename_failure(&reason, self.tui_language);
                self.dispatch_conversation_input(ConversationInputEvent::StatusMessageShown {
                    status_text: self.tui_language.session_rename_failed_status(&reason),
                });
            }
        }
    }

    fn apply_session_catalog_projection(&mut self, snapshot: SessionCatalogSnapshot) {
        self.session_state = match snapshot {
            SessionCatalogSnapshot::Idle => SessionState::Idle,
            SessionCatalogSnapshot::Loading => SessionState::Loading,
            SessionCatalogSnapshot::Ready(ready) => SessionState::Ready(*ready.catalog),
            SessionCatalogSnapshot::Failed { message } => SessionState::Failed(message),
        };
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
            KeyCode::Char('e') if key.modifiers.is_empty() => self.start_session_rename_edit(),
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
    use std::sync::{Arc, Mutex, mpsc};
    use std::time::{Duration, Instant};

    use anyhow::Result;

    use super::*;
    use crate::adapter::inbound::tui::app::language::TuiLanguage;
    use crate::adapter::inbound::tui::app::test_helpers::{
        test_native_tui_app, test_native_tui_app_with_session_catalog_port,
    };
    use crate::adapter::inbound::tui::app::{ConversationState, StartupState};
    use crate::application::port::outbound::session_catalog_port::SessionCatalogPort;
    use crate::core::app::TurnStreamState;
    use crate::domain::recent_sessions::{
        RecentSessions, SessionCatalog, SessionCatalogRequest, SessionCatalogTier,
        SessionRenameRequest,
    };
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

    fn catalog_snapshot(sessions: Vec<SessionSummary>) -> SessionCatalogSnapshot {
        SessionCatalogSnapshot::Ready(crate::core::app::SessionCatalogReadySnapshot::from_catalog(
            SessionCatalog::ready(
                SessionCatalogTier::ProviderBackedCatalog,
                RecentSessions {
                    items: sessions,
                    warnings: Vec::new(),
                    next_cursor: None,
                },
            ),
        ))
    }

    fn accepted_rename_snapshot(
        sessions: Vec<SessionSummary>,
        thread_id: &str,
        previous_title: &str,
        renamed_title: &str,
    ) -> SessionRenameAcceptedSnapshot {
        let mut stream = TurnStreamState::new();
        stream.seed_loaded_thread_identity(thread_id, previous_title, "/tmp/root");
        SessionRenameAcceptedSnapshot {
            session_catalog: catalog_snapshot(sessions),
            turn_stream: stream
                .apply_session_rename(thread_id, renamed_title)
                .map(Box::new),
        }
    }

    fn selected_session_id(app: &NativeTuiApp) -> Option<&str> {
        app.current_session().map(|session| session.id.as_str())
    }

    struct RecordingSessionCatalogPort {
        sessions: Mutex<Vec<SessionSummary>>,
        rename_error: Option<String>,
        rename_requests: Mutex<Vec<SessionRenameRequest>>,
        rename_gate: Option<RenameGate>,
    }

    struct RenameGate {
        started: mpsc::SyncSender<()>,
        release: Mutex<mpsc::Receiver<()>>,
    }

    impl RecordingSessionCatalogPort {
        fn new(rename_error: Option<&str>) -> Self {
            Self {
                sessions: Mutex::new(vec![session("thread-beta", "Beta draft", "/tmp/root")]),
                rename_error: rename_error.map(str::to_string),
                rename_requests: Mutex::new(Vec::new()),
                rename_gate: None,
            }
        }

        fn with_rename_gate(
            mut self,
            started: mpsc::SyncSender<()>,
            release: mpsc::Receiver<()>,
        ) -> Self {
            self.rename_gate = Some(RenameGate {
                started,
                release: Mutex::new(release),
            });
            self
        }
    }

    impl SessionCatalogPort for RecordingSessionCatalogPort {
        fn load_session_catalog(&self, _request: SessionCatalogRequest) -> Result<SessionCatalog> {
            Ok(SessionCatalog::ready(
                SessionCatalogTier::ProviderBackedCatalog,
                RecentSessions {
                    items: self
                        .sessions
                        .lock()
                        .expect("session catalog mutex poisoned")
                        .clone(),
                    warnings: Vec::new(),
                    next_cursor: None,
                },
            ))
        }

        fn rename_session(&self, request: SessionRenameRequest) -> Result<()> {
            self.rename_requests
                .lock()
                .expect("rename request mutex poisoned")
                .push(request.clone());
            if let Some(gate) = &self.rename_gate {
                gate.started
                    .send(())
                    .map_err(|_| anyhow::anyhow!("rename test gate start receiver closed"))?;
                gate.release
                    .lock()
                    .expect("rename test gate mutex poisoned")
                    .recv_timeout(Duration::from_secs(1))
                    .map_err(|error| {
                        anyhow::anyhow!("rename test gate was not released: {error}")
                    })?;
            }
            if let Some(error) = &self.rename_error {
                anyhow::bail!(error.clone());
            }
            if let Some(session) = self
                .sessions
                .lock()
                .expect("session catalog mutex poisoned")
                .iter_mut()
                .find(|session| session.id == request.thread_id)
            {
                session.name = Some(request.name);
            }
            Ok(())
        }
    }

    fn poll_until(app: &mut NativeTuiApp, complete: impl Fn(&NativeTuiApp) -> bool) {
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            app.poll_core_runtime_inputs(8);
            if complete(app) {
                return;
            }
            assert!(
                Instant::now() < deadline,
                "core runtime did not settle within 30 seconds"
            );
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    fn load_recording_catalog(app: &mut NativeTuiApp) {
        app.dispatch_core_command(AppCommand::RunStartupChecks);
        poll_until(app, |app| {
            matches!(app.startup_state, StartupState::Ready(_))
                && matches!(app.session_state, SessionState::Ready(_))
        });
        app.shell_overlay = ShellOverlay::Sessions;
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
            app.pending_conversation_load
                .as_ref()
                .map(|load| load.requested_thread_id.as_str()),
            Some("thread-beta")
        );
        assert!(matches!(app.conversation_state, ConversationState::Loading));
    }

    #[test]
    fn rename_editor_prefills_selected_name_and_cancel_preserves_catalog() {
        let mut app = test_native_tui_app();
        seed_sessions(
            &mut app,
            vec![session("thread-alpha", "Alpha draft", "/tmp/root")],
        );

        assert!(app.handle_session_overlay_key(key(KeyCode::Char('e'))));
        assert!(app.is_session_rename_editing());
        assert_eq!(
            app.session_overlay_ui_state.rename_editor_thread_id(),
            Some("thread-alpha")
        );
        assert_eq!(
            app.session_overlay_ui_state.rename_editor_buffer(),
            "Alpha draft"
        );

        assert!(app.handle_session_rename_editor_key(key(KeyCode::Char('x'))));
        assert!(app.handle_session_rename_editor_key(key(KeyCode::Backspace)));
        assert!(app.handle_session_rename_editor_key(key(KeyCode::Esc)));

        assert!(!app.is_session_rename_editing());
        assert_eq!(selected_session_id(&app), Some("thread-alpha"));
        assert_eq!(
            app.current_session().map(SessionSummary::title).as_deref(),
            Some("Alpha draft")
        );

        if let SessionState::Ready(crate::domain::recent_sessions::SessionCatalog::Ready {
            recent_sessions,
            ..
        }) = &mut app.session_state
        {
            recent_sessions.items[0].name = None;
            recent_sessions.items[0].preview = "Fallback preview title\nsecond line".to_string();
        }
        app.start_session_rename_edit();
        assert_eq!(
            app.session_overlay_ui_state.rename_editor_buffer(),
            "Fallback preview title"
        );
        app.session_overlay_ui_state
            .cancel_rename_edit(TuiLanguage::English);
    }

    #[test]
    fn rename_completion_updates_exact_row_and_failure_keeps_editor_draft() {
        let mut app = test_native_tui_app();
        seed_sessions(
            &mut app,
            vec![
                session("thread-alpha", "Alpha draft", "/tmp/root"),
                session("thread-beta", "Beta draft", "/tmp/root"),
            ],
        );
        app.move_selection(1);
        let ConversationState::Ready(conversation) = &mut app.conversation_state else {
            panic!("test conversation should be ready");
        };
        conversation.thread_id = "thread-beta".to_string();
        conversation.title = "Beta draft".to_string();
        app.start_session_rename_edit();
        let failed_request = app
            .session_overlay_ui_state
            .prepare_rename_request(TuiLanguage::English)
            .expect("rename request should prepare");

        app.apply_session_rename_completion(
            SessionRenameCorrelation::new(1, failed_request),
            Err("provider unavailable".to_string()),
        );

        assert!(app.is_session_rename_editing());
        assert_eq!(
            app.session_overlay_ui_state.rename_editor_buffer(),
            "Beta draft"
        );

        let retry_request = app
            .session_overlay_ui_state
            .prepare_rename_request(TuiLanguage::English)
            .expect("same-value retry should prepare");
        app.apply_session_rename_completion(
            SessionRenameCorrelation::new(2, retry_request),
            Err("retry required".to_string()),
        );

        app.session_overlay_ui_state.pop_rename_character();
        app.session_overlay_ui_state.push_rename_character('2');
        let success_request = app
            .session_overlay_ui_state
            .prepare_rename_request(TuiLanguage::English)
            .expect("retry should prepare");
        app.apply_session_rename_completion(
            SessionRenameCorrelation::new(3, success_request),
            Ok(accepted_rename_snapshot(
                vec![
                    session("thread-alpha", "Alpha draft", "/tmp/root"),
                    session("thread-beta", "Beta draf2", "/tmp/root"),
                ],
                "thread-beta",
                "Beta draft",
                "Beta draf2",
            )),
        );

        assert!(!app.is_session_rename_editing());
        assert_eq!(selected_session_id(&app), Some("thread-beta"));
        assert_eq!(
            app.current_session().map(SessionSummary::title).as_deref(),
            Some("Beta draf2")
        );
        assert!(matches!(
            &app.conversation_state,
            ConversationState::Ready(conversation) if conversation.title == "Beta draf2"
        ));
        app.move_selection(-1);
        assert_eq!(
            app.current_session().map(SessionSummary::title).as_deref(),
            Some("Alpha draft")
        );
    }

    #[test]
    fn rename_enter_runs_service_completion_back_through_core_runtime() {
        let port = Arc::new(RecordingSessionCatalogPort::new(None));
        let mut app = test_native_tui_app_with_session_catalog_port(port.clone());
        load_recording_catalog(&mut app);
        app.dispatch_core_command(AppCommand::LoadConversation {
            thread_id: "thread-beta".to_string(),
            fallback_workspace_directory: "/tmp/root".to_string(),
        });
        poll_until(&mut app, |app| {
            matches!(
                &app.conversation_state,
                ConversationState::Ready(conversation)
                    if conversation.thread_id == "thread-beta"
            )
        });
        app.shell_overlay = ShellOverlay::Sessions;
        app.start_session_rename_edit();
        while !app
            .session_overlay_ui_state
            .rename_editor_buffer()
            .is_empty()
        {
            app.session_overlay_ui_state.pop_rename_character();
        }
        app.handle_session_rename_paste("Beta renamed");

        assert!(app.handle_session_rename_editor_key(key(KeyCode::Enter)));
        assert!(app.session_overlay_ui_state.is_rename_pending());
        poll_until(&mut app, |app| {
            !app.session_overlay_ui_state.is_rename_pending()
        });

        assert_eq!(
            *port
                .rename_requests
                .lock()
                .expect("rename request mutex poisoned"),
            vec![SessionRenameRequest::new("thread-beta", "Beta renamed")]
        );
        assert!(!app.is_session_rename_editing());
        assert_eq!(
            app.current_session().map(SessionSummary::title).as_deref(),
            Some("Beta renamed")
        );
        assert!(matches!(
            &app.conversation_state,
            ConversationState::Ready(conversation)
                if conversation.thread_id == "thread-beta"
                    && conversation.title == "Beta renamed"
        ));
    }

    #[test]
    fn rename_service_failure_returns_to_the_editor_with_its_draft() {
        let port = Arc::new(RecordingSessionCatalogPort::new(Some(
            "provider unavailable",
        )));
        let mut app = test_native_tui_app_with_session_catalog_port(port.clone());
        load_recording_catalog(&mut app);
        app.start_session_rename_edit();

        assert!(app.handle_session_rename_editor_key(key(KeyCode::Enter)));
        poll_until(&mut app, |app| {
            !app.session_overlay_ui_state.is_rename_pending()
        });

        assert_eq!(
            *port
                .rename_requests
                .lock()
                .expect("rename request mutex poisoned"),
            vec![SessionRenameRequest::new("thread-beta", "Beta draft")]
        );
        assert!(app.is_session_rename_editing());
        assert_eq!(
            app.session_overlay_ui_state.rename_editor_buffer(),
            "Beta draft"
        );
        assert_eq!(
            app.current_session().map(SessionSummary::title).as_deref(),
            Some("Beta draft")
        );
    }

    #[test]
    fn catalog_reload_during_rename_is_deferred_without_sticking_loading() {
        let (rename_started_tx, rename_started_rx) = mpsc::sync_channel(1);
        let (release_rename_tx, release_rename_rx) = mpsc::sync_channel(1);
        let port = Arc::new(
            RecordingSessionCatalogPort::new(None)
                .with_rename_gate(rename_started_tx, release_rename_rx),
        );
        let mut app = test_native_tui_app_with_session_catalog_port(port);
        load_recording_catalog(&mut app);
        app.start_session_rename_edit();
        while !app
            .session_overlay_ui_state
            .rename_editor_buffer()
            .is_empty()
        {
            app.session_overlay_ui_state.pop_rename_character();
        }
        app.handle_session_rename_paste("Beta renamed");
        app.handle_session_rename_editor_key(key(KeyCode::Enter));
        rename_started_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("rename worker should reach the test gate");

        app.dispatch_shell_chrome(ShellChromeEvent::SessionsRequested { limit: 10 });
        assert!(matches!(app.session_state, SessionState::Loading));
        release_rename_tx
            .send(())
            .expect("rename worker should still be waiting");
        poll_until(&mut app, |app| {
            !app.session_overlay_ui_state.is_rename_pending()
                && matches!(app.session_state, SessionState::Ready(_))
        });

        assert_eq!(
            app.current_session().map(SessionSummary::title).as_deref(),
            Some("Beta renamed")
        );
    }
}
