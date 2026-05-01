use std::collections::HashMap;

use crate::domain::recent_sessions::RecentSessions;
use crate::domain::session_summary::SessionSummary;

mod search;

use self::search::{RankedSessionIndex, search_query_score, tokenize_search_query};

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

#[cfg(test)]
mod tests;
