use ratatui::widgets::ListState;

#[derive(Debug, Default)]
pub(super) struct StopKeywordEditorState {
    pub is_editing: bool,
    pub buffer: String,
}

#[derive(Debug, Default)]
pub(super) struct MaxAutoTurnsEditorState {
    pub is_editing: bool,
    pub buffer: String,
}

#[derive(Debug, Default)]
pub(super) struct FollowupOverlayUiState {
    pub preview_scroll: u16,
    pub list_state: ListState,
    pub stop_keyword_editor: StopKeywordEditorState,
    pub max_auto_turns_editor: MaxAutoTurnsEditorState,
}

#[derive(Debug, Clone)]
pub(super) enum FollowupOverlayUiEvent {
    OverlayShown {
        stop_keyword: String,
        max_auto_turns: String,
    },
    TemplateChanged,
    ContentReset {
        stop_keyword: String,
        max_auto_turns: String,
    },
    PreviewScrolled {
        delta: i32,
    },
    MaxAutoTurnsValueSynced {
        value: String,
    },
    MaxAutoTurnsEditStarted {
        current_value: String,
    },
    MaxAutoTurnsEditCommitted {
        current_value: String,
    },
    MaxAutoTurnsEditCanceled {
        current_value: String,
    },
    MaxAutoTurnsCharacterTyped {
        character: char,
    },
    MaxAutoTurnsBackspacePressed,
    StopKeywordValueSynced {
        value: String,
    },
    StopKeywordEditStarted {
        current_value: String,
    },
    StopKeywordEditCommitted {
        current_value: String,
    },
    StopKeywordEditCanceled {
        current_value: String,
    },
    StopKeywordCharacterTyped {
        character: char,
    },
    StopKeywordBackspacePressed,
}

pub(super) fn reduce_followup_overlay_ui(
    mut state: FollowupOverlayUiState,
    event: FollowupOverlayUiEvent,
) -> FollowupOverlayUiState {
    match event {
        FollowupOverlayUiEvent::OverlayShown {
            stop_keyword,
            max_auto_turns,
        }
        | FollowupOverlayUiEvent::ContentReset {
            stop_keyword,
            max_auto_turns,
        } => {
            state.preview_scroll = 0;
            state.list_state = ListState::default();
            state.stop_keyword_editor = StopKeywordEditorState {
                is_editing: false,
                buffer: stop_keyword,
            };
            state.max_auto_turns_editor = MaxAutoTurnsEditorState {
                is_editing: false,
                buffer: max_auto_turns,
            };
        }
        FollowupOverlayUiEvent::TemplateChanged => {
            state.preview_scroll = 0;
        }
        FollowupOverlayUiEvent::PreviewScrolled { delta } => {
            let amount = delta.unsigned_abs().min(u16::MAX as u32) as u16;
            if delta.is_negative() {
                state.preview_scroll = state.preview_scroll.saturating_sub(amount);
            } else {
                state.preview_scroll = state.preview_scroll.saturating_add(amount);
            }
        }
        FollowupOverlayUiEvent::MaxAutoTurnsValueSynced { value } => {
            if !state.max_auto_turns_editor.is_editing {
                state.max_auto_turns_editor.buffer = value;
            }
        }
        FollowupOverlayUiEvent::MaxAutoTurnsEditStarted { current_value } => {
            state.max_auto_turns_editor.is_editing = true;
            state.max_auto_turns_editor.buffer = current_value;
            state.stop_keyword_editor.is_editing = false;
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
        FollowupOverlayUiEvent::StopKeywordValueSynced { value } => {
            if !state.stop_keyword_editor.is_editing {
                state.stop_keyword_editor.buffer = value;
            }
        }
        FollowupOverlayUiEvent::StopKeywordEditStarted { current_value } => {
            state.stop_keyword_editor.is_editing = true;
            state.stop_keyword_editor.buffer = current_value;
            state.max_auto_turns_editor.is_editing = false;
        }
        FollowupOverlayUiEvent::StopKeywordEditCommitted { current_value }
        | FollowupOverlayUiEvent::StopKeywordEditCanceled { current_value } => {
            state.stop_keyword_editor.is_editing = false;
            state.stop_keyword_editor.buffer = current_value;
        }
        FollowupOverlayUiEvent::StopKeywordCharacterTyped { character } => {
            if state.stop_keyword_editor.is_editing {
                state.stop_keyword_editor.buffer.push(character);
            }
        }
        FollowupOverlayUiEvent::StopKeywordBackspacePressed => {
            if state.stop_keyword_editor.is_editing {
                state.stop_keyword_editor.buffer.pop();
            }
        }
    }

    state
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn template_changed_resets_preview_scroll() {
        let state = FollowupOverlayUiState {
            preview_scroll: 12,
            ..Default::default()
        };

        let reduced = reduce_followup_overlay_ui(state, FollowupOverlayUiEvent::TemplateChanged);

        assert_eq!(reduced.preview_scroll, 0);
    }

    #[test]
    fn preview_scrolled_saturates_at_zero() {
        let state = FollowupOverlayUiState {
            preview_scroll: 3,
            ..Default::default()
        };

        let reduced = reduce_followup_overlay_ui(
            state,
            FollowupOverlayUiEvent::PreviewScrolled { delta: -12 },
        );

        assert_eq!(reduced.preview_scroll, 0);
    }

    #[test]
    fn preview_scrolled_moves_forward() {
        let state = FollowupOverlayUiState::default();

        let reduced =
            reduce_followup_overlay_ui(state, FollowupOverlayUiEvent::PreviewScrolled { delta: 6 });

        assert_eq!(reduced.preview_scroll, 6);
    }

    #[test]
    fn overlay_shown_resets_list_state() {
        let mut state = FollowupOverlayUiState::default();
        state.list_state.select(Some(3));

        let reduced = reduce_followup_overlay_ui(
            state,
            FollowupOverlayUiEvent::OverlayShown {
                stop_keyword: "AUTO_STOP".to_string(),
                max_auto_turns: "3".to_string(),
            },
        );

        assert_eq!(reduced.list_state.selected(), None);
        assert_eq!(reduced.stop_keyword_editor.buffer, "AUTO_STOP");
        assert!(!reduced.stop_keyword_editor.is_editing);
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
            ..Default::default()
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
            ..Default::default()
        };

        let reduced = reduce_followup_overlay_ui(
            state,
            FollowupOverlayUiEvent::MaxAutoTurnsValueSynced {
                value: "3".to_string(),
            },
        );

        assert_eq!(reduced.max_auto_turns_editor.buffer, "working");
    }

    #[test]
    fn stop_keyword_editing_updates_buffer_and_backspace() {
        let state = FollowupOverlayUiState::default();

        let state = reduce_followup_overlay_ui(
            state,
            FollowupOverlayUiEvent::StopKeywordEditStarted {
                current_value: "AUTO_STOP".to_string(),
            },
        );
        let state = reduce_followup_overlay_ui(
            state,
            FollowupOverlayUiEvent::StopKeywordCharacterTyped { character: '2' },
        );
        let reduced =
            reduce_followup_overlay_ui(state, FollowupOverlayUiEvent::StopKeywordBackspacePressed);

        assert_eq!(reduced.stop_keyword_editor.buffer, "AUTO_STOP");
        assert!(reduced.stop_keyword_editor.is_editing);
    }

    #[test]
    fn stop_keyword_commit_exits_edit_mode_and_syncs_value() {
        let state = FollowupOverlayUiState {
            stop_keyword_editor: StopKeywordEditorState {
                is_editing: true,
                buffer: "AUTO_STOP_2".to_string(),
            },
            ..Default::default()
        };

        let reduced = reduce_followup_overlay_ui(
            state,
            FollowupOverlayUiEvent::StopKeywordEditCommitted {
                current_value: "AUTO_STOP_2".to_string(),
            },
        );

        assert_eq!(reduced.stop_keyword_editor.buffer, "AUTO_STOP_2");
        assert!(!reduced.stop_keyword_editor.is_editing);
    }

    #[test]
    fn stop_keyword_sync_does_not_override_active_edit_buffer() {
        let state = FollowupOverlayUiState {
            stop_keyword_editor: StopKeywordEditorState {
                is_editing: true,
                buffer: "WORKING".to_string(),
            },
            ..Default::default()
        };

        let reduced = reduce_followup_overlay_ui(
            state,
            FollowupOverlayUiEvent::StopKeywordValueSynced {
                value: "AUTO_STOP".to_string(),
            },
        );

        assert_eq!(reduced.stop_keyword_editor.buffer, "WORKING");
    }
}
