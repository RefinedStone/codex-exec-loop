use super::{ConversationViewModel, InlineShellCommand};

#[derive(Debug, Clone)]
pub(super) enum ConversationInputEvent {
    CharacterTyped { character: char },
    NewlineInserted,
    BackspacePressed,
    PreviousWordDeleted,
    InputCleared,
    InlineCommandPaletteSelectionMoved { delta: isize },
    InlineCommandPaletteDismissed,
    InlineCommandPaletteCommandInserted { command: InlineShellCommand },
    StartupSubmitArmed { status_text: String },
    StartupSubmitDisarmed { status_text: Option<String> },
    StatusMessageShown { status_text: String },
}

#[derive(Debug, Clone)]
pub(super) struct ConversationInputReduction {
    pub state: ConversationViewModel,
}

pub(super) fn reduce_conversation_input(
    mut state: ConversationViewModel,
    event: ConversationInputEvent,
) -> ConversationInputReduction {
    match event {
        ConversationInputEvent::CharacterTyped { character } => {
            modify_input_buffer_and_sync(&mut state, |buffer| buffer.push(character));
        }
        ConversationInputEvent::NewlineInserted => {
            modify_input_buffer_and_sync(&mut state, |buffer| buffer.push('\n'));
        }
        ConversationInputEvent::BackspacePressed => {
            modify_input_buffer_and_sync(&mut state, |buffer| {
                buffer.pop();
            });
        }
        ConversationInputEvent::PreviousWordDeleted => {
            modify_input_buffer_and_sync(&mut state, delete_previous_word);
        }
        ConversationInputEvent::InputCleared => {
            modify_input_buffer_and_sync(&mut state, String::clear);
        }
        ConversationInputEvent::InlineCommandPaletteSelectionMoved { delta } => {
            state.move_inline_shell_command_palette_selection(delta);
        }
        ConversationInputEvent::InlineCommandPaletteDismissed => {
            state.dismiss_inline_shell_command_palette();
        }
        ConversationInputEvent::InlineCommandPaletteCommandInserted { command } => {
            clear_startup_submit_after_input_change(&mut state);
            state.insert_inline_shell_command_completion(command);
        }
        ConversationInputEvent::StartupSubmitArmed { status_text } => {
            state.arm_startup_submit();
            state.status_text = status_text;
        }
        ConversationInputEvent::StartupSubmitDisarmed { status_text } => {
            if state.clear_startup_submit()
                && let Some(status_text) = status_text
            {
                state.status_text = status_text;
            }
        }
        ConversationInputEvent::StatusMessageShown { status_text } => {
            state.status_text = status_text;
        }
    }

    ConversationInputReduction { state }
}

fn modify_input_buffer_and_sync(
    state: &mut ConversationViewModel,
    modifier: impl FnOnce(&mut String),
) {
    clear_startup_submit_after_input_change(state);
    modifier(&mut state.input_buffer);
    state.sync_inline_shell_command_palette();
}

fn delete_previous_word(buffer: &mut String) {
    if buffer.is_empty() {
        return;
    }

    let trimmed = buffer.trim_end_matches(|character: char| character.is_whitespace());
    let word_start = trimmed
        .rfind(|character: char| character.is_whitespace())
        .map(|index| index + 1)
        .unwrap_or(0);
    buffer.truncate(word_start);
}

fn clear_startup_submit_after_input_change(state: &mut ConversationViewModel) {
    if state.clear_startup_submit() {
        state.status_text = "queued startup send canceled after input changed".to_string();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn character_typed_appends_to_input_buffer() {
        let state = ConversationViewModel::new_draft("/tmp/root".to_string());

        let reduced = reduce_conversation_input(
            state,
            ConversationInputEvent::CharacterTyped { character: 'a' },
        );

        assert_eq!(reduced.state.input_buffer, "a");
    }

    #[test]
    fn backspace_pressed_removes_last_character() {
        let mut state = ConversationViewModel::new_draft("/tmp/root".to_string());
        state.input_buffer = "draft".to_string();

        let reduced = reduce_conversation_input(state, ConversationInputEvent::BackspacePressed);

        assert_eq!(reduced.state.input_buffer, "draf");
    }

    #[test]
    fn newline_inserted_adds_line_break() {
        let mut state = ConversationViewModel::new_draft("/tmp/root".to_string());
        state.input_buffer = "draft".to_string();

        let reduced = reduce_conversation_input(state, ConversationInputEvent::NewlineInserted);

        assert_eq!(reduced.state.input_buffer, "draft\n");
    }

    #[test]
    fn previous_word_deleted_removes_last_word() {
        let mut state = ConversationViewModel::new_draft("/tmp/root".to_string());
        state.input_buffer = "ship this next".to_string();

        let reduced = reduce_conversation_input(state, ConversationInputEvent::PreviousWordDeleted);

        assert_eq!(reduced.state.input_buffer, "ship this ");
    }

    #[test]
    fn previous_word_deleted_trims_trailing_space_before_removing_last_word() {
        let mut state = ConversationViewModel::new_draft("/tmp/root".to_string());
        state.input_buffer = "ship this   ".to_string();

        let reduced = reduce_conversation_input(state, ConversationInputEvent::PreviousWordDeleted);

        assert_eq!(reduced.state.input_buffer, "ship ");
    }

    #[test]
    fn previous_word_deleted_respects_newline_boundaries() {
        let mut state = ConversationViewModel::new_draft("/tmp/root".to_string());
        state.input_buffer = "first line\nsecond".to_string();

        let reduced = reduce_conversation_input(state, ConversationInputEvent::PreviousWordDeleted);

        assert_eq!(reduced.state.input_buffer, "first line\n");
    }

    #[test]
    fn status_message_shown_replaces_status_text() {
        let state = ConversationViewModel::new_draft("/tmp/root".to_string());

        let reduced = reduce_conversation_input(
            state,
            ConversationInputEvent::StatusMessageShown {
                status_text: "turn still running".to_string(),
            },
        );

        assert_eq!(reduced.state.status_text, "turn still running");
    }

    #[test]
    fn startup_submit_armed_sets_queue_status() {
        let state = ConversationViewModel::new_draft("/tmp/root".to_string());

        let reduced = reduce_conversation_input(
            state,
            ConversationInputEvent::StartupSubmitArmed {
                status_text: "prompt queued until startup checks finish".to_string(),
            },
        );

        assert!(reduced.state.startup_submit_armed);
        assert_eq!(
            reduced.state.status_text,
            "prompt queued until startup checks finish"
        );
    }

    #[test]
    fn input_change_cancels_armed_startup_submit() {
        let mut state = ConversationViewModel::new_draft("/tmp/root".to_string());
        state.arm_startup_submit();

        let reduced = reduce_conversation_input(
            state,
            ConversationInputEvent::CharacterTyped { character: 'a' },
        );

        assert!(!reduced.state.startup_submit_armed);
        assert_eq!(
            reduced.state.status_text,
            "queued startup send canceled after input changed"
        );
        assert_eq!(reduced.state.input_buffer, "a");
    }

    #[test]
    fn colon_input_opens_inline_command_palette() {
        let state = ConversationViewModel::new_draft("/tmp/root".to_string());

        let reduced = reduce_conversation_input(
            state,
            ConversationInputEvent::CharacterTyped { character: ':' },
        );

        assert!(reduced.state.inline_shell_command_palette_state.is_active());
        assert_eq!(
            reduced
                .state
                .inline_shell_command_palette_state
                .selected_command(),
            Some(InlineShellCommand::Diagnostics)
        );
    }

    #[test]
    fn command_palette_can_be_dismissed_without_clearing_input() {
        let mut state = ConversationViewModel::new_draft("/tmp/root".to_string());
        state.input_buffer = ":p".to_string();
        state.sync_inline_shell_command_palette();

        let reduced =
            reduce_conversation_input(state, ConversationInputEvent::InlineCommandPaletteDismissed);

        assert_eq!(reduced.state.input_buffer, ":p");
        assert!(!reduced.state.inline_shell_command_palette_state.is_active());
    }

    #[test]
    fn command_palette_inserted_command_switches_to_argument_entry() {
        let mut state = ConversationViewModel::new_draft("/tmp/root".to_string());
        state.input_buffer = ":t".to_string();
        state.sync_inline_shell_command_palette();

        let reduced = reduce_conversation_input(
            state,
            ConversationInputEvent::InlineCommandPaletteCommandInserted {
                command: InlineShellCommand::MaxAutoTurns,
            },
        );

        assert_eq!(reduced.state.input_buffer, ":turns ");
        assert!(!reduced.state.inline_shell_command_palette_state.is_active());
    }

    #[test]
    fn input_cleared_empties_buffer() {
        let mut state = ConversationViewModel::new_draft("/tmp/root".to_string());
        state.input_buffer = ":diag".to_string();

        let reduced = reduce_conversation_input(state, ConversationInputEvent::InputCleared);

        assert!(reduced.state.input_buffer.is_empty());
    }
}
