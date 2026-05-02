/*
학습 주석: followup overlay UI state는 conversation model의 실제 auto-follow 설정과 분리된 화면 전용
buffer입니다. planning init SimpleReview 안에서 max auto turns를 편집할 때, 사용자가 입력 중인 임시 문자열이
저장 전까지 conversation state를 덮어쓰지 않도록 이 reducer가 작은 UI state machine 역할을 합니다.
*/

#[derive(Debug, Default)]
// 학습 주석: MaxAutoTurnsEditorState는 auto-follow 최대 turn 수 inline editor의 화면 상태입니다.
// followup controller는 키 입력을 이 state로 보내고, followup control reducer는 저장 시점에만 실제 값을 검증/반영합니다.
pub(super) struct MaxAutoTurnsEditorState {
    // 학습 주석: is_editing은 현재 키 입력을 editor가 소유하는지 알려 주는 flag입니다. controller의
    // handle_max_auto_turns_editor_key가 이 값을 보고 Enter/Esc/Backspace/문자 입력을 일반 단축키에서 분리합니다.
    pub is_editing: bool,
    // 학습 주석: buffer는 사용자가 입력 중인 raw 문자열입니다. 중간 상태로 빈 값이나 아직 숫자로 파싱되지 않는
    // 값을 허용해야 하므로 conversation의 확정 max_auto_turns와 별도로 유지합니다.
    pub buffer: String,
}

#[derive(Debug, Default)]
// 학습 주석: FollowupOverlayUiState는 followup 관련 overlay 입력 상태의 root입니다. 지금은 max_auto_turns
// editor 하나만 갖지만, NativeTuiApp에 한 필드로 들어가 followup controller, runtime sync, planning review copy가 공유합니다.
pub(super) struct FollowupOverlayUiState {
    // 학습 주석: planning init simple review 화면의 "max auto turns" inline control이 이 editor state를 읽습니다.
    // rendering은 buffer/is_editing을 표시하고, controller는 여기에만 글자를 쌓은 뒤 save event를 보냅니다.
    pub max_auto_turns_editor: MaxAutoTurnsEditorState,
}

#[derive(Debug, Clone)]
// 학습 주석: FollowupOverlayUiEvent는 화면 상태만 바꾸는 event입니다. 실제 auto-follow 정책 변경은
// FollowupControlEvent가 담당하고, 이 enum은 editor 열기/닫기/문자 입력/외부 값 동기화만 표현합니다.
pub(super) enum FollowupOverlayUiEvent {
    // 학습 주석: ContentReset은 새 draft/session load처럼 conversation context가 바뀌었을 때 editor를 닫고
    // 현재 확정 max_auto_turns label로 화면 buffer를 다시 맞춥니다.
    ContentReset { max_auto_turns: String },
    // 학습 주석: MaxAutoTurnsValueSynced는 control reducer가 실제 값을 바꾼 뒤 UI buffer를 따라오게 하는 event입니다.
    // 단, 사용자가 편집 중이면 아래 reducer가 무시해 active input을 보호합니다.
    MaxAutoTurnsValueSynced { value: String },
    // 학습 주석: MaxAutoTurnsEditStarted는 현재 확정 값을 buffer로 복사하면서 키 입력 소유권을 editor로 넘깁니다.
    MaxAutoTurnsEditStarted { current_value: String },
    // 학습 주석: MaxAutoTurnsEditCommitted는 control reducer가 저장에 성공했음을 UI에 알려 editor를 닫고
    // canonical label로 buffer를 맞추는 commit acknowledgement입니다.
    MaxAutoTurnsEditCommitted { current_value: String },
    // 학습 주석: MaxAutoTurnsEditCanceled는 실제 설정을 바꾸지 않고 editor를 닫습니다. current_value로
    // buffer를 되돌려 다음에 열 때 취소된 임시 입력이 남지 않게 합니다.
    MaxAutoTurnsEditCanceled { current_value: String },
    // 학습 주석: MaxAutoTurnsCharacterTyped는 편집 중 raw buffer에 문자를 추가합니다. 숫자 검증은 저장
    // 이벤트를 처리하는 control reducer 쪽에 두어 typing 중간 상태를 막지 않습니다.
    MaxAutoTurnsCharacterTyped { character: char },
    // 학습 주석: MaxAutoTurnsBackspacePressed는 편집 중 buffer 마지막 문자를 제거합니다. 닫힌 editor에서는
    // 상위 key router의 Backspace 의미를 침범하지 않도록 reducer가 무시합니다.
    MaxAutoTurnsBackspacePressed,
}

// 학습 주석: reduce_followup_overlay_ui는 순수 reducer입니다. NativeTuiApp::dispatch_followup_overlay_ui가
// state를 take해 이 함수에 넣고 새 state를 돌려받으므로, controller/runtime/presentation 사이의 UI state 변경 규칙이 한곳에 모입니다.
pub(super) fn reduce_followup_overlay_ui(
    mut state: FollowupOverlayUiState,
    event: FollowupOverlayUiEvent,
) -> FollowupOverlayUiState {
    match event {
        FollowupOverlayUiEvent::ContentReset { max_auto_turns } => {
            // 학습 주석: context reset은 편집 중 입력보다 우선합니다. 새 conversation/session의 확정 값을
            // 보여 주어야 하므로 editor를 닫고 buffer를 reset payload로 교체합니다.
            state.max_auto_turns_editor = MaxAutoTurnsEditorState {
                is_editing: false,
                buffer: max_auto_turns,
            };
        }
        FollowupOverlayUiEvent::MaxAutoTurnsValueSynced { value } => {
            // 학습 주석: 외부 control state가 바뀌어도 active edit buffer를 덮으면 사용자가 타이핑하던 값이
            // 사라집니다. 그래서 editor가 닫힌 상태에서만 display buffer를 최신 확정 값으로 동기화합니다.
            if !state.max_auto_turns_editor.is_editing {
                state.max_auto_turns_editor.buffer = value;
            }
        }
        FollowupOverlayUiEvent::MaxAutoTurnsEditStarted { current_value } => {
            // 학습 주석: edit 시작은 현재 확정 label을 editing buffer의 기준점으로 삼습니다.
            // 이후 문자/Backspace event는 이 buffer에만 반영되고 실제 auto-follow 설정은 아직 유지됩니다.
            state.max_auto_turns_editor.is_editing = true;
            state.max_auto_turns_editor.buffer = current_value;
        }
        FollowupOverlayUiEvent::MaxAutoTurnsEditCommitted { current_value }
        | FollowupOverlayUiEvent::MaxAutoTurnsEditCanceled { current_value } => {
            // 학습 주석: commit과 cancel 모두 editor를 닫고 확정 label로 buffer를 맞춘다는 UI 결과는 같습니다.
            // 차이는 이 reducer 밖에서 발생합니다. commit은 control reducer가 값을 저장한 뒤 들어오고,
            // cancel은 저장 없이 현재 label을 다시 주입합니다.
            state.max_auto_turns_editor.is_editing = false;
            state.max_auto_turns_editor.buffer = current_value;
        }
        FollowupOverlayUiEvent::MaxAutoTurnsCharacterTyped { character } => {
            // 학습 주석: character event는 editing 중일 때만 buffer를 수정합니다. stale key event가 들어와도
            // 닫힌 editor가 표시 값을 오염시키지 않게 하는 방어선입니다.
            if state.max_auto_turns_editor.is_editing {
                state.max_auto_turns_editor.buffer.push(character);
            }
        }
        FollowupOverlayUiEvent::MaxAutoTurnsBackspacePressed => {
            // 학습 주석: Backspace도 editing 중일 때만 buffer에 적용합니다. 비어 있는 buffer에서 pop은 None을
            // 돌리고 끝나므로 별도 길이 검사는 필요 없습니다.
            if state.max_auto_turns_editor.is_editing {
                state.max_auto_turns_editor.buffer.pop();
            }
        }
    }

    state
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn content_reset_syncs_max_auto_turns() {
        // 학습 주석: content reset은 새 draft/session context를 연 직후의 동기화 계약입니다. editor가 닫히고
        // buffer가 현재 conversation label로 바뀌어야 planning review가 stale 값을 보여 주지 않습니다.
        let state = FollowupOverlayUiState::default();

        let reduced = reduce_followup_overlay_ui(
            state,
            FollowupOverlayUiEvent::ContentReset {
                max_auto_turns: "3".to_string(),
            },
        );

        assert_eq!(reduced.max_auto_turns_editor.buffer, "3");
        assert!(!reduced.max_auto_turns_editor.is_editing);
    }

    #[test]
    fn max_auto_turns_editing_updates_buffer_and_backspace() {
        // 학습 주석: 이 테스트는 typing이 control reducer를 거치지 않고 overlay buffer에만 쌓이는지 확인합니다.
        // 사용자는 저장 전까지 자유롭게 입력하고 지울 수 있어야 합니다.
        let state = FollowupOverlayUiState::default();

        let state = reduce_followup_overlay_ui(
            state,
            FollowupOverlayUiEvent::MaxAutoTurnsEditStarted {
                current_value: "3".to_string(),
            },
        );
        let state = reduce_followup_overlay_ui(
            state,
            FollowupOverlayUiEvent::MaxAutoTurnsCharacterTyped { character: '5' },
        );
        let reduced =
            reduce_followup_overlay_ui(state, FollowupOverlayUiEvent::MaxAutoTurnsBackspacePressed);

        assert_eq!(reduced.max_auto_turns_editor.buffer, "3");
        assert!(reduced.max_auto_turns_editor.is_editing);
    }

    #[test]
    fn max_auto_turns_commit_exits_edit_mode_and_syncs_value() {
        // 학습 주석: commit event는 control reducer가 저장을 받아들인 뒤 UI에 돌아오는 acknowledgement입니다.
        // 따라서 editor는 닫히고 buffer는 저장된 canonical label과 일치해야 합니다.
        let state = FollowupOverlayUiState {
            max_auto_turns_editor: MaxAutoTurnsEditorState {
                is_editing: true,
                buffer: "5".to_string(),
            },
        };

        let reduced = reduce_followup_overlay_ui(
            state,
            FollowupOverlayUiEvent::MaxAutoTurnsEditCommitted {
                current_value: "5".to_string(),
            },
        );

        assert_eq!(reduced.max_auto_turns_editor.buffer, "5");
        assert!(!reduced.max_auto_turns_editor.is_editing);
    }

    #[test]
    fn max_auto_turns_sync_does_not_override_active_edit_buffer() {
        // 학습 주석: runtime/control layer가 최신 값을 sync하더라도 편집 중 buffer를 덮어쓰면 사용자의
        // 미저장 입력이 사라집니다. 이 테스트는 active editor가 외부 sync로부터 격리되는 계약을 고정합니다.
        let state = FollowupOverlayUiState {
            max_auto_turns_editor: MaxAutoTurnsEditorState {
                is_editing: true,
                buffer: "working".to_string(),
            },
        };

        let reduced = reduce_followup_overlay_ui(
            state,
            FollowupOverlayUiEvent::MaxAutoTurnsValueSynced {
                value: "3".to_string(),
            },
        );

        assert_eq!(reduced.max_auto_turns_editor.buffer, "working");
    }
}
