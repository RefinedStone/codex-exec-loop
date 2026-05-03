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

#[cfg(test)]
pub(super) fn build_input_lines_with_context(
    context: &ShellCorePresentationContext<'_>,
) -> Vec<Line<'static>> {
    // Non-ready states cannot expose the editable prompt buffer; they render status-only copy instead.
    match context.conversation_state {
        ShellConversationState::Loading => vec![
            Line::from("Thread is still loading."),
            Line::from("Input becomes available when the shell reaches ready state."),
        ],
        ShellConversationState::Failed(message) => vec![Line::from(message.to_string())],
        ShellConversationState::Ready(conversation) => {
            build_ready_input_lines(conversation, context.shell_action_availability)
        }
    }
}

#[cfg(test)]
pub(in super::super) fn build_ready_input_lines(
    conversation: &ConversationViewModel,
    shell_action_availability: ShellActionAvailability,
) -> Vec<Line<'static>> {
    // Always render the typed buffer first. Additional guidance below is contextual footer copy.
    let prompt_buffer = build_prompt_buffer_view(conversation);
    let mut lines = prompt_buffer.lines;

    if conversation.input_buffer.is_empty() {
        /*
        Empty prompt is where the footer acts like a status guide. Auto-follow state wins over
        generic readiness copy because it explains why the app may send the next prompt itself.
        */
        if let Some(status_lines) = auto_follow_prompt_lines(conversation) {
            lines.extend(status_lines);
            lines.push(Line::from(InlineShellCommand::command_list_line()));
            return lines;
        }
        /*
        Startup availability is checked before ordinary DraftReady/ReadyToContinue copy so the
        operator does not interpret Enter as immediately executable while diagnostics are pending.
        */
        match (conversation.input_state, shell_action_availability) {
            (_, ShellActionAvailability::Pending) if conversation.input_state.can_submit_now() => {
                lines.push(Line::from("Startup checks are still running."));
                lines.push(Line::from(
                    "Type now if you want, then send once diagnostics turn ready.",
                ));
            }
            (_, ShellActionAvailability::Blocked) if conversation.input_state.can_submit_now() => {
                lines.push(Line::from("Startup diagnostics need attention."));
                lines.push(Line::from(
                    "Open Ctrl+d, resolve the warning, then send the prompt.",
                ));
            }
            (ConversationInputState::DraftReady, _) => {
                lines.push(Line::from("Ready to start a new thread."));
                lines.push(Line::from(
                    "Type the first prompt, Ctrl+j for newline, Enter to send.",
                ));
            }
            (ConversationInputState::ReadyToContinue, _) => {
                lines.push(Line::from("Ready to continue this session."));
                lines.push(Line::from(
                    "Type the next prompt, Ctrl+j for newline, Enter to send.",
                ));
            }
            (ConversationInputState::SubmittingTurn, _) => {
                lines.push(Line::from("Sending prompt to Codex..."));
                lines.push(Line::from(
                    "Wait for the turn to open before sending again.",
                ));
            }
            (ConversationInputState::StreamingTurn, _) => {
                lines.push(Line::from("Codex is still working on the current turn."));
                lines.push(Line::from(
                    "Type now; press Enter after the turn completes.",
                ));
            }
        }

        lines.push(Line::from(InlineShellCommand::command_list_line()));
        return lines;
    }

    // While the palette is active, suggestions replace normal buffered-prompt hints.
    if conversation.inline_shell_command_palette_state.is_active() {
        lines.extend(build_shell_command_palette_lines(conversation));
        return lines;
    }

    // Complete shell commands get command-specific guidance instead of generic prompt-send copy.
    if let Some(command) = InlineShellCommandInput::parse(&conversation.input_buffer) {
        lines.push(Line::from(command.buffered_hint()));
        return lines;
    }

    // Manual typing during live auto-follow should be allowed, but Enter must wait until the auto turn ends.
    if conversation.auto_follow_state.has_live_activity()
        && conversation.input_state.can_submit_now()
    {
        lines.push(Line::from(
            "Prompt buffered. Ctrl+j inserts a new line. Press Enter when auto follow-up finishes.",
        ));
        return lines;
    }

    // Non-empty regular prompts fall through to submit timing guidance.
    match (conversation.input_state, shell_action_availability) {
        (
            ConversationInputState::DraftReady | ConversationInputState::ReadyToContinue,
            ShellActionAvailability::Pending,
        ) if conversation.startup_submit_armed => {
            lines.push(Line::from("Prompt queued until startup checks finish."));
            lines.push(Line::from(
                "Ctrl+j inserts a new line. Editing cancels the queued send.",
            ));
        }
        (ConversationInputState::DraftReady, ShellActionAvailability::Ready) => {
            lines.push(Line::from(
                "Press Enter to create thread and send. Ctrl+j inserts a new line.",
            ));
        }
        (ConversationInputState::ReadyToContinue, ShellActionAvailability::Ready) => {
            lines.push(Line::from(
                "Press Enter to send this prompt. Ctrl+j inserts a new line.",
            ));
        }
        (ConversationInputState::DraftReady | ConversationInputState::ReadyToContinue, _) => {
            lines.push(Line::from(
                "Prompt buffered. Ctrl+j inserts a new line. Press Enter after startup diagnostics turn ready.",
            ));
        }
        (ConversationInputState::SubmittingTurn, _)
        | (ConversationInputState::StreamingTurn, _) => {
            lines.push(Line::from(
                "Prompt buffered. Ctrl+j inserts a new line. Press Enter when turn ends.",
            ));
        }
    }

    lines
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

#[cfg(test)]
pub(in super::super) fn build_input_prompt_cursor_offset(
    app: &NativeTuiApp,
    content_width: u16,
) -> Option<(u16, u16)> {
    // Tests call through the app boundary to verify renderer-facing cursor semantics.
    let ConversationState::Ready(conversation) = &app.conversation_state else {
        return None;
    };

    build_prompt_cursor_offset(conversation, content_width)
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
    let buffer_lines = conversation.input_buffer.split('\n').collect::<Vec<_>>();
    let mut lines = Vec::with_capacity(buffer_lines.len().max(1));
    let mut cursor_line_index = 0;
    let mut cursor_column = 0;

    for (index, buffer_line) in buffer_lines.iter().enumerate() {
        let line = if index == 0 {
            Line::from(vec![
                Span::raw(PROMPT_PRIMARY_PREFIX),
                Span::raw((*buffer_line).to_string()),
            ])
        } else {
            Line::from(vec![
                Span::raw(PROMPT_CONTINUATION_PREFIX),
                Span::raw((*buffer_line).to_string()),
            ])
        };
        if index + 1 == buffer_lines.len() {
            cursor_line_index = index;
            cursor_column = line.width();
        }
        lines.push(line);
    }

    PromptBufferView {
        lines,
        cursor_line_index,
        cursor_column,
    }
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
