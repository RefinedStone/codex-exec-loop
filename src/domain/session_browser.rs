use std::collections::HashMap;

use crate::domain::recent_sessions::RecentSessions;
use crate::domain::session_summary::SessionSummary;

mod search;

use self::search::{RankedSessionIndex, search_query_score, tokenize_search_query};

/*
세션 브라우저 도메인은 최근 세션 저장소를 TUI가 바로 그릴 수 있는 목록 모델로 낮춘다.
어댑터의 ratatui 선택 상태나 popup 표시 여부는 모르고, 검색어/프로젝트 필터/페이지 번호와
선택 가능한 세션 ID만 순수 데이터로 계산한다. 그래서 이 파일의 출력은 session overlay,
empty-state copy, 키보드 이동 로직이 공유하는 source of truth가 된다.
*/

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionBrowserState {
    // 사용자가 커밋한 검색어다. 편집 중인 초안은 TUI adapter가 들고 있고, 도메인은 이미
    // 적용된 필터 입력만 보관한다.
    pub search_query: String,
    // 0-based page index다. result set이 바뀌는 입력은 항상 0으로 돌려 stale page를 막는다.
    pub page_index: usize,
    // page_size는 0이 될 수 없다. projection 단계에서 div_ceil과 slice window의 기준이 된다.
    pub page_size: usize,
    pub project_filter: SessionProjectFilter,
}

impl SessionBrowserState {
    pub fn new(page_size: usize) -> Self {
        /*
         * page_size is normalized at construction because every projection assumes it
         * can divide a result set into non-empty pages. Keeping the guard here lets TUI
         * layout code pass its preferred row capacity without duplicating zero checks.
         */
        Self {
            search_query: String::new(),
            page_index: 0,
            page_size: page_size.max(1),
            project_filter: SessionProjectFilter::AllProjects,
        }
    }

    pub fn set_search_query(&mut self, search_query: impl Into<String>) {
        let normalized_query = search_query.into().trim().to_string();
        // trim 결과가 같으면 page도 유지한다. 사용자가 같은 query를 다시 저장했을 때 목록 위치가
        // 불필요하게 첫 페이지로 튀지 않게 하기 위한 작은 안정성 계약이다.
        if self.search_query == normalized_query {
            return;
        }

        self.search_query = normalized_query;
        self.page_index = 0;
    }

    pub fn set_project_filter(&mut self, project_filter: SessionProjectFilter) {
        /*
         * Project filter changes invalidate the current page in the same way search
         * changes do. The selected row is owned by the TUI state, but page_index lives
         * here so projection never points at a stale page after narrowing the list.
         */
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

        // total_pages는 projection 결과에서 온다. 여기서 다시 clamp해 async refresh로 페이지
        // 수가 줄어든 경우에도 상태가 범위를 벗어나지 않게 한다.
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
        // clear는 세션 브라우저 입력 전체를 기본 view로 되돌린다. page_size는 layout 정책이므로
        // 유지한다.
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
    // cwd 문자열 자체가 프로젝트 identity다. 별도 project id가 없는 최근 세션 데이터의
    // 자연스러운 경계가 작업 디렉터리이기 때문이다.
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
    // 요청한 filter가 더 이상 options에 없으면 AllProjects로 보정된 실제 적용 filter다.
    pub active_project_filter: SessionProjectFilter,
    pub project_filter_options: Vec<SessionProjectFilterOption>,
    // empty-state copy가 "현재 workspace에는 세션이 있는지"를 별도로 말할 수 있게 보존한다.
    pub current_workspace_session_count: usize,
    pub total_session_count: usize,
    // project filter 적용 후 count와 search 적용 후 count를 분리해 UI가 어디서 비었는지 설명한다.
    pub project_filtered_session_count: usize,
    pub filtered_session_count: usize,
    pub page_index: usize,
    pub total_pages: usize,
    // 사용자에게 보여주는 1-based range다. 실제 indexing은 `page_session_indexes`가 맡는다.
    pub visible_session_range: Option<(usize, usize)>,
    // RecentSessions 원본 배열의 index다. projection이 정렬/필터를 해도 page builder가 원본
    // SessionSummary borrow를 안전하게 되찾을 수 있게 한다.
    pub page_session_indexes: Vec<usize>,
}

impl SessionBrowserProjection {
    pub fn cycled_project_filter(&self, delta: isize) -> Option<SessionProjectFilter> {
        /*
         * Filter cycling uses the already-built option list so keyboard navigation sees
         * the same ordering as the rendered menu. rem_euclid makes reverse cycling
         * wrap cleanly without special casing negative deltas.
         */
        let option_count = self.project_filter_options.len() as isize;
        if option_count == 0 {
            return None;
        }

        // 현재 filter가 options에서 사라진 stale 상태라면 AllProjects 위치인 0부터 순환한다.
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
    // 원본 RecentSessions를 borrow한 현재 page의 세션들이다. renderer는 이 Vec만 순회하면 된다.
    pub visible_sessions: Vec<&'a SessionSummary>,
    pub selected_index: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionBrowserSelection {
    // visible page 안의 0-based index다. 비어 있을 때도 키 처리기가 저장할 수 있도록 0을 쓴다.
    pub index: usize,
    // selection을 ID로도 반환해 refresh 뒤 같은 세션을 우선 복원할 수 있게 한다.
    pub session_id: Option<String>,
}

impl<'a> SessionBrowserPage<'a> {
    pub fn selected_session(&self) -> Option<&'a SessionSummary> {
        /*
         * The page owns only borrowed visible sessions, so selected_session returns the
         * same borrow rather than cloning SessionSummary. This keeps the renderer and
         * open-session action tied to the exact projection row currently visible.
         */
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
        /*
         * Row movement is page-local. Page changes are handled by SessionBrowserState,
         * while this helper only moves inside the visible slice and returns both row
         * index and session id so refresh can later prefer identity restoration.
         */
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
    /*
     * Projection is the pure boundary between provider-backed recent sessions and TUI
     * overlay state. It produces indexes into the original list instead of cloning
     * sessions, allowing page construction to borrow exact source records while this
     * function remains focused on filtering, ranking, paging, and counts.
     */
    let search_tokens = tokenize_search_query(&browser_state.search_query);
    // filter options는 검색어와 무관하게 전체 최근 세션에서 만든다. 검색 때문에 현재 프로젝트
    // 필터 선택지가 사라지면 키보드 순환이 예측 불가능해지기 때문이다.
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

    // 먼저 프로젝트 경계로 줄인 뒤 검색 score를 계산한다. score 계산은 current workspace
    // boost를 포함하므로, 검색 결과 정렬은 filter와 workspace context 양쪽을 반영한다.
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
        // 검색이 있을 때만 score 정렬을 적용한다. 검색이 없으면 RecentSessions 원본 순서,
        // 즉 최근 업데이트 순서를 그대로 유지한다.
        ranked_sessions.sort_by(|left, right| {
            right
                .score
                .cmp(&left.score)
                .then_with(|| right.updated_at_epoch.cmp(&left.updated_at_epoch))
                .then_with(|| left.index.cmp(&right.index))
        });
    }

    let filtered_session_count = ranked_sessions.len();
    /*
     * Pagination is computed after both project filtering and search scoring. A stale
     * page index is clamped here instead of mutating browser_state, because projection
     * is read-only and callers may decide separately whether to persist repaired UI
     * state.
     */
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
    // page window는 ranked index 목록만 자른다. 실제 SessionSummary borrow는 page builder가
    // 원본 RecentSessions에서 다시 꺼내 수명 관계를 단순하게 유지한다.
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
    /*
     * Page building composes the pure projection with borrowed SessionSummary rows and
     * selection restoration. Keeping this as a separate step lets tests inspect the
     * projection alone while render/input code can ask for ready-to-display rows.
     */
    let projection =
        project_recent_sessions(recent_sessions, browser_state, current_workspace_directory);

    let visible_sessions = projection
        .page_session_indexes
        .iter()
        .filter_map(|session_index| recent_sessions.items.get(*session_index))
        .collect::<Vec<_>>();

    // ID 기반 선택을 먼저 복원하고, 같은 세션이 현재 page에 없으면 index 기반 선택으로
    // fallback한다. 목록 refresh 중에도 사용자가 보던 행을 최대한 안정적으로 유지하려는 규칙이다.
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
    /*
     * Project options are derived from all recent sessions, not from the current search
     * result. That makes the project menu a stable navigation surface: search can
     * explain "no matches in this project" without hiding the project choice itself.
     */
    let mut workspace_counts = HashMap::new();
    let mut workspace_order = Vec::new();

    // HashMap은 count lookup용이고, workspace_order는 최근 세션에 처음 등장한 순서를 보존한다.
    // 이렇게 해야 프로젝트 filter menu가 매번 hash 순서로 흔들리지 않는다.
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
    // ID가 살아 있으면 row index보다 우선한다. 검색/필터/refresh 뒤에도 같은 session을 찾을 수
    // 있으면 cursor가 그 세션을 따라가야 하기 때문이다.
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
    // 최근 세션 목록이 바뀌면 이전 workspace filter가 더 이상 선택지에 없을 수 있다. 그때는
    // 비어 있는 프로젝트를 유지하지 않고 AllProjects로 보정한다.
    if project_filter_options
        .iter()
        .any(|option| option.filter == *project_filter)
    {
        return project_filter.clone();
    }
    SessionProjectFilter::AllProjects
}

fn matches_project_filter(session: &SessionSummary, project_filter: &SessionProjectFilter) -> bool {
    /*
     * Project identity is the raw cwd string because recent-session data has no richer
     * repository/project id. Keeping the comparison exact avoids accidentally merging
     * sibling worktrees or similarly named directories.
     */
    match project_filter {
        SessionProjectFilter::AllProjects => true,
        SessionProjectFilter::RecentProject {
            workspace_directory,
        } => session.cwd == *workspace_directory,
    }
}

#[cfg(test)]
mod tests;
