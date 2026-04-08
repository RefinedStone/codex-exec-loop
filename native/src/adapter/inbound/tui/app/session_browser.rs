use crate::application::service::session_service::{
    SessionBrowserProjection, SessionBrowserState, project_recent_sessions,
};
use crate::domain::recent_sessions::RecentSessions;
use crate::domain::session_summary::SessionSummary;

pub(super) struct SessionBrowserView<'a> {
    pub projection: SessionBrowserProjection,
    pub visible_sessions: Vec<&'a SessionSummary>,
    pub selected_index: Option<usize>,
}

impl<'a> SessionBrowserView<'a> {
    pub fn selected_session(&self) -> Option<&'a SessionSummary> {
        self.selected_index
            .and_then(|selected_index| self.visible_sessions.get(selected_index).copied())
    }
}

pub(super) fn build_session_browser_view<'a>(
    recent_sessions: &'a RecentSessions,
    browser_state: &SessionBrowserState,
    selected_session_index: usize,
) -> SessionBrowserView<'a> {
    let projection = project_recent_sessions(recent_sessions, browser_state);
    let visible_sessions = projection
        .page_session_indexes
        .iter()
        .filter_map(|session_index| recent_sessions.items.get(*session_index))
        .collect::<Vec<_>>();
    let selected_index = projection.clamp_selected_index(selected_session_index);

    SessionBrowserView {
        projection,
        visible_sessions,
        selected_index,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::service::session_service::{SessionBrowserState, SessionProjectFilter};

    #[test]
    fn browser_view_clamps_selection_to_visible_page() {
        let recent_sessions = RecentSessions {
            items: vec![
                sample_session("thread-1", "/tmp/root-a", "alpha"),
                sample_session("thread-2", "/tmp/root-a", "beta"),
                sample_session("thread-3", "/tmp/root-b", "gamma"),
            ],
            warnings: Vec::new(),
            next_cursor: None,
        };
        let browser_state = SessionBrowserState {
            search_query: String::new(),
            page_index: 1,
            page_size: 2,
            project_filter: SessionProjectFilter::AllProjects,
        };

        let browser_view = build_session_browser_view(&recent_sessions, &browser_state, 5);

        assert_eq!(browser_view.selected_index, Some(0));
        assert_eq!(
            browser_view
                .selected_session()
                .map(|session| session.id.as_str()),
            Some("thread-3")
        );
    }

    fn sample_session(id: &str, cwd: &str, preview: &str) -> SessionSummary {
        SessionSummary {
            id: id.to_string(),
            name: Some(id.to_string()),
            preview: preview.to_string(),
            cwd: cwd.to_string(),
            source: "codex".to_string(),
            model_provider: "openai".to_string(),
            updated_at_epoch: 1_700_000_000,
            status_type: "ready".to_string(),
            path: format!("{cwd}/{id}.json"),
            git_branch: Some("main".to_string()),
        }
    }
}
