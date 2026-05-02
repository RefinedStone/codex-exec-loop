use super::{PlanningDraftEditorCloseRequest, PlanningDraftEditorUiState};
use crate::application::service::planning::{PlanningDraftEditorFile, PlanningDraftEditorSession};
use crate::domain::planning::{
    PlanningFileKind, PlanningValidationReport, PlanningValidationSeverity,
};

/*
학습 주석: planning draft editor UI state는 TUI overlay 내부에서 staged planning files를
메모리 buffer로 편집하고, 저장/검증/승격 전에 close guard를 제공하는 presentation state입니다.
이 테스트 파일은 파일 시스템을 직접 건드리지 않고 buffer dirty state, cursor editing, validation
report 교체, close confirmation state machine만 고정합니다.
*/

fn sample_session() -> PlanningDraftEditorSession {
    // 학습 주석: 두 개의 editable file fixture를 둔 이유는 현재 buffer만 dirty해지고 file
    // selection을 바꿔도 다른 buffer의 clean 상태가 유지되는가를 검증하기 위해서입니다.
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
    // 학습 주석: cursor movement와 word deletion 테스트는 multi-file selection noise가 필요 없으므로
    // 하나의 buffer만 가진 session으로 editor primitive의 동작을 좁혀 검증합니다.
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
    // 학습 주석: draft editor는 여러 staged file을 동시에 들고 있습니다. 한 buffer 편집이 전체
    // session save payload에는 반영되어야 하지만, 다른 buffer dirty flag를 오염시키면 안 됩니다.
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
    // 학습 주석: save action은 현재 buffer 내용을 application service에 넘긴 뒤, 성공하면 UI의
    // dirty flag를 지우고 새 validation report를 화면의 source of truth로 교체합니다.
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
    // 학습 주석: dirty draft를 바로 닫으면 staged edit가 사라집니다. 첫 close는 risk를 표시하고,
    // 같은 risk가 pending인 두 번째 close만 실제 confirm으로 통과하는 2-step guard를 고정합니다.
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
    // 학습 주석: 저장되어 dirty하지 않더라도 validation error가 남은 draft는 승격 가능한 상태가
    // 아닙니다. close guard가 unsaved뿐 아니라 invalid saved draft도 막는지 확인합니다.
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
    // 학습 주석: close confirmation은 특정 risk snapshot에 대한 승인입니다. 사용자가 다시 cursor를
    // 움직이거나 편집을 시작하면 risk context가 바뀌므로 pending confirmation을 무효화해야 합니다.
    let mut state = PlanningDraftEditorUiState::default();
    state.open_session(sample_session());
    state.insert_character('#');
    let _ = state.request_close();

    state.move_cursor_right();

    assert!(!state.is_close_confirmation_pending());
}

#[test]
fn delete_previous_word_removes_the_full_word_before_cursor() {
    // 학습 주석: Ctrl-W 계열 편집은 draft editor에서도 shell input과 비슷하게 동작해야 합니다.
    // 한 줄 안에서 직전 단어만 제거하고 cursor를 삭제 지점으로 되돌리는 계약을 고정합니다.
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
    // 학습 주석: word deletion은 단어 앞 공백을 정리하되 줄 경계를 넘어서 이전 line을 합치지
    // 않습니다. markdown/JSON draft에서 줄 구조를 우발적으로 깨지 않기 위한 편집 규칙입니다.
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
    // 학습 주석: editor scroll은 cursor를 보이게 해야 하지만, cursor가 viewport 안에 있는 동안
    // 매 이동마다 scroll을 다시 pinning하면 화면이 불필요하게 튑니다.
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
