use unicode_segmentation::UnicodeSegmentation;

use super::super::inline_shell_commands::{InlineShellCommand, InlineShellCommandPaletteState};

/*
 * Composer state is the complete mutable surface available to the input
 * reducer. Conversation transcript, runtime, planning, approval, and viewport
 * state deliberately live outside this type so prompt edits cannot mutate
 * semantic conversation state by accident.
 */
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct ConversationComposerState {
    pub(crate) input_buffer: String,
    input_cursor_byte_index: Option<usize>,
    pub(crate) inline_shell_command_palette_state: InlineShellCommandPaletteState,
    pub(crate) startup_submit_armed: bool,
}

impl ConversationComposerState {
    pub(crate) fn sync_inline_shell_command_palette(&mut self) {
        let preferred_selection = self.inline_shell_command_palette_state.selected_command();
        self.inline_shell_command_palette_state
            .sync_to_input(&self.input_buffer, preferred_selection);
    }

    pub(crate) fn input_cursor_byte_index(&self) -> usize {
        self.input_cursor_byte_index
            .map(|index| clamp_to_grapheme_boundary(&self.input_buffer, index))
            .unwrap_or(self.input_buffer.len())
    }

    pub(crate) fn set_input_cursor_byte_index(&mut self, index: usize) {
        self.input_cursor_byte_index = Some(clamp_to_grapheme_boundary(&self.input_buffer, index));
    }

    pub(crate) fn move_input_cursor_to_end(&mut self) {
        self.input_cursor_byte_index = None;
    }

    pub(crate) fn move_inline_shell_command_palette_selection(&mut self, delta: isize) -> bool {
        self.inline_shell_command_palette_state
            .move_selection(delta)
    }

    pub(crate) fn dismiss_inline_shell_command_palette(&mut self) -> bool {
        self.inline_shell_command_palette_state.dismiss()
    }

    pub(crate) fn insert_inline_shell_command_completion(&mut self, command: InlineShellCommand) {
        self.input_buffer = command.completion_text().to_string();
        self.move_input_cursor_to_end();
        self.sync_inline_shell_command_palette();
    }

    pub(crate) fn clear_input_buffer(&mut self) {
        self.input_buffer.clear();
        self.input_cursor_byte_index = None;
        self.inline_shell_command_palette_state = InlineShellCommandPaletteState::default();
    }

    pub(crate) fn arm_startup_submit(&mut self) {
        self.startup_submit_armed = true;
    }

    pub(crate) fn clear_startup_submit(&mut self) -> bool {
        std::mem::replace(&mut self.startup_submit_armed, false)
    }
}

fn clamp_to_grapheme_boundary(buffer: &str, index: usize) -> usize {
    let clamped_index = index.min(buffer.len());
    if clamped_index == buffer.len() {
        return buffer.len();
    }
    buffer
        .grapheme_indices(true)
        .map(|(byte_index, _)| byte_index)
        .take_while(|byte_index| *byte_index <= clamped_index)
        .last()
        .unwrap_or(0)
}
