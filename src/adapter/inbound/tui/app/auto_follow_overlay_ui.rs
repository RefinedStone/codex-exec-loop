/*
 * auto-follow overlay UI state는 ConversationViewModel의 auto-follow policy와 의도적으로 분리되어 있다.
 * Planning init SimpleReview는 operator가 max-auto-turns를 raw text로 편집하게 하므로,
 * 이 reducer는 `auto_follow_controls`가 commit을 승인하기 전까지 진행 중인 buffer가 runtime policy를 바꾸지 못하게 막는다.
 */

#[derive(Debug, Default)]
// inline max-auto-turns editor가 key stream을 소유하는 동안만 의미가 있는 screen-local state다.
pub(super) struct MaxAutoTurnsEditorState {
    // true이면 `auto_follow/controller`가 Enter, Esc, Backspace, text input을 global shortcut 대신 이 editor로 보낸다.
    pub is_editing: bool,
    // raw user input은 비어 있거나 임시로 invalid일 수 있다. commit validation은 `auto_follow_controls`가 맡는다.
    pub buffer: String,
}

#[derive(Debug, Default)]
// controller, runtime sync, planning review rendering이 공유하는 root overlay input state다.
pub(super) struct AutoFollowOverlayUiState {
    // planning simple-review max-auto-turns control은 이 buffer와 editing flag를 직접 읽어 inline editor를 그린다.
    pub max_auto_turns_editor: MaxAutoTurnsEditorState,
}

#[derive(Debug, Clone)]
/*
 * AutoFollowOverlayUiEvent는 presentation-owned editor state만 바꾼다.
 * 실제 policy change는 먼저 AutoFollowControlEvent를 통과하고, 성공한 control effect가 sync/commit acknowledgement로 다시 돌아온다.
 */
pub(super) enum AutoFollowOverlayUiEvent {
    // 새 conversation context는 editor를 닫고 buffer를 그 context의 canonical label로 교체한다.
    ContentReset { max_auto_turns: String },
    // 외부 policy sync는 operator가 편집 중이 아닐 때만 display buffer를 갱신한다.
    MaxAutoTurnsValueSynced { value: String },
    // editor open은 현재 policy label을 raw editing buffer의 기준값으로 복사한다.
    MaxAutoTurnsEditStarted { current_value: String },
    // commit acknowledgement는 editor를 닫고 `auto_follow_controls`가 승인한 canonical label을 보여 준다.
    MaxAutoTurnsEditCommitted { current_value: String },
    // cancel은 policy 변경 없이 editor를 닫고 caller가 넘긴 현재 label로 되돌린다.
    MaxAutoTurnsEditCanceled { current_value: String },
    // typing은 raw text만 추가한다. numeric/infinite validation은 commit 시점까지 의도적으로 미룬다.
    MaxAutoTurnsCharacterTyped { character: char },
    // backspace는 열린 buffer만 편집해 닫힌 overlay가 global Backspace behavior를 가로채지 않게 한다.
    MaxAutoTurnsBackspacePressed,
}

// overlay-only editor state의 pure reducer다. NativeTuiApp이 이 state와 conversation policy 사이의 bridge를 소유한다.
pub(super) fn reduce_auto_follow_overlay_ui(
    mut state: AutoFollowOverlayUiState,
    event: AutoFollowOverlayUiEvent,
) -> AutoFollowOverlayUiState {
    match event {
        AutoFollowOverlayUiEvent::ContentReset { max_auto_turns } => {
            // context reset은 새 draft/session의 canonical policy value를 반영하므로 진행 중인 text보다 우선한다.
            state.max_auto_turns_editor = MaxAutoTurnsEditorState {
                is_editing: false,
                buffer: max_auto_turns,
            };
        }
        AutoFollowOverlayUiEvent::MaxAutoTurnsValueSynced { value } => {
            // active typing을 runtime/control sync로 덮으면 uncommitted operator input을 버리게 되므로 닫힌 상태에서만 반영한다.
            if !state.max_auto_turns_editor.is_editing {
                state.max_auto_turns_editor.buffer = value;
            }
        }
        AutoFollowOverlayUiEvent::MaxAutoTurnsEditStarted { current_value } => {
            // edit 시작은 현재 policy label을 local editing baseline으로 snapshot한다.
            state.max_auto_turns_editor.is_editing = true;
            state.max_auto_turns_editor.buffer = current_value;
        }
        AutoFollowOverlayUiEvent::MaxAutoTurnsEditCommitted { current_value }
        | AutoFollowOverlayUiEvent::MaxAutoTurnsEditCanceled { current_value } => {
            // commit과 cancel은 이 reducer 안에서는 같은 UI shape다. editor를 닫고 buffer를 canonical label로 되돌린다.
            state.max_auto_turns_editor.is_editing = false;
            state.max_auto_turns_editor.buffer = current_value;
        }
        AutoFollowOverlayUiEvent::MaxAutoTurnsCharacterTyped { character } => {
            // close 이후 도착한 stale key event는 무시해 display-only buffer가 오염되지 않게 한다.
            if state.max_auto_turns_editor.is_editing {
                state.max_auto_turns_editor.buffer.push(character);
            }
        }
        AutoFollowOverlayUiEvent::MaxAutoTurnsBackspacePressed => {
            // 빈 buffer에서 `pop`은 자연스럽게 no-op이므로 필요한 guard는 editor ownership뿐이다.
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
        // content reset은 draft/session context-change contract다. editor를 닫고 새 label을 보여 준다.
        let state = AutoFollowOverlayUiState::default();

        let reduced = reduce_auto_follow_overlay_ui(
            state,
            AutoFollowOverlayUiEvent::ContentReset {
                max_auto_turns: "3".to_string(),
            },
        );

        assert_eq!(reduced.max_auto_turns_editor.buffer, "3");
        assert!(!reduced.max_auto_turns_editor.is_editing);
    }

    #[test]
    fn max_auto_turns_editing_updates_buffer_and_backspace() {
        // typing은 overlay state에만 남는다. `auto_follow_controls`가 commit하기 전에는 conversation policy를 건드리지 않는다.
        let state = AutoFollowOverlayUiState::default();

        let state = reduce_auto_follow_overlay_ui(
            state,
            AutoFollowOverlayUiEvent::MaxAutoTurnsEditStarted {
                current_value: "3".to_string(),
            },
        );
        let state = reduce_auto_follow_overlay_ui(
            state,
            AutoFollowOverlayUiEvent::MaxAutoTurnsCharacterTyped { character: '5' },
        );
        let reduced = reduce_auto_follow_overlay_ui(
            state,
            AutoFollowOverlayUiEvent::MaxAutoTurnsBackspacePressed,
        );

        assert_eq!(reduced.max_auto_turns_editor.buffer, "3");
        assert!(reduced.max_auto_turns_editor.is_editing);
    }

    #[test]
    fn max_auto_turns_commit_exits_edit_mode_and_syncs_value() {
        // commit은 control reducer가 값을 승인한 뒤에만 도착하므로 buffer는 canonical label과 같아야 한다.
        let state = AutoFollowOverlayUiState {
            max_auto_turns_editor: MaxAutoTurnsEditorState {
                is_editing: true,
                buffer: "5".to_string(),
            },
        };

        let reduced = reduce_auto_follow_overlay_ui(
            state,
            AutoFollowOverlayUiEvent::MaxAutoTurnsEditCommitted {
                current_value: "5".to_string(),
            },
        );

        assert_eq!(reduced.max_auto_turns_editor.buffer, "5");
        assert!(!reduced.max_auto_turns_editor.is_editing);
    }

    #[test]
    fn max_auto_turns_sync_does_not_override_active_edit_buffer() {
        // editor가 key stream을 소유하는 동안 runtime/control sync가 uncommitted operator input을 지우면 안 된다.
        let state = AutoFollowOverlayUiState {
            max_auto_turns_editor: MaxAutoTurnsEditorState {
                is_editing: true,
                buffer: "working".to_string(),
            },
        };

        let reduced = reduce_auto_follow_overlay_ui(
            state,
            AutoFollowOverlayUiEvent::MaxAutoTurnsValueSynced {
                value: "3".to_string(),
            },
        );

        assert_eq!(reduced.max_auto_turns_editor.buffer, "working");
    }
}
