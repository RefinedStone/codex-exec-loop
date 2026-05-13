use super::*;

/*
prompt_composer는 shell footer의 입력 영역을 만드는 presentation adapter다.
conversation reducer가 가진 raw buffer와 input state를 읽어 "지금 사용자가 무엇을 할 수 있는지"를
line copy로 바꾸고, renderer가 cursor를 올바른 terminal row에 놓을 수 있도록 같은 buffer projection을
좌표 계산에도 재사용한다.
*/
const PROMPT_PRIMARY_PREFIX: &str = "> ";
const PROMPT_CONTINUATION_PREFIX: &str = "  ";

pub(super) struct PromptBufferView {
    // Prompt text is already split into ratatui Lines so popup and inline tail renderers share one projection.
    pub(super) lines: Vec<Line<'static>>,
    // Cursor location is relative to this projected prompt buffer, before surrounding footer rows are added.
    pub(super) cursor_line_index: usize,
    pub(super) cursor_column: usize,
}

pub(super) fn build_shell_command_palette_lines(
    conversation: &ConversationViewModel,
) -> Vec<Line<'static>> {
    let palette_state = &conversation.inline_shell_command_palette_state;
    // Dismissed palettes should leave the typed buffer visible without suggestion rows.
    if !palette_state.is_active() {
        return Vec::new();
    }
    // Suggestion prefix is only present while the user is typing the command token, not arguments.
    let Some(prefix) = InlineShellCommand::suggestion_prefix(&conversation.input_buffer) else {
        return Vec::new();
    };
    // Empty results still render feedback so the user knows the palette is active and filtering.
    if palette_state.suggestions().is_empty() {
        return vec![Line::from(vec![
            Span::raw("  no shell commands match `"),
            Span::raw(prefix),
            Span::raw("`"),
        ])];
    }
    let selected_index = palette_state.selected_index().unwrap_or(0);
    let suggestions = palette_state.suggestions();
    let (window_start, window_end) =
        build_shell_command_palette_window(suggestions.len(), selected_index);

    /*
    The palette window is a presentation concern: command registry ordering stays in
    inline_shell_commands, while this layer only decides which visible slice surrounds
    the selected row and how to style the active item.
    */
    suggestions[window_start..window_end]
        .iter()
        .enumerate()
        .map(|(offset, command)| {
            let is_selected = selected_index == window_start + offset;
            let selector = if is_selected { "> " } else { "  " };
            let label_style = if is_selected {
                AkraTheme::brand()
            } else {
                Style::default()
            };
            let detail_style = if is_selected {
                Style::default().add_modifier(Modifier::BOLD)
            } else {
                AkraTheme::subtle()
            };
            Line::from(vec![
                Span::raw(selector),
                Span::styled(command.command_name(), label_style),
                Span::raw("  "),
                Span::styled(command.suggestion_detail(), detail_style),
                if command.requires_argument() {
                    Span::styled(" / add value", detail_style)
                } else {
                    Span::raw("")
                },
            ])
        })
        .collect()
}

fn build_shell_command_palette_window(
    suggestion_count: usize,
    selected_index: usize,
) -> (usize, usize) {
    // Small lists render whole; longer lists keep the selected command roughly centered.
    if suggestion_count <= INLINE_COMMAND_PALETTE_VISIBLE_LIMIT {
        return (0, suggestion_count);
    }
    let max_window_start = suggestion_count - INLINE_COMMAND_PALETTE_VISIBLE_LIMIT;
    let window_start = selected_index
        .saturating_sub(INLINE_COMMAND_PALETTE_VISIBLE_LIMIT / 2)
        .min(max_window_start);
    (
        window_start,
        window_start + INLINE_COMMAND_PALETTE_VISIBLE_LIMIT,
    )
}

pub(super) fn build_prompt_cursor_offset(
    conversation: &ConversationViewModel,
    content_width: u16,
) -> Option<(u16, u16)> {
    // A zero-width area means the renderer cannot place a cursor safely.
    if content_width == 0 {
        return None;
    }
    let prompt_buffer = build_prompt_buffer_view(conversation);
    /*
    Cursor rows must account for terminal wrapping across every prompt line before the active
    line. This keeps multi-line prompts and long single-line prompts aligned with ratatui's
    rendered width rather than byte offsets in the input buffer.
    */
    let wrapped_rows_before_cursor = prompt_buffer.lines[..prompt_buffer.cursor_line_index]
        .iter()
        .map(|line| wrapped_row_count(line.width(), content_width))
        .sum::<usize>();
    let cursor_row_in_line = prompt_buffer.cursor_column / content_width as usize;
    let cursor_column = (prompt_buffer.cursor_column % content_width as usize) as u16;
    let cursor_row = wrapped_rows_before_cursor
        .saturating_add(cursor_row_in_line)
        .min(u16::MAX as usize) as u16;

    Some((cursor_column, cursor_row))
}

pub(super) fn build_prompt_buffer_view(conversation: &ConversationViewModel) -> PromptBufferView {
    /*
    Prefixes are part of the prompt projection, so cursor_column is measured after the prefix.
    That makes renderer cursor placement match exactly what the user sees in the footer.
    */
    let cursor_byte_index = conversation.input_cursor_byte_index();
    let buffer_lines = conversation.input_buffer.split('\n').collect::<Vec<_>>();
    let mut lines = Vec::with_capacity(buffer_lines.len().max(1));
    let mut cursor_line_index = 0;
    let mut cursor_column = 0;
    let mut line_start_byte_index = 0;
    let mut cursor_position_resolved = false;

    for (index, buffer_line) in buffer_lines.iter().enumerate() {
        let prefix = prompt_line_prefix(index);
        let line = Line::from(vec![
            Span::raw(prefix),
            Span::raw((*buffer_line).to_string()),
        ]);
        let line_end_byte_index = line_start_byte_index + buffer_line.len();
        if !cursor_position_resolved && cursor_byte_index <= line_end_byte_index {
            cursor_line_index = index;
            let cursor_line_text =
                &conversation.input_buffer[line_start_byte_index..cursor_byte_index];
            cursor_column = prompt_line_width_before_cursor(prefix, cursor_line_text);
            cursor_position_resolved = true;
        }
        lines.push(line);
        line_start_byte_index = line_end_byte_index.saturating_add(1);
    }

    PromptBufferView {
        lines,
        cursor_line_index,
        cursor_column,
    }
}

fn prompt_line_prefix(line_index: usize) -> &'static str {
    if line_index == 0 {
        PROMPT_PRIMARY_PREFIX
    } else {
        PROMPT_CONTINUATION_PREFIX
    }
}

fn prompt_line_width_before_cursor(prefix: &'static str, text_before_cursor: &str) -> usize {
    Line::from(vec![
        Span::raw(prefix),
        Span::raw(text_before_cursor.to_string()),
    ])
    .width()
}

pub(super) fn wrapped_row_count(line_width: usize, content_width: u16) -> usize {
    // Empty prompt lines still occupy one terminal row.
    if content_width == 0 {
        return 0;
    }
    if line_width == 0 {
        return 1;
    }

    line_width.div_ceil(content_width as usize)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prompt_cursor_offset_uses_conversation_cursor_position() {
        let mut conversation = ConversationViewModel::new_draft("/tmp/root".to_string());
        conversation.input_buffer = "hello".to_string();
        conversation.set_input_cursor_byte_index(2);

        assert_eq!(build_prompt_cursor_offset(&conversation, 80), Some((4, 0)));
    }

    #[test]
    fn prompt_cursor_offset_handles_multiline_middle_cursor() {
        let mut conversation = ConversationViewModel::new_draft("/tmp/root".to_string());
        conversation.input_buffer = "one\ntwo".to_string();
        conversation.set_input_cursor_byte_index("one\n".len() + 1);

        assert_eq!(build_prompt_cursor_offset(&conversation, 80), Some((3, 1)));
    }
}
