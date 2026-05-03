/*
 * Follow-up overlay UI state is deliberately separate from the ConversationViewModel
 * auto-follow policy. Planning init SimpleReview lets the operator edit max-auto-turns
 * as raw text; this reducer keeps that in-progress buffer from mutating runtime policy
 * until the control reducer accepts a commit.
 */

#[derive(Debug, Default)]
// Screen-local state for the inline max-auto-turns editor.
pub(super) struct MaxAutoTurnsEditorState {
    // When true, followup/controller routes Enter, Esc, Backspace, and text input here instead of global shortcuts.
    pub is_editing: bool,
    // Raw user input is allowed to be empty or temporarily invalid; validation belongs to followup_controls on commit.
    pub buffer: String,
}

#[derive(Debug, Default)]
// Root overlay input state shared by controller, runtime sync, and planning review rendering.
pub(super) struct FollowupOverlayUiState {
    // The planning simple-review max-auto-turns control reads this buffer and editing flag directly.
    pub max_auto_turns_editor: MaxAutoTurnsEditorState,
}

#[derive(Debug, Clone)]
/*
 * FollowupOverlayUiEvent changes only presentation-owned editor state. Policy changes
 * flow through FollowupControlEvent first; successful control effects come back here
 * as sync/commit acknowledgements.
 */
pub(super) enum FollowupOverlayUiEvent {
    // New conversation context closes the editor and replaces the buffer with that context's canonical label.
    ContentReset { max_auto_turns: String },
    // External policy sync updates the display buffer only while the operator is not actively editing.
    MaxAutoTurnsValueSynced { value: String },
    // Opening the editor copies the current policy label into the raw editing buffer.
    MaxAutoTurnsEditStarted { current_value: String },
    // Commit acknowledgement closes the editor and shows the canonical label accepted by followup_controls.
    MaxAutoTurnsEditCommitted { current_value: String },
    // Cancel closes the editor without policy changes and restores the caller-provided current label.
    MaxAutoTurnsEditCanceled { current_value: String },
    // Typing appends raw text; numeric/infinite validation is intentionally deferred until commit.
    MaxAutoTurnsCharacterTyped { character: char },
    // Backspace edits only the open buffer so closed overlays do not steal global Backspace behavior.
    MaxAutoTurnsBackspacePressed,
}

// Pure reducer for overlay-only editor state. NativeTuiApp owns the bridge between this state and conversation policy.
pub(super) fn reduce_followup_overlay_ui(
    mut state: FollowupOverlayUiState,
    event: FollowupOverlayUiEvent,
) -> FollowupOverlayUiState {
    match event {
        FollowupOverlayUiEvent::ContentReset { max_auto_turns } => {
            // Context reset outranks in-progress text because a new draft/session has a different canonical policy value.
            state.max_auto_turns_editor = MaxAutoTurnsEditorState {
                is_editing: false,
                buffer: max_auto_turns,
            };
        }
        FollowupOverlayUiEvent::MaxAutoTurnsValueSynced { value } => {
            // Do not overwrite active typing with runtime/control sync; that would discard uncommitted operator input.
            if !state.max_auto_turns_editor.is_editing {
                state.max_auto_turns_editor.buffer = value;
            }
        }
        FollowupOverlayUiEvent::MaxAutoTurnsEditStarted { current_value } => {
            // Starting an edit snapshots the current policy label as the local editing baseline.
            state.max_auto_turns_editor.is_editing = true;
            state.max_auto_turns_editor.buffer = current_value;
        }
        FollowupOverlayUiEvent::MaxAutoTurnsEditCommitted { current_value }
        | FollowupOverlayUiEvent::MaxAutoTurnsEditCanceled { current_value } => {
            // Commit and cancel have the same UI shape here: editor closes and buffer returns to a canonical label.
            state.max_auto_turns_editor.is_editing = false;
            state.max_auto_turns_editor.buffer = current_value;
        }
        FollowupOverlayUiEvent::MaxAutoTurnsCharacterTyped { character } => {
            // Stale key events after close are ignored so display-only buffer cannot be polluted.
            if state.max_auto_turns_editor.is_editing {
                state.max_auto_turns_editor.buffer.push(character);
            }
        }
        FollowupOverlayUiEvent::MaxAutoTurnsBackspacePressed => {
            // `pop` naturally no-ops on an empty buffer; the only guard needed is editor ownership.
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
        // Content reset is the draft/session context-change contract: close the editor and show the new label.
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
        // Typing stays in overlay state only; the conversation policy is not touched until followup_controls commits.
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
        // Commit arrives only after the control reducer accepts the value, so the buffer should match its canonical label.
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
        // Runtime/control sync must not erase uncommitted operator input while the editor owns the key stream.
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
