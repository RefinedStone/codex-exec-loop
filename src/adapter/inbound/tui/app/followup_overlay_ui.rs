#[derive(Debug, Default)]
pub(super) struct MaxAutoTurnsEditorState {
    pub is_editing: bool,
    pub buffer: String,
}

#[derive(Debug, Default)]
pub(super) struct FollowupOverlayUiState {
    pub max_auto_turns_editor: MaxAutoTurnsEditorState,
}

#[derive(Debug, Clone)]
pub(super) enum FollowupOverlayUiEvent {
    ContentReset { max_auto_turns: String },
    MaxAutoTurnsValueSynced { value: String },
    MaxAutoTurnsEditStarted { current_value: String },
    MaxAutoTurnsEditCommitted { current_value: String },
    MaxAutoTurnsEditCanceled { current_value: String },
    MaxAutoTurnsCharacterTyped { character: char },
    MaxAutoTurnsBackspacePressed,
}

pub(super) fn reduce_followup_overlay_ui(
    mut state: FollowupOverlayUiState,
    event: FollowupOverlayUiEvent,
) -> FollowupOverlayUiState {
    match event {
        FollowupOverlayUiEvent::ContentReset { max_auto_turns } => {
            state.max_auto_turns_editor = MaxAutoTurnsEditorState {
                is_editing: false,
                buffer: max_auto_turns,
            };
        }
        FollowupOverlayUiEvent::MaxAutoTurnsValueSynced { value } => {
            if !state.max_auto_turns_editor.is_editing {
                state.max_auto_turns_editor.buffer = value;
            }
        }
        FollowupOverlayUiEvent::MaxAutoTurnsEditStarted { current_value } => {
            state.max_auto_turns_editor.is_editing = true;
            state.max_auto_turns_editor.buffer = current_value;
        }
        FollowupOverlayUiEvent::MaxAutoTurnsEditCommitted { current_value }
        | FollowupOverlayUiEvent::MaxAutoTurnsEditCanceled { current_value } => {
            state.max_auto_turns_editor.is_editing = false;
            state.max_auto_turns_editor.buffer = current_value;
        }
        FollowupOverlayUiEvent::MaxAutoTurnsCharacterTyped { character } => {
            if state.max_auto_turns_editor.is_editing {
                state.max_auto_turns_editor.buffer.push(character);
            }
        }
        FollowupOverlayUiEvent::MaxAutoTurnsBackspacePressed => {
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
