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

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct SessionBrowserSelection {
    pub index: usize,
    pub session_id: Option<String>,
}

impl<'a> SessionBrowserView<'a> {
    pub fn selected_session(&self) -> Option<&'a SessionSummary> {
        self.selected_index
            .and_then(|selected_index| self.visible_sessions.get(selected_index).copied())
    }

    pub fn first_selection(&self) -> SessionBrowserSelection {
        self.selection_at_index(0)
    }

    pub fn last_selection(&self) -> SessionBrowserSelection {
        self.selection_at_index(self.visible_sessions.len().saturating_sub(1))
    }

    pub fn selection_after_delta(&self, delta: isize) -> SessionBrowserSelection {
        if self.visible_sessions.is_empty() {
            return SessionBrowserSelection {
                index: 0,
                session_id: None,
            };
        }

        let current_index = self.selected_index.unwrap_or(0) as isize;
        let max_index = self.visible_sessions.len().saturating_sub(1) as isize;
        let next_index = (current_index + delta).clamp(0, max_index) as usize;

        SessionBrowserSelection {
            index: next_index,
            session_id: self
                .visible_sessions
                .get(next_index)
                .map(|session| session.id.clone()),
        }
    }

    fn selection_at_index(&self, index: usize) -> SessionBrowserSelection {
        if self.visible_sessions.is_empty() {
            return SessionBrowserSelection {
                index: 0,
                session_id: None,
            };
        }

        let next_index = index.min(self.visible_sessions.len().saturating_sub(1));
        SessionBrowserSelection {
            index: next_index,
            session_id: self
                .visible_sessions
                .get(next_index)
                .map(|session| session.id.clone()),
        }
    }
}

pub(super) fn build_session_browser_view<'a>(
    recent_sessions: &'a RecentSessions,
    browser_state: &SessionBrowserState,
    current_workspace_directory: Option<&str>,
    selected_session_id: Option<&str>,
    selected_session_index: usize,
) -> SessionBrowserView<'a> {
    let projection =
        project_recent_sessions(recent_sessions, browser_state, current_workspace_directory);
    let visible_sessions = projection
        .page_session_indexes
        .iter()
        .filter_map(|session_index| recent_sessions.items.get(*session_index))
        .collect::<Vec<_>>();
    let selected_index = resolve_selected_index(
        &visible_sessions,
        selected_session_id,
        selected_session_index,
    );

    SessionBrowserView {
        projection,
        visible_sessions,
        selected_index,
    }
}

fn resolve_selected_index(
    visible_sessions: &[&SessionSummary],
    selected_session_id: Option<&str>,
    selected_session_index: usize,
) -> Option<usize> {
    if let Some(selected_session_id) = selected_session_id {
        if let Some(selected_index) = visible_sessions
            .iter()
            .position(|session| session.id == selected_session_id)
        {
            return Some(selected_index);
        }
    }

    (!visible_sessions.is_empty())
        .then(|| selected_session_index.min(visible_sessions.len().saturating_sub(1)))
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

        let browser_view =
            build_session_browser_view(&recent_sessions, &browser_state, None, None, 5);

        assert_eq!(browser_view.selected_index, Some(0));
        assert_eq!(
            browser_view
                .selected_session()
                .map(|session| session.id.as_str()),
            Some("thread-3")
        );
    }

    #[test]
    fn browser_view_preserves_selected_session_by_id_after_filtering() {
        let recent_sessions = RecentSessions {
            items: vec![
                sample_session("thread-1", "/tmp/root-a", "alpha"),
                sample_session("thread-2", "/tmp/root-a", "beta"),
                sample_session("thread-3", "/tmp/root-b", "docs release"),
            ],
            warnings: Vec::new(),
            next_cursor: None,
        };
        let browser_state = SessionBrowserState {
            search_query: "docs".to_string(),
            page_index: 0,
            page_size: 10,
            project_filter: SessionProjectFilter::AllProjects,
        };

        let browser_view =
            build_session_browser_view(&recent_sessions, &browser_state, None, Some("thread-3"), 1);

        assert_eq!(browser_view.selected_index, Some(0));
        assert_eq!(
            browser_view
                .selected_session()
                .map(|session| session.id.as_str()),
            Some("thread-3")
        );
    }

    #[test]
    fn browser_view_selection_after_delta_clamps_and_preserves_session_id() {
        let recent_sessions = RecentSessions {
            items: vec![
                sample_session("thread-1", "/tmp/root-a", "alpha"),
                sample_session("thread-2", "/tmp/root-a", "beta"),
                sample_session("thread-3", "/tmp/root-b", "gamma"),
            ],
            warnings: Vec::new(),
            next_cursor: None,
        };
        let browser_state = SessionBrowserState::default();
        let browser_view =
            build_session_browser_view(&recent_sessions, &browser_state, None, Some("thread-2"), 0);

        let selection = browser_view.selection_after_delta(5);

        assert_eq!(
            selection,
            SessionBrowserSelection {
                index: 2,
                session_id: Some("thread-3".to_string()),
            }
        );
    }

    #[test]
    fn browser_view_last_selection_returns_last_visible_session() {
        let recent_sessions = RecentSessions {
            items: vec![
                sample_session("thread-1", "/tmp/root-a", "alpha"),
                sample_session("thread-2", "/tmp/root-a", "beta"),
                sample_session("thread-3", "/tmp/root-b", "gamma"),
            ],
            warnings: Vec::new(),
            next_cursor: None,
        };
        let browser_state = SessionBrowserState::default();
        let browser_view =
            build_session_browser_view(&recent_sessions, &browser_state, None, None, 0);

        let selection = browser_view.last_selection();

        assert_eq!(
            selection,
            SessionBrowserSelection {
                index: 2,
                session_id: Some("thread-3".to_string()),
            }
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
