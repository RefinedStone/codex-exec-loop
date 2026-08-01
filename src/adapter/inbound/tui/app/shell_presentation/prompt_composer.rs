use super::*;
use ratatui::buffer::Buffer;
use ratatui::style::Modifier;
use ratatui::widgets::{Paragraph, Widget, Wrap};
use unicode_segmentation::UnicodeSegmentation;

/*
prompt_composer는 shell footer의 입력 영역을 만드는 presentation adapter다.
conversation reducer가 가진 raw buffer와 input state를 읽어 "지금 사용자가 무엇을 할 수 있는지"를
line copy로 바꾸고, renderer가 cursor를 올바른 terminal row에 놓을 수 있도록 같은 buffer projection을
좌표 계산에도 재사용한다.
*/
const PROMPT_PRIMARY_PREFIX: &str = " > ";
const PROMPT_CONTINUATION_PREFIX: &str = "   ";

pub(super) struct PromptBufferView {
    // Prompt text is already split into ratatui Lines so popup and shell tail renderers share one projection.
    pub(super) lines: Vec<Line<'static>>,
}

pub(super) fn build_shell_command_palette_lines(
    composer: &ConversationComposerScreenModel<'_>,
    capabilities: &InlineShellCommandCapabilitySet,
    language: TuiLanguage,
    content_width: u16,
) -> Vec<Line<'static>> {
    let palette_state = &composer.state.inline_shell_command_palette_state;
    // Dismissed palettes should leave the typed buffer visible without suggestion rows.
    if !palette_state.is_active() {
        return Vec::new();
    }
    // Suggestion prefix is only present while the user is typing the command token, not arguments.
    let Some(prefix) = InlineShellCommand::suggestion_prefix(&composer.state.input_buffer) else {
        return Vec::new();
    };
    // Empty results still render feedback so the user knows the palette is active and filtering.
    if palette_state.suggestions().is_empty() {
        return vec![Line::from(format!(
            "  {}",
            language.inline_command_palette_no_matches(&prefix)
        ))];
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
    let effective_width = if content_width == 0 {
        120
    } else {
        content_width
    };
    let show_badge = effective_width >= 36;
    let show_description = effective_width >= 72;
    let mut lines = suggestions[window_start..window_end]
        .iter()
        .enumerate()
        .map(|(offset, command)| {
            let is_selected = selected_index == window_start + offset;
            let selector = if is_selected { "> " } else { "  " };
            let availability = capabilities.availability(*command);
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
            let availability_style = match availability {
                InlineShellCommandAvailability::Ready => AkraTheme::success(),
                InlineShellCommandAvailability::Pending(_) => AkraTheme::warning(),
                InlineShellCommandAvailability::Locked(_) => AkraTheme::subtle(),
            };
            let mut spans = vec![
                Span::raw(selector),
                Span::styled(command.command_name(), label_style),
            ];
            if show_badge {
                spans.push(Span::raw("  "));
                spans.push(Span::styled(
                    language.inline_command_availability_label(availability),
                    availability_style.add_modifier(Modifier::BOLD),
                ));
            }
            if show_description {
                spans.push(Span::raw("  "));
                spans.push(Span::styled(
                    command.suggestion_detail(language),
                    detail_style,
                ));
            }
            Line::from(spans)
        })
        .collect::<Vec<_>>();

    if let Some(command) = palette_state.selected_command() {
        lines.push(build_shell_command_palette_detail_line(
            command,
            capabilities.availability(command),
            capabilities.parallel_mode_enabled(),
            language,
            effective_width,
        ));
    }
    lines
}

fn build_shell_command_palette_detail_line(
    command: InlineShellCommand,
    availability: InlineShellCommandAvailability,
    parallel_mode_enabled: bool,
    language: TuiLanguage,
    content_width: u16,
) -> Line<'static> {
    let availability_label = language.inline_command_availability_label(availability);
    let availability_style = match availability {
        InlineShellCommandAvailability::Ready => AkraTheme::success(),
        InlineShellCommandAvailability::Pending(_) => AkraTheme::warning(),
        InlineShellCommandAvailability::Locked(_) => AkraTheme::subtle(),
    }
    .add_modifier(Modifier::BOLD);
    let reason = availability
        .reason()
        .map(|reason| language.inline_command_availability_reason(reason));

    if content_width < 64 {
        let detail = reason.unwrap_or_else(|| {
            language.inline_command_expected_result(command, parallel_mode_enabled)
        });
        return Line::from(vec![
            Span::styled(
                format!("  {}  ", language.inline_command_palette_detail_label()),
                AkraTheme::subtle(),
            ),
            Span::styled(availability_label, availability_style),
            Span::raw(" · "),
            Span::raw(detail),
        ]);
    }

    let mut spans = vec![Span::styled(
        format!("  {}  ", language.inline_command_palette_detail_label()),
        AkraTheme::subtle(),
    )];
    if content_width >= 100 {
        spans.push(Span::styled(
            format!("{} ", language.inline_command_palette_args_label()),
            AkraTheme::subtle(),
        ));
    }
    spans.push(Span::raw(language.inline_command_argument_preview(command)));
    spans.push(Span::styled(" → ", AkraTheme::subtle()));
    spans.push(Span::raw(
        language.inline_command_expected_result(command, parallel_mode_enabled),
    ));
    spans.push(Span::raw(" · "));
    spans.push(Span::styled(availability_label, availability_style));
    if let Some(reason) = reason {
        spans.push(Span::raw(" · "));
        spans.push(Span::raw(reason));
    }
    Line::from(spans)
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
    composer: &ConversationComposerScreenModel<'_>,
    content_width: u16,
) -> Option<(u16, u16)> {
    // A zero-width area means the renderer cannot place a cursor safely.
    if content_width == 0 {
        return None;
    }
    locate_prompt_cursor_with_word_wrap(composer, content_width)
}

fn locate_prompt_cursor_with_word_wrap(
    composer: &ConversationComposerScreenModel<'_>,
    content_width: u16,
) -> Option<(u16, u16)> {
    let cursor_byte_index = composer.state.input_cursor_byte_index();
    let cursor_prefix = &composer.state.input_buffer[..cursor_byte_index];
    let mut prefix_lines = cursor_prefix
        .split('\n')
        .enumerate()
        .map(|(index, line)| {
            Line::from(vec![
                Span::raw(prompt_line_prefix(index)),
                Span::raw(line.to_string()),
            ])
        })
        .collect::<Vec<_>>();
    let marker_modifier = Modifier::SLOW_BLINK | Modifier::RAPID_BLINK;
    let marker_style = Style::default().add_modifier(marker_modifier);
    prefix_lines
        .last_mut()?
        .spans
        .push(Span::styled(" ", marker_style));
    let estimated_cursor_row = Paragraph::new(prefix_lines)
        .wrap(Wrap { trim: false })
        .line_count(content_width)
        .saturating_sub(1)
        .min(usize::from(u16::MAX)) as u16;

    let cursor_line_index = cursor_prefix.matches('\n').count();
    let cursor_line_start = cursor_prefix
        .rfind('\n')
        .map(|index| index.saturating_add(1))
        .unwrap_or(0);
    let cursor_byte_in_rendered_line = prompt_line_prefix(cursor_line_index)
        .len()
        .saturating_add(cursor_byte_index.saturating_sub(cursor_line_start));
    let mut marked_lines = build_prompt_buffer_view(composer).lines;
    let rendered_line = marked_lines[cursor_line_index].to_string();
    let (before_cursor, from_cursor) = rendered_line.split_at(cursor_byte_in_rendered_line);
    marked_lines[cursor_line_index] =
        if let Some(next_grapheme) = from_cursor.graphemes(true).next() {
            let next_grapheme_end = next_grapheme.len();
            Line::from(vec![
                Span::raw(before_cursor.to_string()),
                Span::styled(from_cursor[..next_grapheme_end].to_string(), marker_style),
                Span::raw(from_cursor[next_grapheme_end..].to_string()),
            ])
        } else {
            Line::from(vec![
                Span::raw(before_cursor.to_string()),
                Span::styled(" ", marker_style),
            ])
        };

    let search_start = estimated_cursor_row.saturating_sub(1);
    let search_height = 3;
    let area = Rect::new(0, 0, content_width, search_height);
    let mut buffer = Buffer::empty(area);
    Paragraph::new(marked_lines)
        .scroll((search_start, 0))
        .wrap(Wrap { trim: false })
        .render(area, &mut buffer);

    for y in 0..search_height {
        for x in 0..content_width {
            let cell = &buffer[(x, y)];
            if cell.modifier.contains(marker_modifier) {
                return Some((x, search_start.saturating_add(y)));
            }
        }
    }
    None
}

pub(super) fn build_prompt_buffer_view(
    composer: &ConversationComposerScreenModel<'_>,
) -> PromptBufferView {
    build_prompt_buffer_view_with_optional_placeholder(composer, None)
}

fn build_prompt_buffer_view_with_optional_placeholder(
    composer: &ConversationComposerScreenModel<'_>,
    placeholder: Option<&str>,
) -> PromptBufferView {
    /*
    Prefixes are part of the prompt projection so rendered input and cursor probes share the same copy.
    */
    let buffer_lines = composer.state.input_buffer.split('\n').collect::<Vec<_>>();
    let mut lines = Vec::with_capacity(buffer_lines.len().max(1));

    for (index, buffer_line) in buffer_lines.iter().enumerate() {
        let prefix = prompt_line_prefix(index);
        let content = if index == 0 && buffer_line.is_empty() {
            placeholder
                .map(|placeholder| Span::styled(placeholder.to_string(), AkraTheme::subtle()))
                .unwrap_or_else(|| Span::raw(""))
        } else {
            Span::raw((*buffer_line).to_string())
        };
        let line = Line::from(vec![Span::styled(prefix, AkraTheme::brand()), content]);
        lines.push(line);
    }

    PromptBufferView { lines }
}

fn prompt_line_prefix(line_index: usize) -> &'static str {
    if line_index == 0 {
        PROMPT_PRIMARY_PREFIX
    } else {
        PROMPT_CONTINUATION_PREFIX
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

#[cfg(test)]
mod tests {
    use super::*;

    fn composer_screen_model(
        conversation: &ConversationViewModel,
    ) -> ConversationComposerScreenModel<'_> {
        ConversationComposerScreenModel::from_conversation(conversation)
    }

    #[test]
    fn prompt_cursor_offset_uses_conversation_cursor_position() {
        let mut conversation = ConversationViewModel::new_draft("/tmp/root".to_string());
        conversation.composer.input_buffer = "hello".to_string();
        conversation.composer.set_input_cursor_byte_index(2);

        assert_eq!(
            build_prompt_cursor_offset(&composer_screen_model(&conversation), 80),
            Some((5, 0))
        );
    }

    #[test]
    fn prompt_cursor_offset_handles_multiline_middle_cursor() {
        let mut conversation = ConversationViewModel::new_draft("/tmp/root".to_string());
        conversation.composer.input_buffer = "one\ntwo".to_string();
        conversation
            .composer
            .set_input_cursor_byte_index("one\n".len() + 1);

        assert_eq!(
            build_prompt_cursor_offset(&composer_screen_model(&conversation), 80),
            Some((4, 1))
        );
    }

    #[test]
    fn prompt_cursor_probe_handles_combining_and_zwj_graphemes() {
        let mut conversation = ConversationViewModel::new_draft("/tmp/root".to_string());
        conversation.composer.input_buffer = "before e\u{301} 👩‍💻 after".to_string();
        let emoji_start = conversation
            .composer
            .input_buffer
            .find("👩‍💻")
            .expect("emoji fixture should exist");
        conversation
            .composer
            .set_input_cursor_byte_index(emoji_start);

        assert_eq!(
            build_prompt_cursor_offset(&composer_screen_model(&conversation), 12),
            Some((0, 1))
        );
    }

    #[test]
    fn contextual_palette_folds_description_then_badge_without_losing_command() {
        let mut conversation = ConversationViewModel::new_draft("/tmp/root".to_string());
        conversation.composer.input_buffer = ":pe".to_string();
        conversation.composer.sync_inline_shell_command_palette();
        let capabilities = InlineShellCommandCapabilitySet::default();
        let composer = composer_screen_model(&conversation);

        let wide =
            build_shell_command_palette_lines(&composer, &capabilities, TuiLanguage::English, 120);
        assert_eq!(wide.len(), 2);
        assert!(
            wide[0]
                .to_string()
                .contains(":peek  LOCKED  parallel agent peek")
        );
        assert!(
            wide[1]
                .to_string()
                .contains("none → inspect active agent work")
        );
        assert!(wide[1].to_string().contains("start parallel mode first"));

        let compact =
            build_shell_command_palette_lines(&composer, &capabilities, TuiLanguage::English, 64);
        assert!(compact[0].to_string().contains(":peek  LOCKED"));
        assert!(!compact[0].to_string().contains("parallel agent peek"));

        let minimal =
            build_shell_command_palette_lines(&composer, &capabilities, TuiLanguage::English, 32);
        assert_eq!(minimal[0].to_string(), "> :peek");
        assert!(minimal[1].to_string().contains("LOCKED"));
    }
}
