use super::{PlanningDraftEditorCloseRequest, PlanningDraftEditorUiState};
use crate::application::service::planning::{PlanningDraftEditorFile, PlanningDraftEditorSession};
use crate::domain::planning::{
    PlanningFileKind, PlanningValidationReport, PlanningValidationSeverity,
};

/*
planning draft editor UI state는 application service가 만든 staged planning files를 TUI overlay 안의
in-memory buffers로 들고 있다. 실제 파일 쓰기, validation 실행, promote는 controller/service가 맡고,
이 state는 "현재 편집 중인 buffer", "저장 전 dirty 여부", "마지막 validation report", "닫기 전 risk"를
화면과 key handler가 같은 source of truth로 읽게 한다.

이 테스트 파일은 filesystem을 건드리지 않고 그 presentation state machine만 고정한다. buffer별 dirty
isolation, save 결과 반영, close confirmation의 2단계 handshake, shell-style 편집 primitive, scroll pinning
계약이 여기서 깨지면 renderer copy와 controller action이 서로 다른 draft 상태를 보게 된다.
*/

fn sample_session() -> PlanningDraftEditorSession {
    /*
     * 두 개의 editable file fixture는 multi-file draft session의 핵심 위험을 노출한다.
     * 현재 buffer만 dirty해지고 file selection을 바꿔도 다른 buffer의 clean 상태가 유지되어야,
     * collect_editable_files가 save payload를 만들 때 "수정된 파일"과 "같이 열린 파일"을 구분할 수 있다.
     */
    PlanningDraftEditorSession {
        draft_name: "bootstrap-test".to_string(),
        draft_directory: "/tmp/bootstrap-test".to_string(),
        editable_files: vec![
            PlanningDraftEditorFile {
                active_path: ".codex-exec-loop/planning/result-output.md".to_string(),
                staged_path: "/tmp/bootstrap-test/result-output.md".to_string(),
                body: "version = 1".to_string(),
            },
            PlanningDraftEditorFile {
                active_path: ".codex-exec-loop/planning/result-output.md".to_string(),
                staged_path: "/tmp/bootstrap-test/.codex-exec-loop/planning/result-output.md"
                    .to_string(),
                body: "{\n  \"version\": 1,\n  \"tasks\": []\n}".to_string(),
            },
        ],
        validation_report: PlanningValidationReport::default(),
    }
}

fn single_buffer_session(body: &str) -> PlanningDraftEditorSession {
    /*
     * cursor movement와 word deletion 테스트는 multi-file selection noise가 필요 없다.
     * 단일 buffer session을 쓰면 editor primitive가 line/cursor/scroll state만 어떻게 바꾸는지 좁게 검증할 수 있다.
     */
    PlanningDraftEditorSession {
        draft_name: "bootstrap-test".to_string(),
        draft_directory: "/tmp/bootstrap-test".to_string(),
        editable_files: vec![PlanningDraftEditorFile {
            active_path: ".codex-exec-loop/planning/result-output.md".to_string(),
            staged_path: "/tmp/bootstrap-test/result-output.md".to_string(),
            body: body.to_string(),
        }],
        validation_report: PlanningValidationReport::default(),
    }
}

#[test]
fn editing_buffer_marks_it_dirty_and_preserves_file_switching() {
    /*
     * draft editor는 여러 staged file을 동시에 연다. 한 buffer 편집은 전체 session save payload에는
     * 반영되어야 하지만, 다른 buffer dirty flag를 오염시키면 안 된다. 이 계약이 깨지면 footer의 dirty
     * label과 실제 save payload가 서로 다른 파일 상태를 말하게 된다.
     */
    let mut state = PlanningDraftEditorUiState::default();
    state.open_session(sample_session());

    state.insert_character('#');
    assert!(state.selected_buffer().expect("buffer").is_dirty());
    assert_eq!(
        state.selected_buffer().expect("buffer").body(),
        "#version = 1"
    );

    state.move_file_selection(1);
    assert_eq!(
        state.selected_buffer().expect("buffer").active_path(),
        ".codex-exec-loop/planning/result-output.md"
    );
    assert!(!state.selected_buffer().expect("buffer").is_dirty());
    let files = state.collect_editable_files();
    assert_eq!(files.len(), 2);
    assert_eq!(files[0].body, "#version = 1");
}

#[test]
fn save_result_clears_dirty_flags_and_updates_validation_report() {
    /*
     * save action은 controller가 collect_editable_files를 service에 넘긴 뒤 성공 결과로 돌아온다.
     * UI state는 그 순간 dirty flags를 지우고 새 validation report를 화면의 source of truth로 교체해야 한다.
     * 그렇지 않으면 editor footer가 마지막 저장본과 현재 buffer 상태를 섞어 보여 준다.
     */
    let mut state = PlanningDraftEditorUiState::default();
    state.open_session(sample_session());
    state.insert_character('#');
    let mut report = PlanningValidationReport::default();
    report.push_warning(
        PlanningFileKind::Directions,
        "test-warning",
        "directions summary still needs review",
    );

    state.apply_save_result(report.clone());

    assert!(!state.has_dirty_buffers());
    assert_eq!(state.validation_report(), Some(&report));
    assert_eq!(
        state
            .validation_report()
            .expect("validation")
            .issues
            .first()
            .expect("issue")
            .severity,
        PlanningValidationSeverity::Warning
    );
}

#[test]
fn request_close_requires_confirmation_for_dirty_buffers() {
    /*
     * dirty draft를 바로 닫으면 아직 service에 저장하지 않은 in-memory edit가 사라진다.
     * 첫 close는 risk를 표시하고 같은 risk가 pending인 두 번째 close만 confirm으로 통과하는
     * 2-step guard를 고정한다.
     */
    let mut state = PlanningDraftEditorUiState::default();
    state.open_session(sample_session());
    state.insert_character('#');
    let first_request = state.request_close();
    let second_request = state.request_close();

    assert_eq!(
        first_request,
        PlanningDraftEditorCloseRequest::ConfirmationRequired(
            state.close_risk().expect("close risk should exist")
        )
    );
    assert_eq!(
        second_request,
        PlanningDraftEditorCloseRequest::Confirmed(
            state
                .close_risk()
                .expect("close risk should remain visible")
        )
    );
}

#[test]
fn request_close_requires_confirmation_for_invalid_saved_draft() {
    /*
     * 저장되어 dirty하지 않더라도 validation error가 남은 draft는 promote 가능한 accepted state가 아니다.
     * close guard는 unsaved edit뿐 아니라 "디스크에 남아 있지만 아직 고쳐야 하는 staged draft"도 operator가
     * 인지하고 닫게 해야 한다.
     */
    let mut state = PlanningDraftEditorUiState::default();
    state.open_session(sample_session());
    let mut invalid_report = PlanningValidationReport::default();
    invalid_report.push_error(
        PlanningFileKind::TaskAuthority,
        "invalid-json",
        "result-output.md must parse as valid markdown",
    );
    state.apply_save_result(invalid_report);
    let request = state.request_close();

    assert_eq!(
        request,
        PlanningDraftEditorCloseRequest::ConfirmationRequired(
            state
                .close_risk()
                .expect("invalid draft should require confirmation")
        )
    );
    assert!(state.is_close_confirmation_pending());
}

#[test]
fn editing_after_close_warning_clears_pending_confirmation() {
    /*
     * close confirmation은 특정 risk snapshot에 대한 승인이다. 사용자가 다시 cursor를 움직이거나 편집을
     * 시작하면 close intent의 맥락이 바뀌므로 pending confirmation을 무효화해야 한다.
     */
    let mut state = PlanningDraftEditorUiState::default();
    state.open_session(sample_session());
    state.insert_character('#');
    let _ = state.request_close();

    state.move_cursor_right();

    assert!(!state.is_close_confirmation_pending());
}

#[test]
fn delete_previous_word_removes_the_full_word_before_cursor() {
    /*
     * Ctrl-W 계열 편집은 draft editor에서도 shell input과 비슷하게 동작해야 한다.
     * 한 줄 안에서 직전 단어만 제거하고 cursor를 삭제 지점으로 되돌리는 계약을 고정한다.
     */
    let mut state = PlanningDraftEditorUiState::default();
    state.open_session(single_buffer_session("alpha beta"));
    for _ in 0..10 {
        state.move_cursor_right();
    }
    state.delete_previous_word();
    let buffer = state.selected_buffer().expect("buffer");
    assert_eq!(buffer.body(), "alpha ");
    assert_eq!(buffer.cursor_line_index(), 0);
    assert_eq!(buffer.cursor_column(), 6);
}

#[test]
fn delete_previous_word_trims_whitespace_and_respects_newline_boundaries() {
    /*
     * word deletion은 단어 앞 공백을 정리하되 줄 경계를 넘어 이전 line을 합치지 않는다.
     * markdown/JSON draft에서 구조적 줄 경계를 우발적으로 깨지 않기 위한 편집 규칙이다.
     */
    let mut state = PlanningDraftEditorUiState::default();
    state.open_session(single_buffer_session("alpha\nbeta gamma"));
    for _ in 0..16 {
        state.move_cursor_right();
    }
    state.delete_previous_word();
    let buffer = state.selected_buffer().expect("buffer");
    assert_eq!(buffer.body(), "alpha\nbeta ");
    assert_eq!(buffer.cursor_line_index(), 1);
    assert_eq!(buffer.cursor_column(), 5);
}

#[test]
fn sync_editor_scroll_keeps_cursor_visible_without_repinning_every_move() {
    /*
     * editor scroll은 cursor를 보이게 해야 하지만, cursor가 viewport 안에 있는 동안 매 이동마다 scroll을
     * 다시 pinning하면 화면이 불필요하게 튄다. 이 테스트는 "밖으로 나갈 때만 따라가기" 동작을 고정한다.
     */
    let mut state = PlanningDraftEditorUiState::default();
    state.open_session(single_buffer_session("1\n2\n3\n4\n5\n6"));
    for _ in 0..4 {
        state.move_cursor_down();
    }
    state.sync_editor_scroll(3);
    assert_eq!(state.selected_buffer().expect("buffer").editor_scroll(), 2);

    state.move_cursor_up();
    state.sync_editor_scroll(3);
    assert_eq!(state.selected_buffer().expect("buffer").editor_scroll(), 2);

    state.move_cursor_up();
    state.sync_editor_scroll(3);
    assert_eq!(state.selected_buffer().expect("buffer").editor_scroll(), 2);

    state.move_cursor_up();
    state.sync_editor_scroll(3);
    assert_eq!(state.selected_buffer().expect("buffer").editor_scroll(), 1);
}
