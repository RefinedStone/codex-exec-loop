use super::{PlanningDraftEditorCloseRequest, PlanningDraftEditorUiState};
use crate::application::service::planning::{PlanningDraftEditorFile, PlanningDraftEditorSession};
use crate::domain::planning::{
    PlanningFileKind, PlanningValidationReport, PlanningValidationSeverity,
};

fn sample_session() -> PlanningDraftEditorSession {
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
    let mut state = PlanningDraftEditorUiState::default();
    state.open_session(sample_session());
    state.insert_character('#');
    let _ = state.request_close();

    state.move_cursor_right();

    assert!(!state.is_close_confirmation_pending());
}

#[test]
fn delete_previous_word_removes_the_full_word_before_cursor() {
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
