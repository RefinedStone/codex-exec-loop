use super::{ConversationComposerState, InlineShellCommand};
use unicode_segmentation::UnicodeSegmentation;

pub(super) const MAX_PROMPT_INPUT_BYTES: usize = 1024 * 1024;
// Composer-side attachment cap mirrors competitor TUIs (opencode allows 5).
pub(super) const MAX_IMAGE_ATTACHMENTS: usize = 5;
const PROMPT_INPUT_LIMIT_STATUS: &str =
    "prompt input limit reached (1048576 bytes); shorten the prompt before submitting";

/*
 * conversation_input is a pure reducer for composer-facing events. Shell
 * controllers translate keys and overlay actions into ConversationInputEvent;
 * this module updates only ConversationComposerState without doing terminal I/O
 * or app-server work, keeping prompt editing testable and replayable.
 */
#[derive(Debug, Clone)]
pub(super) enum ConversationComposerEvent {
    // Direct buffer edits come from the main prompt composer and must keep the
    // inline command palette derived from the latest buffer text.
    CharacterTyped { character: char },
    TextInserted { text: String },
    NewlineInserted,
    BackspacePressed,
    DeletePressed,
    PreviousWordDeleted,
    InputCleared,
    CursorMoved { movement: InputCursorMovement },
    // Image attachments ride beside the text buffer. Adding one changes what
    // the next submission sends, so it cancels a queued startup submit like a
    // text edit would; removal mirrors Backspace on an empty buffer.
    ImageAttachmentAdded { path: String },
    LastImageAttachmentRemoved,
    // Palette events are navigation/completion state changes layered on top of
    // the same input buffer; they do not represent prompt submission.
    InlineCommandPaletteSelectionMoved { delta: isize },
    InlineCommandPaletteDismissed,
    InlineCommandPaletteCommandInserted { command: InlineShellCommand },
    // Startup submit arm/disarm is the gate between "operator pressed Enter" and
    // "startup checks are ready enough to submit". Edits cancel the arm.
    StartupSubmitArmed { status_text: String },
    StartupSubmitDisarmed { status_text: Option<String> },
}

impl ConversationComposerEvent {
    pub(super) fn mutates_input_buffer(&self) -> bool {
        matches!(
            self,
            Self::CharacterTyped { .. }
                | Self::TextInserted { .. }
                | Self::NewlineInserted
                | Self::BackspacePressed
                | Self::DeletePressed
                | Self::PreviousWordDeleted
                | Self::InputCleared
                | Self::InlineCommandPaletteCommandInserted { .. }
        )
    }
}

// Input dispatch also carries semantic conversation updates for existing shell
// callers. app_runtime routes these around the composer reducer so status,
// transcript, and viewport state never enter the reducer's mutable surface.
#[derive(Debug, Clone)]
pub(super) enum ConversationInputEvent {
    Composer(ConversationComposerEvent),
    StatusMessageShown {
        status_text: String,
    },
    ManualPromptPreparationFailed {
        transcript_text: String,
        status_text: String,
    },
}

impl ConversationInputEvent {
    pub(super) fn mutates_input_buffer(&self) -> bool {
        matches!(self, Self::Composer(event) if event.mutates_input_buffer())
    }
}

impl From<ConversationComposerEvent> for ConversationInputEvent {
    fn from(event: ConversationComposerEvent) -> Self {
        Self::Composer(event)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum InputCursorMovement {
    PreviousCharacter,
    NextCharacter,
    PreviousWord,
    NextWord,
    PreviousLine,
    NextLine,
    LineStart,
    LineEnd,
    BufferStart,
    BufferEnd,
}

#[derive(Debug, Clone)]
pub(super) struct ConversationComposerReduction {
    pub state: ConversationComposerState,
    pub effects: Vec<ConversationComposerEffect>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum ConversationComposerEffect {
    ReplaceStatus { status_text: String },
}

pub(super) fn reduce_conversation_input(
    mut state: ConversationComposerState,
    event: ConversationComposerEvent,
) -> ConversationComposerReduction {
    // Keep this reducer exhaustive and side-effect free. Runtime submission and
    // stream effects live in turn_submission_runtime/conversation_runtime; input
    // events only shape local view-model state.
    let mut effects = Vec::new();
    match event {
        ConversationComposerEvent::CharacterTyped { character } => {
            let mut text = String::new();
            text.push(character);
            insert_text_into_input_buffer_and_sync(&mut state, &text, &mut effects);
        }
        ConversationComposerEvent::TextInserted { text } => {
            let normalized_text = normalize_inserted_text(&text);
            insert_text_into_input_buffer_and_sync(&mut state, &normalized_text, &mut effects);
        }
        ConversationComposerEvent::NewlineInserted => {
            insert_text_into_input_buffer_and_sync(&mut state, "\n", &mut effects);
        }
        ConversationComposerEvent::BackspacePressed => {
            modify_input_buffer_and_sync(&mut state, delete_previous_character, &mut effects);
        }
        ConversationComposerEvent::DeletePressed => {
            modify_input_buffer_and_sync(&mut state, delete_next_character, &mut effects);
        }
        ConversationComposerEvent::PreviousWordDeleted => {
            modify_input_buffer_and_sync(&mut state, delete_previous_word, &mut effects);
        }
        ConversationComposerEvent::InputCleared => {
            modify_input_buffer_and_sync(
                &mut state,
                |buffer, _cursor| {
                    buffer.clear();
                    0
                },
                &mut effects,
            );
            // Clearing is a full composer reset: staged image chips must not
            // outlive the prompt they were attached to.
            state.image_attachments.clear();
            state.move_input_cursor_to_end();
        }
        ConversationComposerEvent::CursorMoved { movement } => {
            let cursor_byte_index = move_input_cursor(
                &state.input_buffer,
                state.input_cursor_byte_index(),
                movement,
            );
            state.set_input_cursor_byte_index(cursor_byte_index);
        }
        ConversationComposerEvent::ImageAttachmentAdded { path } => {
            // Attachment changes what Enter will send, so a queued startup
            // submit cannot survive it.
            clear_startup_submit_after_input_change(&mut state, &mut effects);
            let already_attached = state
                .image_attachments
                .iter()
                .any(|existing| existing == &path);
            if !already_attached {
                if state.image_attachments.len() >= MAX_IMAGE_ATTACHMENTS {
                    effects.push(ConversationComposerEffect::ReplaceStatus {
                        status_text: format!(
                            "image attachment limit reached ({} images); remove one before adding another",
                            MAX_IMAGE_ATTACHMENTS
                        ),
                    });
                } else {
                    state.image_attachments.push(path);
                }
            }
        }
        ConversationComposerEvent::LastImageAttachmentRemoved => {
            state.image_attachments.pop();
        }
        ConversationComposerEvent::InlineCommandPaletteSelectionMoved { delta } => {
            state.move_inline_shell_command_palette_selection(delta);
        }
        ConversationComposerEvent::InlineCommandPaletteDismissed => {
            state.dismiss_inline_shell_command_palette();
        }
        ConversationComposerEvent::InlineCommandPaletteCommandInserted { command } => {
            // Command completion changes the prompt text even though it is not a
            // plain character event, so it must cancel a queued startup submit.
            clear_startup_submit_after_input_change(&mut state, &mut effects);
            state.insert_inline_shell_command_completion(command);
        }
        ConversationComposerEvent::StartupSubmitArmed { status_text } => {
            state.arm_startup_submit();
            effects.push(ConversationComposerEffect::ReplaceStatus { status_text });
        }
        ConversationComposerEvent::StartupSubmitDisarmed { status_text } => {
            // Preserve the caller-supplied status only when an arm actually
            // existed. Otherwise late disarm events cannot overwrite newer copy.
            if state.clear_startup_submit()
                && let Some(status_text) = status_text
            {
                effects.push(ConversationComposerEffect::ReplaceStatus { status_text });
            }
        }
    }

    ConversationComposerReduction { state, effects }
}

fn modify_input_buffer_and_sync(
    state: &mut ConversationComposerState,
    modifier: impl FnOnce(&mut String, usize) -> usize,
    effects: &mut Vec<ConversationComposerEffect>,
) {
    // All direct buffer edits pass through this helper so startup-submit safety
    // and inline command palette derivation stay coupled to prompt text changes.
    clear_startup_submit_after_input_change(state, effects);
    let cursor_byte_index = state.input_cursor_byte_index();
    let next_cursor_byte_index = modifier(&mut state.input_buffer, cursor_byte_index);
    state.set_input_cursor_byte_index(next_cursor_byte_index);
    state.sync_inline_shell_command_palette();
}

fn insert_text_into_input_buffer_and_sync(
    state: &mut ConversationComposerState,
    text: &str,
    effects: &mut Vec<ConversationComposerEffect>,
) {
    if state.input_buffer.len().saturating_add(text.len()) > MAX_PROMPT_INPUT_BYTES {
        clear_startup_submit_after_input_change(state, effects);
        effects.push(ConversationComposerEffect::ReplaceStatus {
            status_text: PROMPT_INPUT_LIMIT_STATUS.to_string(),
        });
        return;
    }
    modify_input_buffer_and_sync(
        state,
        |buffer, cursor_byte_index| {
            buffer.insert_str(cursor_byte_index, text);
            cursor_byte_index + text.len()
        },
        effects,
    );
}

fn normalize_inserted_text(text: &str) -> String {
    text.replace("\r\n", "\n").replace('\r', "\n")
}

fn delete_previous_character(buffer: &mut String, cursor_byte_index: usize) -> usize {
    if cursor_byte_index == 0 || buffer.is_empty() {
        return cursor_byte_index;
    }

    let previous_byte_index = previous_character_start(buffer, cursor_byte_index);
    buffer.drain(previous_byte_index..cursor_byte_index);
    previous_byte_index
}

fn delete_next_character(buffer: &mut String, cursor_byte_index: usize) -> usize {
    if cursor_byte_index >= buffer.len() {
        return cursor_byte_index;
    }

    let next_byte_index = next_character_end(buffer, cursor_byte_index);
    buffer.drain(cursor_byte_index..next_byte_index);
    cursor_byte_index
}

fn delete_previous_word(buffer: &mut String, cursor_byte_index: usize) -> usize {
    if buffer.is_empty() {
        return cursor_byte_index;
    }
    /*
     * Ctrl+W behaves like a terminal word erase: first drop trailing whitespace,
     * then remove the previous non-whitespace segment while preserving the
     * separator before it. Newlines are whitespace here, so multi-line prompts
     * collapse back to the prior line boundary naturally.
     */
    let word_start = move_to_previous_word_start(buffer, cursor_byte_index);
    buffer.drain(word_start..cursor_byte_index);
    word_start
}

fn move_input_cursor(
    buffer: &str,
    cursor_byte_index: usize,
    movement: InputCursorMovement,
) -> usize {
    match movement {
        InputCursorMovement::PreviousCharacter => {
            previous_character_start(buffer, cursor_byte_index)
        }
        InputCursorMovement::NextCharacter => next_character_end(buffer, cursor_byte_index),
        InputCursorMovement::PreviousWord => move_to_previous_word_start(buffer, cursor_byte_index),
        InputCursorMovement::NextWord => move_to_next_word_end(buffer, cursor_byte_index),
        InputCursorMovement::PreviousLine => {
            move_to_adjacent_line(buffer, cursor_byte_index, LineDirection::Previous)
        }
        InputCursorMovement::NextLine => {
            move_to_adjacent_line(buffer, cursor_byte_index, LineDirection::Next)
        }
        InputCursorMovement::LineStart => current_line_start(buffer, cursor_byte_index),
        InputCursorMovement::LineEnd => current_line_end(buffer, cursor_byte_index),
        InputCursorMovement::BufferStart => 0,
        InputCursorMovement::BufferEnd => buffer.len(),
    }
}

fn previous_character_start(buffer: &str, cursor_byte_index: usize) -> usize {
    if cursor_byte_index == 0 {
        return 0;
    }

    buffer[..cursor_byte_index]
        .grapheme_indices(true)
        .next_back()
        .map(|(byte_index, _)| byte_index)
        .unwrap_or(0)
}

fn next_character_end(buffer: &str, cursor_byte_index: usize) -> usize {
    if cursor_byte_index >= buffer.len() {
        return buffer.len();
    }

    buffer[cursor_byte_index..]
        .graphemes(true)
        .next()
        .map(|grapheme| cursor_byte_index + grapheme.len())
        .unwrap_or(buffer.len())
}

fn move_to_previous_word_start(buffer: &str, cursor_byte_index: usize) -> usize {
    let mut next_cursor_byte_index = cursor_byte_index;

    while next_cursor_byte_index > 0
        && character_before(buffer, next_cursor_byte_index)
            .is_some_and(|character| character.is_whitespace())
    {
        next_cursor_byte_index = previous_character_start(buffer, next_cursor_byte_index);
    }

    while next_cursor_byte_index > 0
        && character_before(buffer, next_cursor_byte_index)
            .is_some_and(|character| !character.is_whitespace())
    {
        next_cursor_byte_index = previous_character_start(buffer, next_cursor_byte_index);
    }

    next_cursor_byte_index
}

fn move_to_next_word_end(buffer: &str, cursor_byte_index: usize) -> usize {
    let mut next_cursor_byte_index = cursor_byte_index;

    while next_cursor_byte_index < buffer.len()
        && character_at(buffer, next_cursor_byte_index)
            .is_some_and(|character| character.is_whitespace())
    {
        next_cursor_byte_index = next_character_end(buffer, next_cursor_byte_index);
    }

    while next_cursor_byte_index < buffer.len()
        && character_at(buffer, next_cursor_byte_index)
            .is_some_and(|character| !character.is_whitespace())
    {
        next_cursor_byte_index = next_character_end(buffer, next_cursor_byte_index);
    }

    next_cursor_byte_index
}

fn character_before(buffer: &str, cursor_byte_index: usize) -> Option<char> {
    if cursor_byte_index == 0 {
        return None;
    }

    buffer[..cursor_byte_index].chars().next_back()
}

fn character_at(buffer: &str, cursor_byte_index: usize) -> Option<char> {
    if cursor_byte_index >= buffer.len() {
        return None;
    }

    buffer[cursor_byte_index..].chars().next()
}

fn current_line_start(buffer: &str, cursor_byte_index: usize) -> usize {
    buffer[..cursor_byte_index]
        .rfind('\n')
        .map(|newline_index| newline_index + 1)
        .unwrap_or(0)
}

fn current_line_end(buffer: &str, cursor_byte_index: usize) -> usize {
    buffer[cursor_byte_index..]
        .find('\n')
        .map(|offset| cursor_byte_index + offset)
        .unwrap_or(buffer.len())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LineDirection {
    Previous,
    Next,
}

fn move_to_adjacent_line(
    buffer: &str,
    cursor_byte_index: usize,
    direction: LineDirection,
) -> usize {
    let current_start = current_line_start(buffer, cursor_byte_index);
    let current_column = buffer[current_start..cursor_byte_index]
        .graphemes(true)
        .count();

    match direction {
        LineDirection::Previous => {
            if current_start == 0 {
                return cursor_byte_index;
            }

            let previous_end = current_start - 1;
            let previous_start = current_line_start(buffer, previous_end);
            byte_index_at_line_column(buffer, previous_start, previous_end, current_column)
        }
        LineDirection::Next => {
            let current_end = current_line_end(buffer, cursor_byte_index);
            if current_end == buffer.len() {
                return cursor_byte_index;
            }

            let next_start = current_end + 1;
            let next_end = current_line_end(buffer, next_start);
            byte_index_at_line_column(buffer, next_start, next_end, current_column)
        }
    }
}

fn byte_index_at_line_column(
    buffer: &str,
    line_start: usize,
    line_end: usize,
    column: usize,
) -> usize {
    buffer[line_start..line_end]
        .grapheme_indices(true)
        .nth(column)
        .map(|(offset, _)| line_start + offset)
        .unwrap_or(line_end)
}

fn clear_startup_submit_after_input_change(
    state: &mut ConversationComposerState,
    effects: &mut Vec<ConversationComposerEffect>,
) {
    // Startup submit is a promise to send the exact prompt that was visible when
    // Enter was pressed. Any subsequent edit invalidates that promise and must
    // leave explicit copy explaining why the queued send disappeared.
    if state.clear_startup_submit() {
        effects.push(ConversationComposerEffect::ReplaceStatus {
            status_text: "queued startup send canceled after input changed".to_string(),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn character_typed_appends_to_input_buffer() {
        // Basic character input locks the reducer contract: caller owns key
        // decoding, this module owns appending to the view-model buffer.
        let state = ConversationComposerState::default();
        let reduced = reduce_conversation_input(
            state,
            ConversationComposerEvent::CharacterTyped { character: 'a' },
        );

        assert_eq!(reduced.state.input_buffer, "a");
    }

    #[test]
    fn backspace_pressed_removes_last_character() {
        // Backspace is modeled as a buffer edit so it also travels through the
        // startup-submit cancellation and palette-sync path.
        let mut state = ConversationComposerState::default();
        state.input_buffer = "draft".to_string();
        let reduced = reduce_conversation_input(state, ConversationComposerEvent::BackspacePressed);

        assert_eq!(reduced.state.input_buffer, "draf");
    }

    #[test]
    fn character_typed_inserts_at_cursor_position() {
        let mut state = ConversationComposerState::default();
        state.input_buffer = "ship now".to_string();
        state.set_input_cursor_byte_index("ship ".len());
        let reduced = reduce_conversation_input(
            state,
            ConversationComposerEvent::CharacterTyped { character: 'i' },
        );

        assert_eq!(reduced.state.input_buffer, "ship inow");
        assert_eq!(reduced.state.input_cursor_byte_index(), "ship i".len());
    }

    #[test]
    fn text_inserted_preserves_newlines_without_submitting() {
        let mut state = ConversationComposerState::default();
        state.input_buffer = "before ".to_string();
        let reduced = reduce_conversation_input(
            state,
            ConversationComposerEvent::TextInserted {
                text: "first\r\nsecond".to_string(),
            },
        );

        assert_eq!(reduced.state.input_buffer, "before first\nsecond");
    }

    #[test]
    fn text_inserted_rejects_an_atomic_paste_above_the_prompt_limit() {
        let mut state = ConversationComposerState::default();
        state.input_buffer = "a".repeat(MAX_PROMPT_INPUT_BYTES - 1);
        let original = state.input_buffer.clone();

        let reduced = reduce_conversation_input(
            state,
            ConversationComposerEvent::TextInserted {
                text: "한".to_string(),
            },
        );

        assert_eq!(reduced.state.input_buffer, original);
        assert_eq!(
            reduced.effects,
            vec![ConversationComposerEffect::ReplaceStatus {
                status_text: PROMPT_INPUT_LIMIT_STATUS.to_string(),
            }]
        );
        assert_eq!(
            reduced.state.input_cursor_byte_index(),
            MAX_PROMPT_INPUT_BYTES - 1
        );
    }

    #[test]
    fn newline_inserted_adds_line_break() {
        // Shift/Alt-enter style input adds a literal newline to the prompt; it is
        // not a submit signal at this reducer level.
        let mut state = ConversationComposerState::default();
        state.input_buffer = "draft".to_string();
        let reduced = reduce_conversation_input(state, ConversationComposerEvent::NewlineInserted);

        assert_eq!(reduced.state.input_buffer, "draft\n");
    }

    #[test]
    fn backspace_and_cursor_moves_never_split_multibyte_graphemes() {
        // 한글 + 결합 이모지 프롬프트에서 커서와 삭제는 항상 그래핌 경계에
        // 머물러야 한다. 그렇지 않으면 패닉 또는 문자열 깨짐으로 이어져
        // 프롬프트 전송 플로우가 망가진다.
        let mut state = ConversationComposerState::default();
        let inserted = reduce_conversation_input(
            state,
            ConversationComposerEvent::TextInserted {
                text: "한글\u{1F44D}".to_string(),
            },
        );
        state = inserted.state;
        assert_eq!(state.input_buffer, "한글\u{1F44D}");

        // Backspace removes the whole emoji grapheme in one step.
        let reduced = reduce_conversation_input(state, ConversationComposerEvent::BackspacePressed);
        assert_eq!(reduced.state.input_buffer, "한글");

        // Cursor-left lands on the boundary before 글 (byte index 3); backspace
        // then removes 글 and leaves only 한.
        let mut state = reduced.state;
        let moved = reduce_conversation_input(
            state,
            ConversationComposerEvent::CursorMoved {
                movement: InputCursorMovement::PreviousCharacter,
            },
        );
        state = moved.state;
        assert_eq!(state.input_cursor_byte_index(), "한".len());

        let reduced = reduce_conversation_input(state, ConversationComposerEvent::BackspacePressed);
        assert_eq!(reduced.state.input_buffer, "글");
    }

    #[test]
    fn cursor_movement_controls_backspace_target() {
        let mut state = ConversationComposerState::default();
        state.input_buffer = "hello".to_string();
        let moved_left = reduce_conversation_input(
            state,
            ConversationComposerEvent::CursorMoved {
                movement: InputCursorMovement::PreviousCharacter,
            },
        );
        let moved_left = reduce_conversation_input(
            moved_left.state,
            ConversationComposerEvent::CursorMoved {
                movement: InputCursorMovement::PreviousCharacter,
            },
        );
        let reduced = reduce_conversation_input(
            moved_left.state,
            ConversationComposerEvent::BackspacePressed,
        );

        assert_eq!(reduced.state.input_buffer, "helo");
        assert_eq!(reduced.state.input_cursor_byte_index(), "he".len());
    }

    #[test]
    fn cursor_movement_and_deletion_keep_extended_graphemes_intact() {
        let mut state = ConversationComposerState::default();
        state.input_buffer = "e\u{301}👩‍💻".to_string();

        let moved = reduce_conversation_input(
            state,
            ConversationComposerEvent::CursorMoved {
                movement: InputCursorMovement::PreviousCharacter,
            },
        );
        assert_eq!(moved.state.input_cursor_byte_index(), "e\u{301}".len());

        let reduced =
            reduce_conversation_input(moved.state, ConversationComposerEvent::BackspacePressed);
        assert_eq!(reduced.state.input_buffer, "👩‍💻");
        assert_eq!(reduced.state.input_cursor_byte_index(), 0);
    }

    #[test]
    fn cursor_index_clamps_to_the_start_of_a_combining_grapheme() {
        let mut state = ConversationComposerState::default();
        state.input_buffer = "e\u{301}x".to_string();

        state.set_input_cursor_byte_index(1);

        assert_eq!(state.input_cursor_byte_index(), 0);
    }

    #[test]
    fn word_cursor_movement_targets_previous_word_start() {
        let mut state = ConversationComposerState::default();
        state.input_buffer = "one two three".to_string();
        let moved = reduce_conversation_input(
            state,
            ConversationComposerEvent::CursorMoved {
                movement: InputCursorMovement::PreviousWord,
            },
        );
        let reduced = reduce_conversation_input(
            moved.state,
            ConversationComposerEvent::CharacterTyped { character: 'X' },
        );

        assert_eq!(reduced.state.input_buffer, "one two Xthree");
    }

    #[test]
    fn vertical_cursor_movement_uses_matching_line_column() {
        let mut state = ConversationComposerState::default();
        state.input_buffer = "ab\ncde".to_string();
        let moved = reduce_conversation_input(
            state,
            ConversationComposerEvent::CursorMoved {
                movement: InputCursorMovement::PreviousLine,
            },
        );
        let reduced = reduce_conversation_input(
            moved.state,
            ConversationComposerEvent::CharacterTyped { character: 'X' },
        );

        assert_eq!(reduced.state.input_buffer, "abX\ncde");
    }

    #[test]
    fn previous_word_deleted_removes_last_word() {
        // Ctrl+W removes only the last word and leaves the separator before it,
        // matching terminal editor expectations for continued typing.
        let mut state = ConversationComposerState::default();
        state.input_buffer = "ship this next".to_string();
        let reduced =
            reduce_conversation_input(state, ConversationComposerEvent::PreviousWordDeleted);

        assert_eq!(reduced.state.input_buffer, "ship this ");
    }

    #[test]
    fn previous_word_deleted_trims_trailing_space_before_removing_last_word() {
        // Trailing whitespace is not treated as a word, so repeated spaces do not
        // require multiple Ctrl+W presses before useful text is removed.
        let mut state = ConversationComposerState::default();
        state.input_buffer = "ship this   ".to_string();
        let reduced =
            reduce_conversation_input(state, ConversationComposerEvent::PreviousWordDeleted);

        assert_eq!(reduced.state.input_buffer, "ship ");
    }

    #[test]
    fn previous_word_deleted_respects_newline_boundaries() {
        // Newlines participate in whitespace detection, preserving the previous
        // line prefix when deleting the first word of the current line.
        let mut state = ConversationComposerState::default();
        state.input_buffer = "first line\nsecond".to_string();
        let reduced =
            reduce_conversation_input(state, ConversationComposerEvent::PreviousWordDeleted);

        assert_eq!(reduced.state.input_buffer, "first line\n");
    }

    #[test]
    fn startup_submit_armed_sets_queue_status() {
        // Arming records both the boolean gate and the status line the composer
        // uses while startup checks are still blocking immediate send.
        let state = ConversationComposerState::default();
        let reduced = reduce_conversation_input(
            state,
            ConversationComposerEvent::StartupSubmitArmed {
                status_text: "prompt queued until startup checks finish".to_string(),
            },
        );

        assert!(reduced.state.startup_submit_armed);
        assert_eq!(
            reduced.effects,
            vec![ConversationComposerEffect::ReplaceStatus {
                status_text: "prompt queued until startup checks finish".to_string(),
            }]
        );
    }

    #[test]
    fn input_change_cancels_armed_startup_submit() {
        // This is the core race-prevention rule: queued startup submit cannot
        // outlive a prompt edit, because it would send text the user no longer
        // sees in the composer.
        let mut state = ConversationComposerState::default();
        state.arm_startup_submit();
        let reduced = reduce_conversation_input(
            state,
            ConversationComposerEvent::CharacterTyped { character: 'a' },
        );

        assert!(!reduced.state.startup_submit_armed);
        assert_eq!(
            reduced.effects,
            vec![ConversationComposerEffect::ReplaceStatus {
                status_text: "queued startup send canceled after input changed".to_string(),
            }]
        );
        assert_eq!(reduced.state.input_buffer, "a");
    }

    #[test]
    fn disarming_without_an_arm_does_not_replace_newer_status() {
        let reduced = reduce_conversation_input(
            ConversationComposerState::default(),
            ConversationComposerEvent::StartupSubmitDisarmed {
                status_text: Some("stale startup status".to_string()),
            },
        );

        assert!(reduced.effects.is_empty());
    }

    #[test]
    fn oversized_paste_cancels_startup_arm_before_reporting_the_limit() {
        let mut state = ConversationComposerState::default();
        state.input_buffer = "a".repeat(MAX_PROMPT_INPUT_BYTES - 1);
        state.arm_startup_submit();

        let reduced = reduce_conversation_input(
            state,
            ConversationComposerEvent::TextInserted {
                text: "한".to_string(),
            },
        );

        assert!(!reduced.state.startup_submit_armed);
        assert_eq!(
            reduced.effects,
            vec![
                ConversationComposerEffect::ReplaceStatus {
                    status_text: "queued startup send canceled after input changed".to_string(),
                },
                ConversationComposerEffect::ReplaceStatus {
                    status_text: PROMPT_INPUT_LIMIT_STATUS.to_string(),
                },
            ]
        );
    }

    #[test]
    fn colon_input_opens_inline_command_palette() {
        // Palette visibility is derived from buffer content, not a separate key
        // mode. A typed colon therefore opens the command palette via sync.
        let state = ConversationComposerState::default();
        let reduced = reduce_conversation_input(
            state,
            ConversationComposerEvent::CharacterTyped { character: ':' },
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
        let mut state = ConversationComposerState::default();
        state.input_buffer = ":p".to_string();
        state.sync_inline_shell_command_palette();
        let reduced = reduce_conversation_input(
            state,
            ConversationComposerEvent::InlineCommandPaletteDismissed,
        );

        assert_eq!(reduced.state.input_buffer, ":p");
        assert!(!reduced.state.inline_shell_command_palette_state.is_active());
    }

    #[test]
    fn command_palette_inserted_command_switches_to_argument_entry() {
        // Completion replaces the command prefix with canonical command text and
        // a trailing space, moving the composer into argument entry.
        let mut state = ConversationComposerState::default();
        state.input_buffer = ":r".to_string();
        state.sync_inline_shell_command_palette();
        let reduced = reduce_conversation_input(
            state,
            ConversationComposerEvent::InlineCommandPaletteCommandInserted {
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
        let mut state = ConversationComposerState::default();
        state.input_buffer = ":diag".to_string();
        let reduced = reduce_conversation_input(state, ConversationComposerEvent::InputCleared);

        assert!(reduced.state.input_buffer.is_empty());
    }

    #[test]
    fn image_attachment_added_appends_path_and_cancels_startup_submit() {
        let mut state = ConversationComposerState::default();
        state.input_buffer = "look at this".to_string();
        state.arm_startup_submit();

        let reduced = reduce_conversation_input(
            state,
            ConversationComposerEvent::ImageAttachmentAdded {
                path: "/tmp/shot.png".to_string(),
            },
        );

        assert_eq!(reduced.state.image_attachments, vec!["/tmp/shot.png"]);
        assert!(!reduced.state.startup_submit_armed);
    }

    #[test]
    fn image_attachment_added_rejects_duplicate_paths() {
        let event = ConversationComposerEvent::ImageAttachmentAdded {
            path: "/tmp/shot.png".to_string(),
        };
        let reduced =
            reduce_conversation_input(ConversationComposerState::default(), event.clone());
        let reduced = reduce_conversation_input(reduced.state, event);

        assert_eq!(reduced.state.image_attachments.len(), 1);
        assert!(reduced.effects.is_empty());
    }

    #[test]
    fn image_attachment_added_enforces_the_attachment_cap() {
        let mut state = ConversationComposerState::default();
        for index in 0..super::MAX_IMAGE_ATTACHMENTS {
            let reduced = reduce_conversation_input(
                state,
                ConversationComposerEvent::ImageAttachmentAdded {
                    path: format!("/tmp/shot-{index}.png"),
                },
            );
            state = reduced.state;
        }

        let reduced = reduce_conversation_input(
            state,
            ConversationComposerEvent::ImageAttachmentAdded {
                path: "/tmp/overflow.png".to_string(),
            },
        );

        assert_eq!(
            reduced.state.image_attachments.len(),
            super::MAX_IMAGE_ATTACHMENTS
        );
        assert!(matches!(
            reduced.effects.as_slice(),
            [ConversationComposerEffect::ReplaceStatus { .. }]
        ));
    }

    #[test]
    fn last_image_attachment_removed_pops_in_order() {
        let mut state = ConversationComposerState::default();
        for path in ["/tmp/first.png", "/tmp/second.png"] {
            let reduced = reduce_conversation_input(
                state,
                ConversationComposerEvent::ImageAttachmentAdded {
                    path: path.to_string(),
                },
            );
            state = reduced.state;
        }
        let reduced =
            reduce_conversation_input(state, ConversationComposerEvent::LastImageAttachmentRemoved);

        assert_eq!(reduced.state.image_attachments, vec!["/tmp/first.png"]);
    }

    #[test]
    fn clearing_input_also_clears_image_attachments() {
        let mut state = ConversationComposerState::default();
        let attached = reduce_conversation_input(
            state,
            ConversationComposerEvent::ImageAttachmentAdded {
                path: "/tmp/shot.png".to_string(),
            },
        );
        state = attached.state;

        let cleared = reduce_conversation_input(state, ConversationComposerEvent::InputCleared);
        assert!(cleared.state.image_attachments.is_empty());
    }
}
