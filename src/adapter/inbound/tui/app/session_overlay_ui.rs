// 학습 주석: `use`는 긴 모듈 경로의 이름을 현재 파일로 가져와 아래 코드에서 짧게 쓰도록 합니다.
use ratatui::widgets::ListState;

// 학습 주석: `use`는 긴 모듈 경로의 이름을 현재 파일로 가져와 아래 코드에서 짧게 쓰도록 합니다.
use crate::domain::session_browser::{SessionBrowserState, SessionProjectFilter};

// 학습 주석: `#[...]` 속성은 바로 뒤의 항목에 메타데이터를 붙여 파생 구현, 조건부 컴파일, 테스트 동작 등을 지정합니다.
#[derive(Debug, Default)]
// 학습 주석: `struct`는 여러 값을 하나의 의미 있는 데이터 묶음으로 다루기 위한 Rust의 구조체 정의입니다.
struct SessionSearchQueryEditorState {
    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
    is_editing: bool,
    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
    buffer: String,
}

// 학습 주석: `#[...]` 속성은 바로 뒤의 항목에 메타데이터를 붙여 파생 구현, 조건부 컴파일, 테스트 동작 등을 지정합니다.
#[derive(Debug)]
// 학습 주석: `struct`는 여러 값을 하나의 의미 있는 데이터 묶음으로 다루기 위한 Rust의 구조체 정의입니다.
pub(super) struct SessionOverlayUiState {
    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
    pub list_state: ListState,
    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
    browser_state: SessionBrowserState,
    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
    selected_session_id: Option<String>,
    // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
    search_query_editor: SessionSearchQueryEditorState,
}

// 학습 주석: `impl` 블록은 특정 타입이나 trait 구현에 속한 함수들을 한곳에 묶습니다.
impl Default for SessionOverlayUiState {
    // 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
    fn default() -> Self {
        // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
        Self::new(10)
    }
}

// 학습 주석: `impl` 블록은 특정 타입이나 trait 구현에 속한 함수들을 한곳에 묶습니다.
impl SessionOverlayUiState {
    // 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
    pub fn new(page_size: usize) -> Self {
        Self {
            // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
            list_state: ListState::default(),
            // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
            browser_state: SessionBrowserState::new(page_size),
            // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
            selected_session_id: None,
            // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
            search_query_editor: SessionSearchQueryEditorState::default(),
        }
    }

    // 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
    pub fn browser_state(&self) -> &SessionBrowserState {
        &self.browser_state
    }

    // 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
    pub fn selected_session_id(&self) -> Option<&str> {
        self.selected_session_id.as_deref()
    }

    // 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
    pub fn set_selected_session_id(&mut self, selected_session_id: Option<String>) {
        self.selected_session_id = selected_session_id;
    }

    // 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
    pub fn is_search_query_editing(&self) -> bool {
        self.search_query_editor.is_editing
    }

    // 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
    pub fn search_query_editor_buffer(&self) -> &str {
        &self.search_query_editor.buffer
    }

    // 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
    pub fn start_search_query_edit(&mut self) {
        self.search_query_editor.is_editing = true;
        self.search_query_editor.buffer = self.browser_state.search_query.clone();
    }

    // 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
    pub fn save_search_query_edit(&mut self) {
        // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
        let next_query = self.search_query_editor.buffer.clone();
        self.set_search_query(next_query);
        self.search_query_editor.is_editing = false;
        self.search_query_editor.buffer = self.browser_state.search_query.clone();
    }

    // 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
    pub fn cancel_search_query_edit(&mut self) {
        self.search_query_editor.is_editing = false;
        self.search_query_editor.buffer = self.browser_state.search_query.clone();
    }

    // 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
    pub fn push_search_query_character(&mut self, character: char) {
        self.search_query_editor.buffer.push(character);
    }

    // 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
    pub fn pop_search_query_character(&mut self) {
        self.search_query_editor.buffer.pop();
    }

    // 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
    pub fn set_search_query(&mut self, search_query: impl Into<String>) {
        self.browser_state.set_search_query(search_query);
        self.list_state = ListState::default();
    }

    // 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
    pub fn set_project_filter(&mut self, project_filter: SessionProjectFilter) {
        self.browser_state.set_project_filter(project_filter);
        self.list_state = ListState::default();
    }

    // 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
    pub fn move_page(&mut self, delta: isize, total_pages: usize) {
        self.browser_state.move_page(delta, total_pages);
        self.list_state = ListState::default();
    }

    // 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
    pub fn jump_to_first_page(&mut self) {
        self.browser_state.jump_to_first_page();
        self.list_state = ListState::default();
    }

    // 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
    pub fn jump_to_last_page(&mut self, total_pages: usize) {
        self.browser_state.jump_to_last_page(total_pages);
        self.list_state = ListState::default();
    }

    // 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
    pub fn clear_browser_state(&mut self) {
        self.browser_state.clear();
        self.list_state = ListState::default();
        self.selected_session_id = None;
        self.search_query_editor.is_editing = false;
        self.search_query_editor.buffer = self.browser_state.search_query.clone();
    }

    // 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
    pub fn sync_selected_session(&mut self, selected_session_index: Option<usize>) {
        self.list_state.select(selected_session_index);
    }

    // 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
    pub fn reset(&mut self) {
        self.list_state = ListState::default();
        self.selected_session_id = None;
        self.search_query_editor.is_editing = false;
        self.search_query_editor.buffer = self.browser_state.search_query.clone();
    }
}

// 학습 주석: `#[...]` 속성은 바로 뒤의 항목에 메타데이터를 붙여 파생 구현, 조건부 컴파일, 테스트 동작 등을 지정합니다.
#[cfg(test)]
// 학습 주석: `mod` 선언은 Rust 파일/하위 모듈을 현재 모듈 트리에 연결하는 입구 역할을 합니다.
mod tests {
    // 학습 주석: `use`는 긴 모듈 경로의 이름을 현재 파일로 가져와 아래 코드에서 짧게 쓰도록 합니다.
    use super::*;

    // 학습 주석: `#[...]` 속성은 바로 뒤의 항목에 메타데이터를 붙여 파생 구현, 조건부 컴파일, 테스트 동작 등을 지정합니다.
    #[test]
    // 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
    fn sync_selected_session_preserves_existing_offset() {
        // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
        let mut state = SessionOverlayUiState::new(10);
        state.list_state = ListState::default().with_offset(4).with_selected(Some(5));

        state.sync_selected_session(Some(2));

        assert_eq!(state.list_state.selected(), Some(2));
        assert_eq!(state.list_state.offset(), 4);
    }

    // 학습 주석: `#[...]` 속성은 바로 뒤의 항목에 메타데이터를 붙여 파생 구현, 조건부 컴파일, 테스트 동작 등을 지정합니다.
    #[test]
    // 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
    fn sync_selected_session_with_none_clears_offset() {
        // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
        let mut state = SessionOverlayUiState::new(10);
        state.list_state = ListState::default().with_offset(4).with_selected(Some(5));

        state.sync_selected_session(None);

        assert_eq!(state.list_state.selected(), None);
        assert_eq!(state.list_state.offset(), 0);
    }

    // 학습 주석: `#[...]` 속성은 바로 뒤의 항목에 메타데이터를 붙여 파생 구현, 조건부 컴파일, 테스트 동작 등을 지정합니다.
    #[test]
    // 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
    fn reset_clears_selection_editor_and_selected_session_but_keeps_browser_query() {
        // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
        let mut state = SessionOverlayUiState::new(10);
        state.list_state = ListState::default().with_offset(4).with_selected(Some(5));
        state.set_search_query("bugfix");
        state.set_selected_session_id(Some("thread-2".to_string()));
        state.start_search_query_edit();
        state.push_search_query_character('x');

        state.reset();

        assert_eq!(state.list_state.selected(), None);
        assert_eq!(state.list_state.offset(), 0);
        assert_eq!(state.selected_session_id(), None);
        assert!(!state.is_search_query_editing());
        assert_eq!(state.search_query_editor_buffer(), "bugfix");
        assert_eq!(state.browser_state().search_query, "bugfix");
    }

    // 학습 주석: `#[...]` 속성은 바로 뒤의 항목에 메타데이터를 붙여 파생 구현, 조건부 컴파일, 테스트 동작 등을 지정합니다.
    #[test]
    // 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
    fn search_query_resets_list_state_and_page_index() {
        // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
        let mut state = SessionOverlayUiState::new(10);
        state.list_state = ListState::default().with_offset(4).with_selected(Some(5));
        state.move_page(1, 4);

        state.set_search_query("bugfix");

        assert_eq!(state.browser_state().search_query, "bugfix");
        assert_eq!(state.browser_state().page_index, 0);
        assert_eq!(state.list_state.selected(), None);
        assert_eq!(state.list_state.offset(), 0);
    }

    // 학습 주석: `#[...]` 속성은 바로 뒤의 항목에 메타데이터를 붙여 파생 구현, 조건부 컴파일, 테스트 동작 등을 지정합니다.
    #[test]
    // 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
    fn project_filter_resets_page_and_selection() {
        // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
        let mut state = SessionOverlayUiState::new(10);
        state.list_state = ListState::default().with_offset(3).with_selected(Some(4));
        state.move_page(2, 5);

        state.set_project_filter(SessionProjectFilter::RecentProject {
            // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
            workspace_directory: "/tmp/root".to_string(),
        });

        assert_eq!(
            state.browser_state().project_filter,
            // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
            SessionProjectFilter::RecentProject {
                // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
                workspace_directory: "/tmp/root".to_string(),
            }
        );
        assert_eq!(state.browser_state().page_index, 0);
        assert_eq!(state.list_state.selected(), None);
    }

    // 학습 주석: `#[...]` 속성은 바로 뒤의 항목에 메타데이터를 붙여 파생 구현, 조건부 컴파일, 테스트 동작 등을 지정합니다.
    #[test]
    // 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
    fn move_page_clamps_and_clears_list_state() {
        // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
        let mut state = SessionOverlayUiState::new(10);
        state.list_state = ListState::default().with_offset(2).with_selected(Some(1));

        state.move_page(4, 2);

        assert_eq!(state.browser_state().page_index, 1);
        assert_eq!(state.list_state.selected(), None);
        assert_eq!(state.list_state.offset(), 0);
    }

    // 학습 주석: `#[...]` 속성은 바로 뒤의 항목에 메타데이터를 붙여 파생 구현, 조건부 컴파일, 테스트 동작 등을 지정합니다.
    #[test]
    // 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
    fn clear_browser_state_resets_query_filter_selection_and_editor() {
        // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
        let mut state = SessionOverlayUiState::new(10);
        state.set_search_query("docs");
        state.set_project_filter(SessionProjectFilter::RecentProject {
            // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
            workspace_directory: "/tmp/root".to_string(),
        });
        state.move_page(3, 5);
        state.set_selected_session_id(Some("thread-2".to_string()));
        state.start_search_query_edit();
        state.push_search_query_character('x');

        state.clear_browser_state();

        assert_eq!(state.browser_state().search_query, "");
        assert_eq!(state.browser_state().page_index, 0);
        assert_eq!(
            state.browser_state().project_filter,
            // 학습 주석: 이 줄은 이름, 타입, 값 또는 경로를 연결해 Rust가 어떤 대상을 다루는지 분명히 합니다.
            SessionProjectFilter::AllProjects
        );
        assert_eq!(state.selected_session_id(), None);
        assert!(!state.is_search_query_editing());
        assert_eq!(state.search_query_editor_buffer(), "");
        assert_eq!(state.list_state.selected(), None);
        assert_eq!(state.list_state.offset(), 0);
    }

    // 학습 주석: `#[...]` 속성은 바로 뒤의 항목에 메타데이터를 붙여 파생 구현, 조건부 컴파일, 테스트 동작 등을 지정합니다.
    #[test]
    // 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
    fn jump_to_last_page_clamps_and_clears_list_state() {
        // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
        let mut state = SessionOverlayUiState::new(10);
        state.list_state = ListState::default().with_offset(2).with_selected(Some(1));

        state.jump_to_last_page(3);

        assert_eq!(state.browser_state().page_index, 2);
        assert_eq!(state.list_state.selected(), None);
        assert_eq!(state.list_state.offset(), 0);
    }

    // 학습 주석: `#[...]` 속성은 바로 뒤의 항목에 메타데이터를 붙여 파생 구현, 조건부 컴파일, 테스트 동작 등을 지정합니다.
    #[test]
    // 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
    fn save_search_query_edit_commits_trimmed_query() {
        // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
        let mut state = SessionOverlayUiState::new(10);
        state.start_search_query_edit();
        state.push_search_query_character(' ');
        state.push_search_query_character('d');
        state.push_search_query_character('o');
        state.push_search_query_character('c');
        state.push_search_query_character('s');
        state.push_search_query_character(' ');

        state.save_search_query_edit();

        assert!(!state.is_search_query_editing());
        assert_eq!(state.browser_state().search_query, "docs");
        assert_eq!(state.search_query_editor_buffer(), "docs");
    }

    // 학습 주석: `#[...]` 속성은 바로 뒤의 항목에 메타데이터를 붙여 파생 구현, 조건부 컴파일, 테스트 동작 등을 지정합니다.
    #[test]
    // 학습 주석: `fn`은 재사용 가능한 동작 단위이며, 입력 매개변수와 반환 타입으로 호출 계약을 분명히 합니다.
    fn cancel_search_query_edit_restores_saved_query() {
        // 학습 주석: `let`은 새 지역 변수를 만들며, `mut`가 있을 때만 이후에 값을 다시 대입할 수 있습니다.
        let mut state = SessionOverlayUiState::new(10);
        state.set_search_query("release");
        state.start_search_query_edit();
        state.push_search_query_character(' ');
        state.push_search_query_character('x');

        state.cancel_search_query_edit();

        assert!(!state.is_search_query_editing());
        assert_eq!(state.browser_state().search_query, "release");
        assert_eq!(state.search_query_editor_buffer(), "release");
    }
}
