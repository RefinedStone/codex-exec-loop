use super::{ConversationViewModel, InlineShellCommand};

/*
 * conversation_input is a pure reducer for composer-facing events. Shell
 * controllers translate keys and overlay actions into ConversationInputEvent;
 * this module updates ConversationViewModel without doing terminal I/O or
 * app-server work, keeping prompt editing testable and replayable.
 */
#[derive(Debug, Clone)]
pub(super) enum ConversationInputEvent {
    // Direct buffer edits come from the main prompt composer and must keep the
    // inline command palette derived from the latest buffer text.
    CharacterTyped { character: char },
    NewlineInserted,
    BackspacePressed,
    PreviousWordDeleted,
    InputCleared,
    // Palette events are navigation/completion state changes layered on top of
    // the same input buffer; they do not represent prompt submission.
    InlineCommandPaletteSelectionMoved { delta: isize },
    InlineCommandPaletteDismissed,
    InlineCommandPaletteCommandInserted { command: InlineShellCommand },
    // Startup submit arm/disarm is the gate between "operator pressed Enter" and
    // "startup checks are ready enough to submit". Edits cancel the arm.
    StartupSubmitArmed { status_text: String },
    StartupSubmitDisarmed { status_text: Option<String> },
    // Status-only events let planning, parallel-mode, and shell controllers share
    // the same conversation status field without mutating transcript state.
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
    // Keep this reducer exhaustive and side-effect free. Runtime submission and
    // stream effects live in turn_submission_runtime/conversation_runtime; input
    // events only shape local view-model state.
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
            // Command completion changes the prompt text even though it is not a
            // plain character event, so it must cancel a queued startup submit.
            clear_startup_submit_after_input_change(&mut state);
            state.insert_inline_shell_command_completion(command);
        }
        ConversationInputEvent::StartupSubmitArmed { status_text } => {
            state.arm_startup_submit();
            state.status_text = status_text;
        }
        ConversationInputEvent::StartupSubmitDisarmed { status_text } => {
            // Preserve the caller-supplied status only when an arm actually
            // existed. Otherwise late disarm events cannot overwrite newer copy.
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
    // All direct buffer edits pass through this helper so startup-submit safety
    // and inline command palette derivation stay coupled to prompt text changes.
    clear_startup_submit_after_input_change(state);
    modifier(&mut state.input_buffer);
    state.sync_inline_shell_command_palette();
}

fn delete_previous_word(buffer: &mut String) {
    if buffer.is_empty() {
        return;
    }
    /*
     * Ctrl+W behaves like a terminal word erase: first drop trailing whitespace,
     * then remove the previous non-whitespace segment while preserving the
     * separator before it. Newlines are whitespace here, so multi-line prompts
     * collapse back to the prior line boundary naturally.
     */
    let trimmed = buffer.trim_end_matches(|character: char| character.is_whitespace());
    let word_start = trimmed
        .rfind(|character: char| character.is_whitespace())
        .map(|index| index + 1)
        .unwrap_or(0);
    buffer.truncate(word_start);
}

fn clear_startup_submit_after_input_change(state: &mut ConversationViewModel) {
    // Startup submit is a promise to send the exact prompt that was visible when
    // Enter was pressed. Any subsequent edit invalidates that promise and must
    // leave explicit copy explaining why the queued send disappeared.
    if state.clear_startup_submit() {
        state.status_text = "queued startup send canceled after input changed".to_string();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn character_typed_appends_to_input_buffer() {
        // Basic character input locks the reducer contract: caller owns key
        // decoding, this module owns appending to the view-model buffer.
        let state = ConversationViewModel::new_draft("/tmp/root".to_string());
        let reduced = reduce_conversation_input(
            state,
            ConversationInputEvent::CharacterTyped { character: 'a' },
        );

        assert_eq!(reduced.state.input_buffer, "a");
    }

    #[test]
    fn backspace_pressed_removes_last_character() {
        // Backspace is modeled as a buffer edit so it also travels through the
        // startup-submit cancellation and palette-sync path.
        let mut state = ConversationViewModel::new_draft("/tmp/root".to_string());
        state.input_buffer = "draft".to_string();
        let reduced = reduce_conversation_input(state, ConversationInputEvent::BackspacePressed);

        assert_eq!(reduced.state.input_buffer, "draf");
    }

    #[test]
    fn newline_inserted_adds_line_break() {
        // Shift/Alt-enter style input adds a literal newline to the prompt; it is
        // not a submit signal at this reducer level.
        let mut state = ConversationViewModel::new_draft("/tmp/root".to_string());
        state.input_buffer = "draft".to_string();
        let reduced = reduce_conversation_input(state, ConversationInputEvent::NewlineInserted);

        assert_eq!(reduced.state.input_buffer, "draft\n");
    }

    #[test]
    fn previous_word_deleted_removes_last_word() {
        // Ctrl+W removes only the last word and leaves the separator before it,
        // matching terminal editor expectations for continued typing.
        let mut state = ConversationViewModel::new_draft("/tmp/root".to_string());
        state.input_buffer = "ship this next".to_string();
        let reduced = reduce_conversation_input(state, ConversationInputEvent::PreviousWordDeleted);

        assert_eq!(reduced.state.input_buffer, "ship this ");
    }

    #[test]
    fn previous_word_deleted_trims_trailing_space_before_removing_last_word() {
        // Trailing whitespace is not treated as a word, so repeated spaces do not
        // require multiple Ctrl+W presses before useful text is removed.
        let mut state = ConversationViewModel::new_draft("/tmp/root".to_string());
        state.input_buffer = "ship this   ".to_string();
        let reduced = reduce_conversation_input(state, ConversationInputEvent::PreviousWordDeleted);

        assert_eq!(reduced.state.input_buffer, "ship ");
    }

    #[test]
    fn previous_word_deleted_respects_newline_boundaries() {
        // Newlines participate in whitespace detection, preserving the previous
        // line prefix when deleting the first word of the current line.
        let mut state = ConversationViewModel::new_draft("/tmp/root".to_string());
        state.input_buffer = "first line\nsecond".to_string();
        let reduced = reduce_conversation_input(state, ConversationInputEvent::PreviousWordDeleted);

        assert_eq!(reduced.state.input_buffer, "first line\n");
    }

    #[test]
    fn status_message_shown_replaces_status_text() {
        // Status events let controllers publish operator-facing copy without
        // touching input, transcript, or runtime lifecycle fields.
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
        // Arming records both the boolean gate and the status line the composer
        // uses while startup checks are still blocking immediate send.
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
        // This is the core race-prevention rule: queued startup submit cannot
        // outlive a prompt edit, because it would send text the user no longer
        // sees in the composer.
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
        // Palette visibility is derived from buffer content, not a separate key
        // mode. A typed colon therefore opens the command palette via sync.
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
        // Dismissal hides suggestions while preserving typed command text so the
        // operator can continue editing or submit the literal prompt.
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
        // Completion replaces the command prefix with canonical command text and
        // a trailing space, moving the composer into argument entry.
        let mut state = ConversationViewModel::new_draft("/tmp/root".to_string());
        state.input_buffer = ":r".to_string();
        state.sync_inline_shell_command_palette();
        let reduced = reduce_conversation_input(
            state,
            ConversationInputEvent::InlineCommandPaletteCommandInserted {
                command: InlineShellCommand::Reset,
            },
        );

        assert_eq!(reduced.state.input_buffer, ":reset ");
        assert!(!reduced.state.inline_shell_command_palette_state.is_active());
    }

    #[test]
    fn input_cleared_empties_buffer() {
        // Clear follows the same reducer path as other edits, so palette state
        // and startup-submit state cannot linger behind an empty composer.
        let mut state = ConversationViewModel::new_draft("/tmp/root".to_string());
        state.input_buffer = ":diag".to_string();
        let reduced = reduce_conversation_input(state, ConversationInputEvent::InputCleared);

        assert!(reduced.state.input_buffer.is_empty());
    }
}
