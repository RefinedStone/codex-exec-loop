use super::{NativeTuiApp, SessionState, TuiLanguage};
use crate::domain::recent_sessions::{SessionCatalog, SessionCatalogStatus, SessionCatalogTier};
use crate::domain::session_browser::{SessionBrowserProjection, build_session_browser_page};
use crate::domain::session_summary::SessionSummary;

/*
 * SessionOverlayScreenModel is the immutable boundary between the mutable TUI aggregate and
 * session presentation. It owns only the current browser page and editor facts, so one frame does
 * not clone the full provider catalog or reread NativeTuiApp while building independent sections.
 */
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::adapter::inbound::tui::app) struct SessionOverlayScreenModel {
    pub(in crate::adapter::inbound::tui::app) can_open_session_list: bool,
    pub(in crate::adapter::inbound::tui::app) current_workspace_directory: String,
    pub(in crate::adapter::inbound::tui::app) language: TuiLanguage,
    pub(in crate::adapter::inbound::tui::app) committed_search_query: String,
    pub(in crate::adapter::inbound::tui::app) search_query_editor: Option<String>,
    pub(in crate::adapter::inbound::tui::app) rename_editor: Option<SessionRenameEditorScreenModel>,
    pub(in crate::adapter::inbound::tui::app) catalog: SessionOverlayCatalogScreenModel,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::adapter::inbound::tui::app) struct SessionRenameEditorScreenModel {
    pub(in crate::adapter::inbound::tui::app) thread_id: String,
    pub(in crate::adapter::inbound::tui::app) buffer: String,
    pub(in crate::adapter::inbound::tui::app) feedback: Option<String>,
    pub(in crate::adapter::inbound::tui::app) pending: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::adapter::inbound::tui::app) enum SessionOverlayCatalogScreenModel {
    Idle,
    Loading,
    Failed(String),
    Unsupported(SessionCatalogStatus),
    Partial(SessionCatalogStatus),
    Queryable(SessionBrowserScreenModel),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::adapter::inbound::tui::app) struct SessionBrowserScreenModel {
    pub(in crate::adapter::inbound::tui::app) tier: SessionCatalogTier,
    pub(in crate::adapter::inbound::tui::app) projection: SessionBrowserProjection,
    pub(in crate::adapter::inbound::tui::app) visible_sessions: Vec<SessionSummary>,
    pub(in crate::adapter::inbound::tui::app) selected_session_id: Option<String>,
    pub(in crate::adapter::inbound::tui::app) selected_index: Option<usize>,
    pub(in crate::adapter::inbound::tui::app) warnings: Vec<String>,
    pub(in crate::adapter::inbound::tui::app) next_cursor_available: bool,
}

impl SessionOverlayScreenModel {
    pub(in crate::adapter::inbound::tui::app) fn capture(app: &NativeTuiApp) -> Self {
        let current_workspace_directory = app.current_workspace_directory();
        let overlay_state = &app.session_overlay_ui_state;
        let browser_state = overlay_state.browser_state();
        let catalog = capture_catalog(
            &app.session_state,
            browser_state,
            &current_workspace_directory,
            overlay_state.selected_session_id(),
            app.selected_session_index,
        );
        let search_query_editor = overlay_state
            .is_search_query_editing()
            .then(|| overlay_state.search_query_editor_buffer().to_string());
        let rename_editor = overlay_state.rename_editor_thread_id().map(|thread_id| {
            SessionRenameEditorScreenModel {
                thread_id: thread_id.to_string(),
                buffer: overlay_state.rename_editor_buffer().to_string(),
                feedback: overlay_state.rename_editor_feedback().map(str::to_string),
                pending: overlay_state.is_rename_pending(),
            }
        });

        Self {
            can_open_session_list: app.can_open_session_list(),
            current_workspace_directory,
            language: app.tui_language,
            committed_search_query: browser_state.search_query.clone(),
            search_query_editor,
            rename_editor,
            catalog,
        }
    }

    pub(in crate::adapter::inbound::tui::app) fn browser_available(&self) -> bool {
        self.browser().is_some()
    }

    pub(in crate::adapter::inbound::tui::app) fn browser(
        &self,
    ) -> Option<&SessionBrowserScreenModel> {
        match &self.catalog {
            SessionOverlayCatalogScreenModel::Queryable(browser) => Some(browser),
            SessionOverlayCatalogScreenModel::Idle
            | SessionOverlayCatalogScreenModel::Loading
            | SessionOverlayCatalogScreenModel::Failed(_)
            | SessionOverlayCatalogScreenModel::Unsupported(_)
            | SessionOverlayCatalogScreenModel::Partial(_) => None,
        }
    }
}

impl SessionBrowserScreenModel {
    pub(in crate::adapter::inbound::tui::app) fn selected_session(
        &self,
    ) -> Option<&SessionSummary> {
        let selected_session_id = self.selected_session_id.as_deref()?;
        self.selected_index
            .and_then(|selected_index| self.visible_sessions.get(selected_index))
            .filter(|session| session.id == selected_session_id)
    }
}

fn capture_catalog(
    session_state: &SessionState,
    browser_state: &crate::domain::session_browser::SessionBrowserState,
    current_workspace_directory: &str,
    selected_session_id: Option<&str>,
    selected_session_index: usize,
) -> SessionOverlayCatalogScreenModel {
    match session_state {
        SessionState::Idle => SessionOverlayCatalogScreenModel::Idle,
        SessionState::Loading => SessionOverlayCatalogScreenModel::Loading,
        SessionState::Failed(message) => SessionOverlayCatalogScreenModel::Failed(message.clone()),
        SessionState::Ready(SessionCatalog::Unsupported(status)) => {
            SessionOverlayCatalogScreenModel::Unsupported(status.clone())
        }
        SessionState::Ready(SessionCatalog::Partial(status)) => {
            SessionOverlayCatalogScreenModel::Partial(status.clone())
        }
        SessionState::Ready(SessionCatalog::Ready {
            tier,
            recent_sessions,
        }) => {
            let browser_page = build_session_browser_page(
                recent_sessions,
                browser_state,
                Some(current_workspace_directory),
                selected_session_id,
                selected_session_index,
            );
            let selected_session_id = browser_page
                .selected_session()
                .map(|session| session.id.clone());
            SessionOverlayCatalogScreenModel::Queryable(SessionBrowserScreenModel {
                tier: *tier,
                projection: browser_page.projection,
                visible_sessions: browser_page.visible_sessions.into_iter().cloned().collect(),
                selected_session_id,
                selected_index: browser_page.selected_index,
                warnings: recent_sessions.warnings.clone(),
                next_cursor_available: recent_sessions.next_cursor.is_some(),
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::test_helpers::test_native_tui_app;
    use super::*;
    use crate::domain::recent_sessions::{RecentSessions, SessionCatalogStatus};

    #[test]
    fn capture_owns_one_projected_page_and_editor_selection_facts() {
        let mut app = test_native_tui_app();
        let workspace_directory = app.current_workspace_directory();
        app.session_state = SessionState::Ready(SessionCatalog::ready(
            SessionCatalogTier::ProviderBackedCatalog,
            RecentSessions {
                items: (0..12)
                    .map(|index| {
                        session(
                            &format!("thread-{index:02}"),
                            &format!("Task {index:02}"),
                            &workspace_directory,
                        )
                    })
                    .collect(),
                warnings: vec!["catalog warning".to_string()],
                next_cursor: Some("next-page".to_string()),
            },
        ));
        app.session_overlay_ui_state.set_search_query("task");
        app.session_overlay_ui_state.move_page(1, 2);
        app.session_overlay_ui_state
            .set_selected_session_id(Some("thread-11".to_string()));
        app.session_overlay_ui_state.start_search_query_edit();
        app.session_overlay_ui_state
            .push_search_query_character('!');
        app.session_overlay_ui_state
            .start_rename_edit("thread-11", "Renamed task");
        let rename_request = app
            .session_overlay_ui_state
            .prepare_rename_request(TuiLanguage::Korean)
            .expect("rename should become pending");
        assert!(app.session_overlay_ui_state.record_rename_admission(
            crate::core::app::SessionRenameCorrelation::new(1, rename_request),
            TuiLanguage::Korean,
        ));
        app.tui_language = TuiLanguage::Korean;

        let screen_model = SessionOverlayScreenModel::capture(&app);

        assert_eq!(
            screen_model.current_workspace_directory,
            workspace_directory
        );
        assert_eq!(screen_model.language, TuiLanguage::Korean);
        assert_eq!(screen_model.committed_search_query, "task");
        assert_eq!(screen_model.search_query_editor.as_deref(), Some("task!"));
        assert!(screen_model.browser_available());
        let browser = screen_model.browser().expect("catalog should be queryable");
        assert_eq!(browser.projection.page_index, 1);
        assert_eq!(browser.visible_sessions.len(), 2);
        assert_eq!(browser.selected_session_id.as_deref(), Some("thread-11"));
        assert_eq!(browser.selected_index, Some(1));
        assert_eq!(
            browser
                .selected_session()
                .map(|session| session.id.as_str()),
            Some("thread-11")
        );
        assert_eq!(browser.warnings, vec!["catalog warning".to_string()]);
        assert!(browser.next_cursor_available);
        let rename = screen_model
            .rename_editor
            .as_ref()
            .expect("rename editor should be captured");
        assert_eq!(rename.thread_id, "thread-11");
        assert_eq!(rename.buffer, "Renamed task");
        assert!(rename.pending);
        assert!(rename.feedback.is_some());

        let mut mismatched_selection = screen_model.clone();
        let SessionOverlayCatalogScreenModel::Queryable(browser) =
            &mut mismatched_selection.catalog
        else {
            panic!("catalog should remain queryable");
        };
        browser.selected_session_id = Some("thread-10".to_string());
        assert!(
            browser.selected_session().is_none(),
            "stable identity and page-local index must identify the same row"
        );

        app.session_overlay_ui_state
            .set_selected_session_id(Some("stale-thread".to_string()));
        app.selected_session_index = 0;
        let repaired_selection = SessionOverlayScreenModel::capture(&app);
        let repaired_browser = repaired_selection
            .browser()
            .expect("catalog should remain queryable");
        assert_eq!(
            repaired_browser.selected_session_id.as_deref(),
            Some("thread-10"),
            "the model must own the domain projection's resolved fallback id"
        );
        assert_eq!(repaired_browser.selected_index, Some(0));

        app.session_state = SessionState::Failed("new failure".to_string());
        app.session_overlay_ui_state.clear_browser_state();
        app.tui_language = TuiLanguage::English;

        assert_eq!(
            screen_model
                .browser()
                .and_then(SessionBrowserScreenModel::selected_session)
                .map(|session| session.id.as_str()),
            Some("thread-11"),
            "the fully-owned sample must not change with later app mutations"
        );
        assert_eq!(screen_model.language, TuiLanguage::Korean);
        assert_eq!(screen_model.committed_search_query, "task");
    }

    #[test]
    fn capture_preserves_non_queryable_catalog_variants_without_loss() {
        let mut app = test_native_tui_app();
        let status = SessionCatalogStatus {
            tier: SessionCatalogTier::AttachOnly,
            detail: "provider catalog unavailable".to_string(),
            warnings: vec!["manual attach only".to_string()],
        };

        app.session_state = SessionState::Ready(SessionCatalog::Unsupported(status.clone()));
        assert_eq!(
            SessionOverlayScreenModel::capture(&app).catalog,
            SessionOverlayCatalogScreenModel::Unsupported(status.clone())
        );

        app.session_state = SessionState::Ready(SessionCatalog::Partial(status.clone()));
        assert_eq!(
            SessionOverlayScreenModel::capture(&app).catalog,
            SessionOverlayCatalogScreenModel::Partial(status)
        );

        app.session_state = SessionState::Failed("catalog failed".to_string());
        assert_eq!(
            SessionOverlayScreenModel::capture(&app).catalog,
            SessionOverlayCatalogScreenModel::Failed("catalog failed".to_string())
        );
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
            git_branch: Some("feature/session-screen-model".to_string()),
        }
    }
}
