use std::collections::HashMap;
use std::sync::Arc;

use anyhow::Result;

use crate::application::port::outbound::codex_app_server_port::CodexAppServerPort;
use crate::domain::recent_sessions::RecentSessions;
use crate::domain::session_summary::SessionSummary;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionBrowserState {
    pub search_query: String,
    pub page_index: usize,
    pub page_size: usize,
    pub project_filter: SessionProjectFilter,
}

impl SessionBrowserState {
    pub fn new(page_size: usize) -> Self {
        Self {
            search_query: String::new(),
            page_index: 0,
            page_size: page_size.max(1),
            project_filter: SessionProjectFilter::AllProjects,
        }
    }

    pub fn set_search_query(&mut self, search_query: impl Into<String>) {
        let normalized_query = search_query.into().trim().to_string();
        if self.search_query == normalized_query {
            return;
        }

        self.search_query = normalized_query;
        self.page_index = 0;
    }

    pub fn set_project_filter(&mut self, project_filter: SessionProjectFilter) {
        if self.project_filter == project_filter {
            return;
        }

        self.project_filter = project_filter;
        self.page_index = 0;
    }

    pub fn move_page(&mut self, delta: isize, total_pages: usize) {
        if total_pages == 0 {
            self.page_index = 0;
            return;
        }

        let max_page_index = total_pages.saturating_sub(1) as isize;
        let next_page_index = (self.page_index as isize + delta).clamp(0, max_page_index);
        self.page_index = next_page_index as usize;
    }
}

impl Default for SessionBrowserState {
    fn default() -> Self {
        Self::new(10)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionProjectFilter {
    AllProjects,
    RecentProject { workspace_directory: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionProjectFilterOption {
    pub filter: SessionProjectFilter,
    pub label: String,
    pub session_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionBrowserProjection {
    pub active_project_filter: SessionProjectFilter,
    pub project_filter_options: Vec<SessionProjectFilterOption>,
    pub filtered_session_count: usize,
    pub page_index: usize,
    pub total_pages: usize,
    pub page_session_indexes: Vec<usize>,
}

impl SessionBrowserProjection {
    pub fn clamp_selected_index(&self, selected_session_index: usize) -> Option<usize> {
        (!self.page_session_indexes.is_empty())
            .then(|| selected_session_index.min(self.page_session_indexes.len().saturating_sub(1)))
    }
}

#[derive(Clone)]
pub struct SessionService {
    codex_app_server_port: Arc<dyn CodexAppServerPort>,
}

impl SessionService {
    pub fn new(codex_app_server_port: Arc<dyn CodexAppServerPort>) -> Self {
        Self {
            codex_app_server_port,
        }
    }

    pub fn load_recent_sessions(&self, limit: usize) -> Result<RecentSessions> {
        self.codex_app_server_port.load_recent_sessions(limit)
    }
}

pub fn project_recent_sessions(
    recent_sessions: &RecentSessions,
    browser_state: &SessionBrowserState,
) -> SessionBrowserProjection {
    let search_tokens = tokenize_search_query(&browser_state.search_query);
    let project_filter_options = build_project_filter_options(&recent_sessions.items);
    let active_project_filter =
        resolve_active_project_filter(&browser_state.project_filter, &project_filter_options);
    let matching_indexes = recent_sessions
        .items
        .iter()
        .enumerate()
        .filter(|(_, session)| matches_project_filter(session, &active_project_filter))
        .filter(|(_, session)| matches_search_query(session, &search_tokens))
        .map(|(index, _)| index)
        .collect::<Vec<_>>();

    let filtered_session_count = matching_indexes.len();
    let total_pages = if filtered_session_count == 0 {
        0
    } else {
        (filtered_session_count + browser_state.page_size - 1) / browser_state.page_size
    };
    let page_index = if total_pages == 0 {
        0
    } else {
        browser_state.page_index.min(total_pages.saturating_sub(1))
    };
    let page_start = page_index.saturating_mul(browser_state.page_size);
    let page_session_indexes = matching_indexes
        .into_iter()
        .skip(page_start)
        .take(browser_state.page_size)
        .collect::<Vec<_>>();

    SessionBrowserProjection {
        active_project_filter,
        project_filter_options,
        filtered_session_count,
        page_index,
        total_pages,
        page_session_indexes,
    }
}

fn build_project_filter_options(sessions: &[SessionSummary]) -> Vec<SessionProjectFilterOption> {
    let mut workspace_counts = HashMap::new();
    let mut workspace_order = Vec::new();

    for session in sessions {
        let workspace_directory = session.cwd.as_str();
        let count = workspace_counts.entry(workspace_directory).or_insert(0);
        if *count == 0 {
            workspace_order.push(workspace_directory);
        }
        *count += 1;
    }

    let mut project_filter_options = vec![SessionProjectFilterOption {
        filter: SessionProjectFilter::AllProjects,
        label: "all projects".to_string(),
        session_count: sessions.len(),
    }];

    for workspace_directory in workspace_order {
        project_filter_options.push(SessionProjectFilterOption {
            filter: SessionProjectFilter::RecentProject {
                workspace_directory: workspace_directory.to_string(),
            },
            label: workspace_directory.to_string(),
            session_count: *workspace_counts
                .get(workspace_directory)
                .expect("workspace count should exist"),
        });
    }

    project_filter_options
}

fn resolve_active_project_filter(
    project_filter: &SessionProjectFilter,
    project_filter_options: &[SessionProjectFilterOption],
) -> SessionProjectFilter {
    if project_filter_options
        .iter()
        .any(|option| option.filter == *project_filter)
    {
        return project_filter.clone();
    }

    SessionProjectFilter::AllProjects
}

fn matches_project_filter(session: &SessionSummary, project_filter: &SessionProjectFilter) -> bool {
    match project_filter {
        SessionProjectFilter::AllProjects => true,
        SessionProjectFilter::RecentProject {
            workspace_directory,
        } => session.cwd == *workspace_directory,
    }
}

fn tokenize_search_query(search_query: &str) -> Vec<String> {
    search_query
        .split_whitespace()
        .map(|token| token.trim().to_ascii_lowercase())
        .filter(|token| !token.is_empty())
        .collect()
}

fn matches_search_query(session: &SessionSummary, search_tokens: &[String]) -> bool {
    if search_tokens.is_empty() {
        return true;
    }

    search_tokens
        .iter()
        .all(|search_token| matches_search_token(session, search_token))
}

fn matches_search_token(session: &SessionSummary, search_token: &str) -> bool {
    contains_ascii_case_insensitive(&session.id, search_token)
        || contains_ascii_case_insensitive(&session.preview, search_token)
        || contains_ascii_case_insensitive(&session.cwd, search_token)
        || contains_ascii_case_insensitive(&session.path, search_token)
        || session
            .name
            .as_deref()
            .is_some_and(|name| contains_ascii_case_insensitive(name, search_token))
        || session
            .git_branch
            .as_deref()
            .is_some_and(|branch| contains_ascii_case_insensitive(branch, search_token))
}

fn contains_ascii_case_insensitive(haystack: &str, needle: &str) -> bool {
    if needle.is_empty() {
        return true;
    }

    let haystack_bytes = haystack.as_bytes();
    let needle_bytes = needle.as_bytes();
    if needle_bytes.len() > haystack_bytes.len() {
        return false;
    }

    haystack_bytes
        .windows(needle_bytes.len())
        .any(|window| window.eq_ignore_ascii_case(needle_bytes))
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;
    use crate::application::port::outbound::codex_app_server_port::AppServerStartupContext;
    use crate::domain::conversation::{ConversationSnapshot, ConversationStreamEvent};

    #[derive(Default)]
    struct FakeCodexAppServerPort {
        limits: Mutex<Vec<usize>>,
    }

    impl CodexAppServerPort for FakeCodexAppServerPort {
        fn load_startup_context(&self) -> Result<AppServerStartupContext> {
            unreachable!("startup context is not used in session service tests")
        }

        fn load_recent_sessions(&self, limit: usize) -> Result<RecentSessions> {
            self.limits
                .lock()
                .expect("session limit mutex poisoned")
                .push(limit);
            Ok(RecentSessions {
                items: Vec::new(),
                warnings: Vec::new(),
                next_cursor: None,
            })
        }

        fn load_conversation_snapshot(&self, _thread_id: &str) -> Result<ConversationSnapshot> {
            unreachable!("conversation snapshots are not used in session service tests")
        }

        fn run_new_thread_stream(
            &self,
            _cwd: &str,
            _prompt: &str,
            _event_sender: std::sync::mpsc::Sender<ConversationStreamEvent>,
        ) -> Result<()> {
            unreachable!("new-thread streaming is not used in session service tests")
        }

        fn run_turn_stream(
            &self,
            _thread_id: &str,
            _prompt: &str,
            _event_sender: std::sync::mpsc::Sender<ConversationStreamEvent>,
        ) -> Result<()> {
            unreachable!("turn streaming is not used in session service tests")
        }
    }

    #[test]
    fn load_recent_sessions_delegates_requested_limit() {
        let port = Arc::new(FakeCodexAppServerPort::default());
        let service = SessionService::new(port.clone());

        service
            .load_recent_sessions(25)
            .expect("load recent sessions should succeed");

        assert_eq!(
            *port.limits.lock().expect("session limit mutex poisoned"),
            vec![25]
        );
    }

    #[test]
    fn search_query_resets_page_index() {
        let mut state = SessionBrowserState::new(5);
        state.page_index = 2;

        state.set_search_query("bugfix");

        assert_eq!(state.search_query, "bugfix");
        assert_eq!(state.page_index, 0);
    }

    #[test]
    fn move_page_clamps_to_available_range() {
        let mut state = SessionBrowserState::new(5);

        state.move_page(3, 2);
        assert_eq!(state.page_index, 1);

        state.move_page(-9, 2);
        assert_eq!(state.page_index, 0);
    }

    #[test]
    fn project_recent_sessions_filters_by_query_and_project() {
        let recent_sessions = RecentSessions {
            items: vec![
                sample_session("thread-1", "/tmp/root-a", "bugfix queue"),
                sample_session("thread-2", "/tmp/root-a", "docs refresh"),
                sample_session("thread-3", "/tmp/root-b", "bugfix release"),
            ],
            warnings: Vec::new(),
            next_cursor: None,
        };
        let mut browser_state = SessionBrowserState::new(2);
        browser_state.set_search_query("bugfix");
        browser_state.set_project_filter(SessionProjectFilter::RecentProject {
            workspace_directory: "/tmp/root-b".to_string(),
        });

        let projection = project_recent_sessions(&recent_sessions, &browser_state);

        assert_eq!(projection.filtered_session_count, 1);
        assert_eq!(projection.total_pages, 1);
        assert_eq!(projection.page_session_indexes, vec![2]);
        assert_eq!(
            projection.active_project_filter,
            SessionProjectFilter::RecentProject {
                workspace_directory: "/tmp/root-b".to_string(),
            }
        );
    }

    #[test]
    fn project_recent_sessions_clamps_stale_page_and_filter_state() {
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
            page_index: 5,
            page_size: 2,
            project_filter: SessionProjectFilter::RecentProject {
                workspace_directory: "/tmp/missing".to_string(),
            },
        };

        let projection = project_recent_sessions(&recent_sessions, &browser_state);

        assert_eq!(
            projection.active_project_filter,
            SessionProjectFilter::AllProjects
        );
        assert_eq!(projection.total_pages, 2);
        assert_eq!(projection.page_index, 1);
        assert_eq!(projection.page_session_indexes, vec![2]);
    }

    #[test]
    fn project_recent_sessions_matches_query_without_allocating_title_haystacks() {
        let recent_sessions = RecentSessions {
            items: vec![
                sample_session("thread-1", "/tmp/root-a", "Docs release prep"),
                sample_session("thread-2", "/tmp/root-b", "bugfix queue"),
            ],
            warnings: Vec::new(),
            next_cursor: None,
        };
        let mut browser_state = SessionBrowserState::new(10);
        browser_state.set_search_query("docs release");

        let projection = project_recent_sessions(&recent_sessions, &browser_state);

        assert_eq!(projection.page_session_indexes, vec![0]);
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
