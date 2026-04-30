use std::collections::HashMap;

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

    pub fn jump_to_first_page(&mut self) {
        self.page_index = 0;
    }

    pub fn jump_to_last_page(&mut self, total_pages: usize) {
        self.page_index = total_pages.saturating_sub(1);
    }

    pub fn clear(&mut self) {
        self.search_query.clear();
        self.page_index = 0;
        self.project_filter = SessionProjectFilter::AllProjects;
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
    pub session_count: usize,
    pub is_current_workspace: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionBrowserProjection {
    pub active_project_filter: SessionProjectFilter,
    pub project_filter_options: Vec<SessionProjectFilterOption>,
    pub current_workspace_session_count: usize,
    pub total_session_count: usize,
    pub project_filtered_session_count: usize,
    pub filtered_session_count: usize,
    pub page_index: usize,
    pub total_pages: usize,
    pub visible_session_range: Option<(usize, usize)>,
    pub page_session_indexes: Vec<usize>,
}

impl SessionBrowserProjection {
    pub fn clamp_selected_index(&self, selected_session_index: usize) -> Option<usize> {
        (!self.page_session_indexes.is_empty())
            .then(|| selected_session_index.min(self.page_session_indexes.len().saturating_sub(1)))
    }

    pub fn cycled_project_filter(&self, delta: isize) -> Option<SessionProjectFilter> {
        let option_count = self.project_filter_options.len() as isize;
        if option_count == 0 {
            return None;
        }

        let current_index = self
            .project_filter_options
            .iter()
            .position(|option| option.filter == self.active_project_filter)
            .unwrap_or(0) as isize;
        let next_index = (current_index + delta).rem_euclid(option_count) as usize;

        self.project_filter_options
            .get(next_index)
            .map(|option| option.filter.clone())
    }

    pub fn active_project_filter_option(&self) -> Option<&SessionProjectFilterOption> {
        self.project_filter_options
            .iter()
            .find(|option| option.filter == self.active_project_filter)
    }
}

pub struct SessionBrowserPage<'a> {
    pub projection: SessionBrowserProjection,
    pub visible_sessions: Vec<&'a SessionSummary>,
    pub selected_index: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionBrowserSelection {
    pub index: usize,
    pub session_id: Option<String>,
}

impl<'a> SessionBrowserPage<'a> {
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

pub fn project_recent_sessions(
    recent_sessions: &RecentSessions,
    browser_state: &SessionBrowserState,
    current_workspace_directory: Option<&str>,
) -> SessionBrowserProjection {
    let search_tokens = tokenize_search_query(&browser_state.search_query);
    let project_filter_options =
        build_project_filter_options(&recent_sessions.items, current_workspace_directory);
    let current_workspace_session_count = current_workspace_directory
        .map(|workspace_directory| {
            recent_sessions
                .items
                .iter()
                .filter(|session| session.cwd == workspace_directory)
                .count()
        })
        .unwrap_or(0);
    let active_project_filter =
        resolve_active_project_filter(&browser_state.project_filter, &project_filter_options);
    let total_session_count = recent_sessions.items.len();
    let project_filtered_sessions = recent_sessions
        .items
        .iter()
        .enumerate()
        .filter(|(_, session)| matches_project_filter(session, &active_project_filter))
        .collect::<Vec<_>>();
    let project_filtered_session_count = project_filtered_sessions.len();
    let mut ranked_sessions = project_filtered_sessions
        .into_iter()
        .filter_map(|(index, session)| {
            search_query_score(session, &search_tokens, current_workspace_directory).map(|score| {
                RankedSessionIndex {
                    index,
                    updated_at_epoch: session.updated_at_epoch,
                    score,
                }
            })
        })
        .collect::<Vec<_>>();

    if !search_tokens.is_empty() {
        ranked_sessions.sort_by(|left, right| {
            right
                .score
                .cmp(&left.score)
                .then_with(|| right.updated_at_epoch.cmp(&left.updated_at_epoch))
                .then_with(|| left.index.cmp(&right.index))
        });
    }

    let filtered_session_count = ranked_sessions.len();
    let total_pages = if filtered_session_count == 0 {
        0
    } else {
        filtered_session_count.div_ceil(browser_state.page_size)
    };
    let page_index = if total_pages == 0 {
        0
    } else {
        browser_state.page_index.min(total_pages.saturating_sub(1))
    };
    let page_start = page_index.saturating_mul(browser_state.page_size);
    let page_session_indexes = ranked_sessions
        .iter()
        .skip(page_start)
        .take(browser_state.page_size)
        .map(|ranked_session| ranked_session.index)
        .collect::<Vec<_>>();
    let visible_session_range = (!page_session_indexes.is_empty())
        .then_some((page_start + 1, page_start + page_session_indexes.len()));

    SessionBrowserProjection {
        active_project_filter,
        project_filter_options,
        current_workspace_session_count,
        total_session_count,
        project_filtered_session_count,
        filtered_session_count,
        page_index,
        total_pages,
        visible_session_range,
        page_session_indexes,
    }
}

pub fn build_session_browser_page<'a>(
    recent_sessions: &'a RecentSessions,
    browser_state: &SessionBrowserState,
    current_workspace_directory: Option<&str>,
    selected_session_id: Option<&str>,
    selected_session_index: usize,
) -> SessionBrowserPage<'a> {
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

    SessionBrowserPage {
        projection,
        visible_sessions,
        selected_index,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct RankedSessionIndex {
    index: usize,
    updated_at_epoch: i64,
    score: u32,
}

fn build_project_filter_options(
    sessions: &[SessionSummary],
    current_workspace_directory: Option<&str>,
) -> Vec<SessionProjectFilterOption> {
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
        session_count: sessions.len(),
        is_current_workspace: false,
    }];

    for workspace_directory in workspace_order {
        let is_current_workspace =
            current_workspace_directory.is_some_and(|current| current == workspace_directory);
        project_filter_options.push(SessionProjectFilterOption {
            filter: SessionProjectFilter::RecentProject {
                workspace_directory: workspace_directory.to_string(),
            },
            session_count: *workspace_counts
                .get(workspace_directory)
                .expect("workspace count should exist"),
            is_current_workspace,
        });
    }

    project_filter_options
}

fn resolve_selected_index(
    visible_sessions: &[&SessionSummary],
    selected_session_id: Option<&str>,
    selected_session_index: usize,
) -> Option<usize> {
    if let Some(selected_session_id) = selected_session_id
        && let Some(selected_index) = visible_sessions
            .iter()
            .position(|session| session.id == selected_session_id)
    {
        return Some(selected_index);
    }

    (!visible_sessions.is_empty())
        .then(|| selected_session_index.min(visible_sessions.len().saturating_sub(1)))
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
        .map(|token| token.trim().to_lowercase())
        .filter(|token| !token.is_empty())
        .collect()
}

fn search_query_score(
    session: &SessionSummary,
    search_tokens: &[String],
    current_workspace_directory: Option<&str>,
) -> Option<u32> {
    let mut score = current_workspace_bonus(session, current_workspace_directory);
    for search_token in search_tokens {
        score += search_token_score(session, search_token)?;
    }

    Some(score)
}

fn current_workspace_bonus(
    session: &SessionSummary,
    current_workspace_directory: Option<&str>,
) -> u32 {
    if current_workspace_directory
        .is_some_and(|workspace_directory| session.cwd == workspace_directory)
    {
        4
    } else {
        0
    }
}

fn search_token_score(session: &SessionSummary, search_token: &str) -> Option<u32> {
    [
        score_search_field(&session.id, search_token, 220, 200, 140),
        score_search_field(&session.preview, search_token, 90, 80, 60),
        score_search_field(&session.cwd, search_token, 150, 135, 100),
        score_search_field(&session.path, search_token, 130, 115, 90),
        session
            .name
            .as_deref()
            .and_then(|name| score_search_field(name, search_token, 210, 190, 130)),
        session
            .git_branch
            .as_deref()
            .and_then(|branch| score_search_field(branch, search_token, 160, 145, 110)),
    ]
    .into_iter()
    .flatten()
    .max()
}

fn score_search_field(
    haystack: &str,
    needle: &str,
    exact_score: u32,
    prefix_score: u32,
    contains_score: u32,
) -> Option<u32> {
    if haystack.eq_ignore_ascii_case(needle) {
        return Some(exact_score);
    }

    if starts_with_ascii_case_insensitive(haystack, needle) {
        return Some(prefix_score);
    }

    contains_ascii_case_insensitive(haystack, needle).then_some(contains_score)
}

fn starts_with_ascii_case_insensitive(haystack: &str, needle: &str) -> bool {
    if needle.is_empty() {
        return true;
    }

    let haystack_bytes = haystack.as_bytes();
    let needle_bytes = needle.as_bytes();
    haystack_bytes
        .get(..needle_bytes.len())
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case(needle_bytes))
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
    use super::*;

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
    fn clear_resets_query_filter_and_page_index() {
        let mut state = SessionBrowserState::new(5);
        state.set_search_query("docs");
        state.set_project_filter(SessionProjectFilter::RecentProject {
            workspace_directory: "/tmp/root".to_string(),
        });
        state.page_index = 3;

        state.clear();

        assert!(state.search_query.is_empty());
        assert_eq!(state.page_index, 0);
        assert_eq!(state.project_filter, SessionProjectFilter::AllProjects);
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

        let projection = project_recent_sessions(&recent_sessions, &browser_state, None);

        assert_eq!(projection.total_session_count, 3);
        assert_eq!(projection.project_filtered_session_count, 1);
        assert_eq!(projection.filtered_session_count, 1);
        assert_eq!(projection.total_pages, 1);
        assert_eq!(projection.visible_session_range, Some((1, 1)));
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

        let projection = project_recent_sessions(&recent_sessions, &browser_state, None);

        assert_eq!(
            projection.active_project_filter,
            SessionProjectFilter::AllProjects
        );
        assert_eq!(projection.total_session_count, 3);
        assert_eq!(projection.project_filtered_session_count, 3);
        assert_eq!(projection.total_pages, 2);
        assert_eq!(projection.page_index, 1);
        assert_eq!(projection.visible_session_range, Some((3, 3)));
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

        let projection = project_recent_sessions(&recent_sessions, &browser_state, None);

        assert_eq!(projection.page_session_indexes, vec![0]);
    }

    #[test]
    fn project_recent_sessions_ranks_name_and_branch_hits_ahead_of_preview_only_matches() {
        let recent_sessions = RecentSessions {
            items: vec![
                sample_named_session(
                    "thread-preview",
                    "/tmp/root-a",
                    "release notes hidden in preview",
                    None,
                    Some("main"),
                    1_700_000_300,
                ),
                sample_named_session(
                    "thread-name",
                    "/tmp/root-b",
                    "maintenance",
                    Some("release prep"),
                    Some("main"),
                    1_700_000_100,
                ),
                sample_named_session(
                    "thread-branch",
                    "/tmp/root-c",
                    "maintenance",
                    None,
                    Some("release/final"),
                    1_700_000_200,
                ),
            ],
            warnings: Vec::new(),
            next_cursor: None,
        };
        let mut browser_state = SessionBrowserState::new(10);
        browser_state.set_search_query("release");

        let projection = project_recent_sessions(&recent_sessions, &browser_state, None);

        assert_eq!(
            projection.page_session_indexes,
            vec![1, 2, 0],
            "name hits should outrank branch hits, and branch hits should outrank preview-only hits"
        );
    }

    #[test]
    fn project_recent_sessions_reports_visible_match_range_for_ranked_results() {
        let recent_sessions = RecentSessions {
            items: vec![
                sample_named_session(
                    "thread-1",
                    "/tmp/root-a",
                    "docs checklist",
                    Some("alpha"),
                    Some("main"),
                    1_700_000_000,
                ),
                sample_named_session(
                    "thread-2",
                    "/tmp/root-a",
                    "release prep",
                    Some("docs launch"),
                    Some("main"),
                    1_699_999_900,
                ),
                sample_named_session(
                    "thread-3",
                    "/tmp/root-a",
                    "docs rollout",
                    Some("zeta"),
                    Some("main"),
                    1_700_000_100,
                ),
            ],
            warnings: Vec::new(),
            next_cursor: None,
        };
        let mut browser_state = SessionBrowserState::new(10);
        browser_state.set_search_query("docs");

        let projection = project_recent_sessions(&recent_sessions, &browser_state, None);

        assert_eq!(projection.page_session_indexes, vec![1, 2, 0]);
        assert_eq!(projection.visible_session_range, Some((1, 3)));
    }

    #[test]
    fn project_recent_sessions_marks_current_workspace_filter_context() {
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

        let projection =
            project_recent_sessions(&recent_sessions, &browser_state, Some("/tmp/root-b"));

        assert_eq!(projection.current_workspace_session_count, 1);
        assert_eq!(
            projection
                .project_filter_options
                .iter()
                .find(|option| option.is_current_workspace)
                .map(|option| &option.filter),
            Some(&SessionProjectFilter::RecentProject {
                workspace_directory: "/tmp/root-b".to_string()
            })
        );
    }

    #[test]
    fn cycled_project_filter_wraps_across_available_options() {
        let projection = SessionBrowserProjection {
            active_project_filter: SessionProjectFilter::RecentProject {
                workspace_directory: "/tmp/root-b".to_string(),
            },
            project_filter_options: vec![
                SessionProjectFilterOption {
                    filter: SessionProjectFilter::AllProjects,
                    session_count: 3,
                    is_current_workspace: false,
                },
                SessionProjectFilterOption {
                    filter: SessionProjectFilter::RecentProject {
                        workspace_directory: "/tmp/root-a".to_string(),
                    },
                    session_count: 2,
                    is_current_workspace: false,
                },
                SessionProjectFilterOption {
                    filter: SessionProjectFilter::RecentProject {
                        workspace_directory: "/tmp/root-b".to_string(),
                    },
                    session_count: 1,
                    is_current_workspace: true,
                },
            ],
            current_workspace_session_count: 1,
            total_session_count: 3,
            project_filtered_session_count: 1,
            filtered_session_count: 1,
            page_index: 0,
            total_pages: 1,
            visible_session_range: Some((1, 1)),
            page_session_indexes: vec![2],
        };

        assert_eq!(
            projection.cycled_project_filter(1),
            Some(SessionProjectFilter::AllProjects)
        );
        assert_eq!(
            projection.cycled_project_filter(-1),
            Some(SessionProjectFilter::RecentProject {
                workspace_directory: "/tmp/root-a".to_string(),
            })
        );
    }

    #[test]
    fn browser_page_clamps_selection_to_visible_page() {
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

        let browser_page =
            build_session_browser_page(&recent_sessions, &browser_state, None, None, 5);

        assert_eq!(browser_page.selected_index, Some(0));
        assert_eq!(
            browser_page
                .selected_session()
                .map(|session| session.id.as_str()),
            Some("thread-3")
        );
    }

    #[test]
    fn browser_page_preserves_selected_session_by_id_after_filtering() {
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

        let browser_page =
            build_session_browser_page(&recent_sessions, &browser_state, None, Some("thread-3"), 1);

        assert_eq!(browser_page.selected_index, Some(0));
        assert_eq!(
            browser_page
                .selected_session()
                .map(|session| session.id.as_str()),
            Some("thread-3")
        );
    }

    #[test]
    fn browser_page_selection_after_delta_clamps_and_preserves_session_id() {
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
        let browser_page =
            build_session_browser_page(&recent_sessions, &browser_state, None, Some("thread-2"), 0);

        let selection = browser_page.selection_after_delta(5);

        assert_eq!(
            selection,
            SessionBrowserSelection {
                index: 2,
                session_id: Some("thread-3".to_string()),
            }
        );
    }

    #[test]
    fn browser_page_last_selection_returns_last_visible_session() {
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
        let browser_page =
            build_session_browser_page(&recent_sessions, &browser_state, None, None, 0);

        let selection = browser_page.last_selection();

        assert_eq!(
            selection,
            SessionBrowserSelection {
                index: 2,
                session_id: Some("thread-3".to_string()),
            }
        );
    }

    fn sample_session(id: &str, cwd: &str, preview: &str) -> SessionSummary {
        sample_named_session(id, cwd, preview, Some(id), Some("main"), 1_700_000_000)
    }

    fn sample_named_session(
        id: &str,
        cwd: &str,
        preview: &str,
        name: Option<&str>,
        git_branch: Option<&str>,
        updated_at_epoch: i64,
    ) -> SessionSummary {
        SessionSummary {
            id: id.to_string(),
            name: name.map(str::to_string),
            preview: preview.to_string(),
            cwd: cwd.to_string(),
            source: "codex".to_string(),
            model_provider: "openai".to_string(),
            updated_at_epoch,
            status_type: "ready".to_string(),
            path: format!("{cwd}/{id}.json"),
            git_branch: git_branch.map(str::to_string),
        }
    }
}
