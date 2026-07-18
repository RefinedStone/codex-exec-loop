/*
 * auto-follow overlay UI state는 ConversationViewModel의 auto-follow policy와 의도적으로 분리되어 있다.
 * Planning init SimpleReview는 operator가 max-auto-turns를 raw text로 편집하게 하므로,
 * 이 reducer는 `auto_follow_controls`가 commit을 승인하기 전까지 진행 중인 buffer가 runtime policy를 바꾸지 못하게 막는다.
 */

#[derive(Debug, Default)]
// 닫힌 editor는 canonical budget을 복제하지 않고, 열린 동안의 raw draft만 보관한다.
pub(super) struct AutoFollowOverlayUiState {
    max_auto_turns_edit_buffer: Option<String>,
}

impl AutoFollowOverlayUiState {
    pub(super) fn max_auto_turns_edit_buffer(&self) -> Option<&str> {
        self.max_auto_turns_edit_buffer.as_deref()
    }
}

#[derive(Debug, Clone)]
/*
 * AutoFollowOverlayUiEvent는 presentation-owned editor state만 바꾼다.
 * 실제 policy change는 먼저 AutoFollowControlEvent를 통과하며, 성공한 commit이나 context reset은 draft만 닫는다.
 */
pub(super) enum AutoFollowOverlayUiEvent {
    // editor open은 현재 policy label을 raw editing buffer의 기준값으로 복사한다.
    EditStarted { current_value: String },
    // commit, cancel, conversation 전환은 local draft만 버린다. 닫힌 copy는 canonical policy를 직접 읽는다.
    EditFinished,
    // typing은 raw text만 추가한다. numeric/infinite validation은 commit 시점까지 의도적으로 미룬다.
    CharacterTyped { character: char },
    // backspace는 열린 draft만 편집해 닫힌 overlay가 global Backspace behavior를 가로채지 않게 한다.
    BackspacePressed,
}

// overlay-only editor state의 pure reducer다. NativeTuiApp이 이 state와 conversation policy 사이의 bridge를 소유한다.
pub(super) fn reduce_auto_follow_overlay_ui(
    mut state: AutoFollowOverlayUiState,
    event: AutoFollowOverlayUiEvent,
) -> AutoFollowOverlayUiState {
    match event {
        AutoFollowOverlayUiEvent::EditStarted { current_value } => {
            // edit 시작은 현재 policy label을 local editing baseline으로 snapshot한다.
            state.max_auto_turns_edit_buffer = Some(current_value);
        }
        AutoFollowOverlayUiEvent::EditFinished => {
            state.max_auto_turns_edit_buffer = None;
        }
        AutoFollowOverlayUiEvent::CharacterTyped { character } => {
            if let Some(buffer) = &mut state.max_auto_turns_edit_buffer {
                buffer.push(character);
            }
        }
        AutoFollowOverlayUiEvent::BackspacePressed => {
            if let Some(buffer) = &mut state.max_auto_turns_edit_buffer {
                buffer.pop();
            }
        }
    }

    state
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn closed_editor_does_not_mirror_the_canonical_budget() {
        let state = AutoFollowOverlayUiState::default();

        assert_eq!(state.max_auto_turns_edit_buffer(), None);
    }

    #[test]
    fn max_auto_turns_editing_updates_buffer_and_backspace() {
        // typing은 overlay state에만 남는다. `auto_follow_controls`가 commit하기 전에는 conversation policy를 건드리지 않는다.
        let state = AutoFollowOverlayUiState::default();

        let state = reduce_auto_follow_overlay_ui(
            state,
            AutoFollowOverlayUiEvent::EditStarted {
                current_value: "3".to_string(),
            },
        );
        let state = reduce_auto_follow_overlay_ui(
            state,
            AutoFollowOverlayUiEvent::CharacterTyped { character: '5' },
        );
        let reduced =
            reduce_auto_follow_overlay_ui(state, AutoFollowOverlayUiEvent::BackspacePressed);

        assert_eq!(reduced.max_auto_turns_edit_buffer(), Some("3"));
    }

    #[test]
    fn max_auto_turns_finish_drops_the_active_draft() {
        let state = AutoFollowOverlayUiState {
            max_auto_turns_edit_buffer: Some("5".to_string()),
        };

        let reduced = reduce_auto_follow_overlay_ui(state, AutoFollowOverlayUiEvent::EditFinished);

        assert_eq!(reduced.max_auto_turns_edit_buffer(), None);
    }

    #[test]
    fn stale_typing_and_backspace_do_not_create_a_closed_draft() {
        let state = reduce_auto_follow_overlay_ui(
            AutoFollowOverlayUiState::default(),
            AutoFollowOverlayUiEvent::CharacterTyped { character: '5' },
        );
        let reduced =
            reduce_auto_follow_overlay_ui(state, AutoFollowOverlayUiEvent::BackspacePressed);

        assert_eq!(reduced.max_auto_turns_edit_buffer(), None);
    }
}
