use ratatui::widgets::ListState;

use crate::domain::session_browser::{SessionBrowserState, SessionProjectFilter};

// 세션 오버레이의 검색 입력은 즉시 커밋되는 필터가 아니라 편집 중인 초안이다.
// Enter가 눌릴 때만 `SessionBrowserState`로 넘어가야 하므로 커밋된 쿼리와 버퍼를
// 분리해 둔다.
#[derive(Debug, Default)]
struct SessionSearchQueryEditorState {
    is_editing: bool,
    buffer: String,
}

// TUI 어댑터가 세션 브라우저 도메인 상태와 ratatui 렌더링 상태를 묶어 들고 있는
// 작은 경계 객체다. 도메인의 검색/필터/페이지 규칙은 `SessionBrowserState`에
// 맡기고, 행 선택과 스크롤 오프셋처럼 위젯 구현에 가까운 값만 여기에서 관리한다.
#[derive(Debug)]
pub(super) struct SessionOverlayUiState {
    // `ListState`는 렌더러가 행 선택과 스크롤 오프셋을 갱신하는 가변 상태다.
    // 결과 집합이 바뀌면 이전 행 번호가 새 페이지를 가리키지 않도록 즉시 초기화한다.
    pub list_state: ListState,
    // 검색어, 프로젝트 필터, 페이지 번호처럼 세션 목록 조회 요청에 직접 반영되는
    // 순수 브라우저 상태다.
    browser_state: SessionBrowserState,
    // 현재 표시 중인 상세 영역과 열기 동작은 행 번호보다 세션 ID가 안정적이므로,
    // 선택된 행과 별도로 세션 식별자를 보관한다.
    selected_session_id: Option<String>,
    search_query_editor: SessionSearchQueryEditorState,
}

impl Default for SessionOverlayUiState {
    fn default() -> Self {
        // production 경로는 상위 컨트롤러가 페이지 크기를 주입한다. 기본값은 단위
        // 테스트와 작은 보조 생성 경로에서만 쓰이는 보수적인 fallback이다.
        Self::new(10)
    }
}

impl SessionOverlayUiState {
    pub fn new(page_size: usize) -> Self {
        Self {
            list_state: ListState::default(),
            browser_state: SessionBrowserState::new(page_size),
            selected_session_id: None,
            search_query_editor: SessionSearchQueryEditorState::default(),
        }
    }
    pub fn browser_state(&self) -> &SessionBrowserState {
        &self.browser_state
    }
    pub fn selected_session_id(&self) -> Option<&str> {
        self.selected_session_id.as_deref()
    }
    pub fn set_selected_session_id(&mut self, selected_session_id: Option<String>) {
        self.selected_session_id = selected_session_id;
    }
    pub fn is_search_query_editing(&self) -> bool {
        self.search_query_editor.is_editing
    }
    pub fn search_query_editor_buffer(&self) -> &str {
        &self.search_query_editor.buffer
    }

    pub fn start_search_query_edit(&mut self) {
        // 편집 시작 시점의 커밋된 검색어를 복사해 Esc 취소와 기존 쿼리 수정이 같은
        // 버퍼 경로를 타게 한다.
        self.search_query_editor.is_editing = true;
        self.search_query_editor.buffer = self.browser_state.search_query.clone();
    }

    pub fn save_search_query_edit(&mut self) {
        let next_query = self.search_query_editor.buffer.clone();
        // 커밋은 반드시 `set_search_query`를 거친다. 그래야 도메인 쪽 trim/page reset과
        // TUI 쪽 list reset 계약이 한 번에 적용된다.
        self.set_search_query(next_query);
        self.search_query_editor.is_editing = false;
        self.search_query_editor.buffer = self.browser_state.search_query.clone();
    }

    pub fn cancel_search_query_edit(&mut self) {
        // 취소는 브라우저 상태를 건드리지 않고, 초안만 마지막 커밋 상태로 되돌린다.
        self.search_query_editor.is_editing = false;
        self.search_query_editor.buffer = self.browser_state.search_query.clone();
    }

    pub fn push_search_query_character(&mut self, character: char) {
        self.search_query_editor.buffer.push(character);
    }

    pub fn pop_search_query_character(&mut self) {
        self.search_query_editor.buffer.pop();
    }

    pub fn set_search_query(&mut self, search_query: impl Into<String>) {
        self.browser_state.set_search_query(search_query);
        // 검색어 변경은 완전히 다른 결과 집합을 만들 수 있으므로 선택 행과 스크롤
        // 오프셋을 이전 페이지에서 가져오지 않는다.
        self.list_state = ListState::default();
    }

    pub fn set_project_filter(&mut self, project_filter: SessionProjectFilter) {
        self.browser_state.set_project_filter(project_filter);
        // 프로젝트 필터 역시 목록의 identity를 바꾸는 입력이다. 도메인은 첫 페이지로
        // 되돌리고, 어댑터는 위젯 선택 상태를 함께 비운다.
        self.list_state = ListState::default();
    }

    pub fn move_page(&mut self, delta: isize, total_pages: usize) {
        self.browser_state.move_page(delta, total_pages);
        // 페이지 이동 뒤의 행 번호는 이전 페이지 행 번호와 의미가 다르다.
        self.list_state = ListState::default();
    }

    pub fn jump_to_first_page(&mut self) {
        self.browser_state.jump_to_first_page();
        self.list_state = ListState::default();
    }

    pub fn jump_to_last_page(&mut self, total_pages: usize) {
        self.browser_state.jump_to_last_page(total_pages);
        self.list_state = ListState::default();
    }

    pub fn clear_browser_state(&mut self) {
        // 명시적인 clear는 검색어와 프로젝트 필터까지 포함한 브라우저 입력 전체를
        // 기본값으로 되돌리는 강한 초기화다.
        self.browser_state.clear();
        self.list_state = ListState::default();
        self.selected_session_id = None;
        self.search_query_editor.is_editing = false;
        self.search_query_editor.buffer = self.browser_state.search_query.clone();
    }

    pub fn sync_selected_session(&mut self, selected_session_index: Option<usize>) {
        // `ListState::select(Some(_))`는 offset을 보존하고, `None`은 offset까지 0으로
        // 비운다. 오버레이 상세 선택과 ratatui 위젯 상태가 같은 규칙을 공유하게 둔다.
        self.list_state.select(selected_session_index);
    }

    pub fn reset(&mut self) {
        // 오버레이를 닫았다 다시 열 때의 가벼운 reset이다. 사용자가 입력한 검색/필터는
        // 유지하고, 화면 선택과 편집 중인 초안만 정리한다.
        self.list_state = ListState::default();
        self.selected_session_id = None;
        self.search_query_editor.is_editing = false;
        self.search_query_editor.buffer = self.browser_state.search_query.clone();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sync_selected_session_preserves_existing_offset() {
        let mut state = SessionOverlayUiState::new(10);
        state.list_state = ListState::default().with_offset(4).with_selected(Some(5));

        state.sync_selected_session(Some(2));

        // Some 선택은 현재 viewport offset을 유지해야 키보드 이동 중 스크롤이 튀지 않는다.
        assert_eq!(state.list_state.selected(), Some(2));
        assert_eq!(state.list_state.offset(), 4);
    }

    #[test]
    fn sync_selected_session_with_none_clears_offset() {
        let mut state = SessionOverlayUiState::new(10);
        state.list_state = ListState::default().with_offset(4).with_selected(Some(5));

        state.sync_selected_session(None);

        // 선택 해제는 ratatui의 기본 규칙대로 offset까지 비워 다음 렌더가 첫 행에서 시작한다.
        assert_eq!(state.list_state.selected(), None);
        assert_eq!(state.list_state.offset(), 0);
    }

    #[test]
    fn reset_clears_selection_editor_and_selected_session_but_keeps_browser_query() {
        let mut state = SessionOverlayUiState::new(10);
        state.list_state = ListState::default().with_offset(4).with_selected(Some(5));
        state.set_search_query("bugfix");
        state.set_selected_session_id(Some("thread-2".to_string()));
        state.start_search_query_edit();
        state.push_search_query_character('x');

        state.reset();

        // reset은 오버레이 표시 상태만 정리하고, 사용자가 확정한 목록 쿼리는 보존한다.
        assert_eq!(state.list_state.selected(), None);
        assert_eq!(state.list_state.offset(), 0);
        assert_eq!(state.selected_session_id(), None);
        assert!(!state.is_search_query_editing());
        assert_eq!(state.search_query_editor_buffer(), "bugfix");
        assert_eq!(state.browser_state().search_query, "bugfix");
    }

    #[test]
    fn search_query_resets_list_state_and_page_index() {
        let mut state = SessionOverlayUiState::new(10);
        state.list_state = ListState::default().with_offset(4).with_selected(Some(5));
        state.move_page(1, 4);

        state.set_search_query("bugfix");

        // 새 검색 결과는 첫 페이지에서 시작하고 이전 행 선택을 이어받지 않는다.
        assert_eq!(state.browser_state().search_query, "bugfix");
        assert_eq!(state.browser_state().page_index, 0);
        assert_eq!(state.list_state.selected(), None);
        assert_eq!(state.list_state.offset(), 0);
    }

    #[test]
    fn project_filter_resets_page_and_selection() {
        let mut state = SessionOverlayUiState::new(10);
        state.list_state = ListState::default().with_offset(3).with_selected(Some(4));
        state.move_page(2, 5);

        state.set_project_filter(SessionProjectFilter::RecentProject {
            workspace_directory: "/tmp/root".to_string(),
        });

        assert_eq!(
            state.browser_state().project_filter,
            SessionProjectFilter::RecentProject {
                workspace_directory: "/tmp/root".to_string(),
            }
        );
        // 필터가 바뀐 뒤에는 기존 페이지와 row index가 같은 대상을 뜻하지 않는다.
        assert_eq!(state.browser_state().page_index, 0);
        assert_eq!(state.list_state.selected(), None);
    }

    #[test]
    fn move_page_clamps_and_clears_list_state() {
        let mut state = SessionOverlayUiState::new(10);
        state.list_state = ListState::default().with_offset(2).with_selected(Some(1));

        state.move_page(4, 2);

        // 도메인이 마지막 페이지로 clamp하고, 어댑터는 새 페이지의 선택을 비운다.
        assert_eq!(state.browser_state().page_index, 1);
        assert_eq!(state.list_state.selected(), None);
        assert_eq!(state.list_state.offset(), 0);
    }

    #[test]
    fn clear_browser_state_resets_query_filter_selection_and_editor() {
        let mut state = SessionOverlayUiState::new(10);
        state.set_search_query("docs");
        state.set_project_filter(SessionProjectFilter::RecentProject {
            workspace_directory: "/tmp/root".to_string(),
        });
        state.move_page(3, 5);
        state.set_selected_session_id(Some("thread-2".to_string()));
        state.start_search_query_edit();
        state.push_search_query_character('x');

        state.clear_browser_state();

        // clear는 검색/필터/페이지/선택/편집 초안을 모두 기본 상태로 되돌린다.
        assert_eq!(state.browser_state().search_query, "");
        assert_eq!(state.browser_state().page_index, 0);
        assert_eq!(
            state.browser_state().project_filter,
            SessionProjectFilter::AllProjects
        );
        assert_eq!(state.selected_session_id(), None);
        assert!(!state.is_search_query_editing());
        assert_eq!(state.search_query_editor_buffer(), "");
        assert_eq!(state.list_state.selected(), None);
        assert_eq!(state.list_state.offset(), 0);
    }

    #[test]
    fn jump_to_last_page_clamps_and_clears_list_state() {
        let mut state = SessionOverlayUiState::new(10);
        state.list_state = ListState::default().with_offset(2).with_selected(Some(1));

        state.jump_to_last_page(3);

        // 마지막 페이지 이동도 결과 viewport가 바뀌므로 위젯 선택 상태를 초기화한다.
        assert_eq!(state.browser_state().page_index, 2);
        assert_eq!(state.list_state.selected(), None);
        assert_eq!(state.list_state.offset(), 0);
    }

    #[test]
    fn save_search_query_edit_commits_trimmed_query() {
        let mut state = SessionOverlayUiState::new(10);
        state.start_search_query_edit();
        state.push_search_query_character(' ');
        state.push_search_query_character('d');
        state.push_search_query_character('o');
        state.push_search_query_character('c');
        state.push_search_query_character('s');
        state.push_search_query_character(' ');

        state.save_search_query_edit();

        // 저장은 도메인 검색어 정규화 결과를 편집 버퍼에도 되돌려 보여준다.
        assert!(!state.is_search_query_editing());
        assert_eq!(state.browser_state().search_query, "docs");
        assert_eq!(state.search_query_editor_buffer(), "docs");
    }

    #[test]
    fn cancel_search_query_edit_restores_saved_query() {
        let mut state = SessionOverlayUiState::new(10);
        state.set_search_query("release");
        state.start_search_query_edit();
        state.push_search_query_character(' ');
        state.push_search_query_character('x');

        state.cancel_search_query_edit();

        // 취소된 초안은 커밋된 검색어로 복구되고 브라우저 쿼리는 변경되지 않는다.
        assert!(!state.is_search_query_editing());
        assert_eq!(state.browser_state().search_query, "release");
        assert_eq!(state.search_query_editor_buffer(), "release");
    }
}
