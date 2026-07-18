use crate::adapter::inbound::tui::app::conversation_model::{
    ProgressiveActivityExpandState, tool_message_digest, tool_message_fact,
    tool_message_is_expandable, tool_message_title,
};
use crate::adapter::inbound::tui::conversation_text::conversation_message_label;

use super::{
    AkraTheme, ConversationMessage, ConversationMessageKind, ConversationViewMode, Line,
    MAX_CONVERSATION_HISTORY_LINES, Modifier, Span, Style,
};

// Default transcript formatting is user-facing: message debug detail stays hidden unless a debug-aware caller opts in.
pub(in super::super) fn format_conversation_lines(
    messages: &[ConversationMessage],
) -> Vec<Line<'static>> {
    format_conversation_lines_for_view(messages, ConversationViewMode::Medium, false)
}

pub(in super::super) fn format_conversation_lines_for_view(
    messages: &[ConversationMessage],
    view_mode: ConversationViewMode,
    show_debug_details: bool,
) -> Vec<Line<'static>> {
    format_conversation_lines_capped(messages, view_mode, show_debug_details, None)
}

#[cfg(test)]
pub(in super::super) fn format_conversation_lines_with_debug(
    messages: &[ConversationMessage],
    // Debug detail is operator-only transcript copy and stays out of cached/default message lines.
    show_debug_details: bool,
) -> Vec<Line<'static>> {
    let view_mode = if show_debug_details {
        ConversationViewMode::Detail
    } else {
        ConversationViewMode::Medium
    };
    format_conversation_lines_capped(messages, view_mode, show_debug_details, None)
}

pub(in super::super) fn format_conversation_scrollback_lines_with_expand(
    messages: &[ConversationMessage],
    view_mode: ConversationViewMode,
    show_debug_details: bool,
    expand_state: Option<&ProgressiveActivityExpandState>,
) -> Vec<Line<'static>> {
    format_conversation_lines_uncapped(messages, view_mode, show_debug_details, expand_state)
}

fn format_conversation_lines_capped(
    messages: &[ConversationMessage],
    view_mode: ConversationViewMode,
    show_debug_details: bool,
    expand_state: Option<&ProgressiveActivityExpandState>,
) -> Vec<Line<'static>> {
    let mut lines =
        format_conversation_lines_uncapped(messages, view_mode, show_debug_details, expand_state);

    // Keep recent terminal history bounded; rendering and inline tail logic operate on this capped line buffer.
    if lines.len() > MAX_CONVERSATION_HISTORY_LINES {
        lines.drain(0..lines.len() - MAX_CONVERSATION_HISTORY_LINES);
    }

    lines
}

// Project logical conversation messages into terminal transcript lines.
// Each message becomes a styled label, indented body/debug lines, and a blank separator so history reads as blocks.
fn format_conversation_lines_uncapped(
    messages: &[ConversationMessage],
    view_mode: ConversationViewMode,
    show_debug_details: bool,
    expand_state: Option<&ProgressiveActivityExpandState>,
) -> Vec<Line<'static>> {
    let mut lines = Vec::new();

    for message in messages {
        if !view_mode.includes_message(message) {
            continue;
        }

        if message.kind == ConversationMessageKind::Tool {
            lines.extend(format_tool_card_lines(message, view_mode, expand_state));
            lines.push(Line::from(""));
            continue;
        }

        // Labels use the shared conversation_text helper so transcript, approval, and other surfaces name speakers alike.
        let label = conversation_message_label(message);
        lines.push(Line::from(Span::styled(
            format!("{label}:"),
            label_style(message.kind),
        )));

        // Preserve author line breaks but indent body rows under the label; tabs are normalized for stable TUI width.
        let mut in_markdown_code_fence = false;
        for text_line in message.text.lines() {
            lines.push(format_markdown_body_line(
                text_line,
                &mut in_markdown_code_fence,
            ));
        }

        // Debug rows follow the body in the same block, but muted style keeps them visually secondary.
        if show_debug_details && let Some(debug_detail) = message.debug_detail.as_deref() {
            for detail_line in debug_detail.lines() {
                lines.push(Line::from(Span::styled(
                    format!("  {}", expand_tui_tabs(detail_line)),
                    AkraTheme::muted(),
                )));
            }
        }

        // Separator participates in history capping so rendered scroll height matches what the user sees.
        lines.push(Line::from(""));
    }

    // Empty threads still need visible transcript content so the panel does not look broken.
    if lines.is_empty() {
        let empty_message = if messages.is_empty() {
            "No messages in this thread yet.".to_string()
        } else {
            format!("No messages visible in {} view.", view_mode.label())
        };
        lines.push(Line::from(empty_message));
    }

    lines
}

fn format_tool_card_lines(
    message: &ConversationMessage,
    view_mode: ConversationViewMode,
    expand_state: Option<&ProgressiveActivityExpandState>,
) -> Vec<Line<'static>> {
    let expandable = tool_message_is_expandable(&message.text);
    let digest = tool_message_digest(&message.text);
    // Detail view expands multi-line tool cards by default; Medium keeps them collapsed
    // unless the operator toggled the card. Single-line tools stay header-only.
    let expanded = expandable && expand_state.is_some_and(|state| state.is_tool_expanded(digest))
        || expandable && matches!(view_mode, ConversationViewMode::Detail);

    let indicator = if !expandable {
        AkraTheme::non_expandable_indicator()
    } else if expanded {
        AkraTheme::expanded_indicator()
    } else {
        AkraTheme::collapsed_indicator()
    };
    let title = tool_message_title(&message.text);
    let fact = tool_message_fact(&message.text);
    let label = message
        .display_label
        .as_deref()
        .filter(|label| !label.trim().is_empty())
        .unwrap_or("tool");

    let mut lines = vec![Line::from(vec![
        Span::styled(
            indicator.to_string(),
            if expandable {
                AkraTheme::expandable_indicator()
            } else {
                AkraTheme::subtle()
            },
        ),
        Span::styled(
            AkraTheme::tool_card_bullet_glyph().to_string(),
            AkraTheme::tool_card_bullet(),
        ),
        Span::styled(format!("{label:<9} "), AkraTheme::tool_card_header()),
        Span::styled(title, AkraTheme::tool_card_header()),
        if fact.is_empty() {
            Span::raw(String::new())
        } else {
            Span::styled(format!("  {fact}"), AkraTheme::muted())
        },
    ])];

    if expanded {
        let mut in_markdown_code_fence = false;
        for text_line in message.text.lines() {
            let mut body = format_markdown_body_line(text_line, &mut in_markdown_code_fence);
            // Dim every non-indent span so multi-span markdown tool bodies stay consistent.
            for span in body.spans.iter_mut().skip(1) {
                span.style = span.style.patch(AkraTheme::tool_card_body());
            }
            lines.push(body);
        }
    }

    lines
}

// Normalize tabs before ratatui width/layout calculations so transcript alignment is terminal-independent.
fn expand_tui_tabs(text: &str) -> String {
    text.replace('\t', "    ")
}

fn format_markdown_body_line(text: &str, in_markdown_code_fence: &mut bool) -> Line<'static> {
    let expanded = expand_tui_tabs(text);
    let mut spans = vec![Span::raw("  ")];
    if is_markdown_code_fence(&expanded) {
        *in_markdown_code_fence = !*in_markdown_code_fence;
        spans.push(Span::styled(expanded, AkraTheme::markdown_fence()));
        return Line::from(spans);
    }
    if *in_markdown_code_fence {
        spans.push(Span::styled(expanded, AkraTheme::markdown_code_block()));
        return Line::from(spans);
    }
    let block = markdown_block_line(&expanded);
    spans.extend(block.prefix_spans);
    spans.extend(format_markdown_inline_spans(block.body, block.body_style));
    Line::from(spans)
}

fn is_markdown_code_fence(text: &str) -> bool {
    let trimmed = text.trim_start();
    trimmed.starts_with("```") || trimmed.starts_with("~~~")
}

fn markdown_heading_body(text: &str) -> Option<&str> {
    let marker_len = text.chars().take_while(|ch| *ch == '#').count();
    if !(1..=6).contains(&marker_len) {
        return None;
    }
    let marker_bytes = marker_len;
    text.get(marker_bytes..)
        .and_then(|tail| tail.strip_prefix(' '))
        .filter(|tail| !tail.trim().is_empty())
}

struct MarkdownBlockLine<'a> {
    prefix_spans: Vec<Span<'static>>,
    body: &'a str,
    body_style: Style,
}

fn markdown_block_line(text: &str) -> MarkdownBlockLine<'_> {
    if let Some(heading) = markdown_heading_body(text) {
        return MarkdownBlockLine {
            prefix_spans: Vec::new(),
            body: heading,
            body_style: AkraTheme::markdown_heading(),
        };
    }
    if markdown_horizontal_rule(text) {
        return MarkdownBlockLine {
            prefix_spans: Vec::new(),
            body: text,
            body_style: AkraTheme::markdown_fence(),
        };
    }

    let leading_len = text.len() - text.trim_start().len();
    let leading = &text[..leading_len];
    let tail = &text[leading_len..];
    if let Some((marker, body)) = markdown_quote_parts(tail) {
        return MarkdownBlockLine {
            prefix_spans: markdown_prefix_spans(leading, marker, AkraTheme::markdown_quote()),
            body,
            body_style: AkraTheme::markdown_quote(),
        };
    }
    if let Some((marker, body)) = markdown_list_parts(tail) {
        return MarkdownBlockLine {
            prefix_spans: markdown_prefix_spans(leading, marker, AkraTheme::markdown_marker()),
            body,
            body_style: Style::default(),
        };
    }

    MarkdownBlockLine {
        prefix_spans: Vec::new(),
        body: text,
        body_style: Style::default(),
    }
}

fn markdown_horizontal_rule(text: &str) -> bool {
    let trimmed = text.trim();
    trimmed.len() >= 3 && trimmed.chars().all(|ch| matches!(ch, '-' | '*' | '_'))
}

fn markdown_quote_parts(text: &str) -> Option<(&str, &str)> {
    text.strip_prefix("> ")
        .map(|body| ("> ", body))
        .or_else(|| text.strip_prefix('>').map(|body| (">", body)))
}

fn markdown_list_parts(text: &str) -> Option<(&str, &str)> {
    if text.starts_with("- ") || text.starts_with("+ ") || text.starts_with("* ") {
        return Some((&text[..2], &text[2..]));
    }
    let digit_count = text.chars().take_while(|ch| ch.is_ascii_digit()).count();
    if digit_count == 0 {
        return None;
    }
    let marker_tail = &text[digit_count..];
    if marker_tail.starts_with(". ") || marker_tail.starts_with(") ") {
        let marker_len = digit_count + 2;
        return Some((&text[..marker_len], &text[marker_len..]));
    }
    None
}

fn markdown_prefix_spans(leading: &str, marker: &str, marker_style: Style) -> Vec<Span<'static>> {
    let mut spans = Vec::new();
    if !leading.is_empty() {
        spans.push(Span::raw(leading.to_string()));
    }
    spans.push(Span::styled(marker.to_string(), marker_style));
    spans
}

fn format_markdown_inline_spans(text: &str, base_style: Style) -> Vec<Span<'static>> {
    let mut spans = Vec::new();
    let mut remaining = text;
    while !remaining.is_empty() {
        if let Some((marker, marker_style)) = next_markdown_marker(remaining, base_style)
            && let Some(after_open) = remaining.strip_prefix(marker)
            && let Some(close_index) = after_open.find(marker)
        {
            let content = &after_open[..close_index];
            if !content.is_empty() {
                spans.push(Span::styled(content.to_string(), marker_style));
                remaining = &after_open[close_index + marker.len()..];
                continue;
            }
        }

        let Some((prefix, marker_start)) = split_before_next_markdown_marker(remaining) else {
            spans.push(Span::styled(remaining.to_string(), base_style));
            break;
        };
        if !prefix.is_empty() {
            spans.push(Span::styled(prefix.to_string(), base_style));
            remaining = &remaining[marker_start..];
            continue;
        }

        let (first_char_end, first_char) = remaining
            .char_indices()
            .nth(1)
            .map(|(index, _)| (index, &remaining[..index]))
            .unwrap_or((remaining.len(), remaining));
        spans.push(Span::styled(first_char.to_string(), base_style));
        remaining = &remaining[first_char_end..];
    }
    if spans.is_empty() {
        spans.push(Span::raw(String::new()));
    }
    spans
}

fn next_markdown_marker(text: &str, base_style: Style) -> Option<(&'static str, Style)> {
    if text.starts_with("**") {
        return Some(("**", markdown_bold_style(base_style)));
    }
    if text.starts_with("__") {
        return Some(("__", markdown_bold_style(base_style)));
    }
    if text.starts_with('`') {
        return Some(("`", AkraTheme::markdown_inline_code()));
    }
    None
}

fn split_before_next_markdown_marker(text: &str) -> Option<(&str, usize)> {
    let candidates = ["**", "__", "`"];
    candidates
        .iter()
        .filter_map(|marker| text.find(marker).map(|index| (index, *marker)))
        .min_by_key(|(index, _)| *index)
        .map(|(index, _)| (&text[..index], index))
}

fn markdown_bold_style(base_style: Style) -> Style {
    if base_style == Style::default() {
        AkraTheme::markdown_emphasis()
    } else {
        base_style.add_modifier(Modifier::BOLD)
    }
}

// Speaker labels keep the strongest style; body markdown uses smaller inline emphasis so prose and logs remain readable.
fn label_style(kind: ConversationMessageKind) -> Style {
    match kind {
        ConversationMessageKind::User => AkraTheme::shortcut(),
        ConversationMessageKind::Agent => AkraTheme::brand(),
        ConversationMessageKind::Tool => AkraTheme::tool(),
        ConversationMessageKind::Status => AkraTheme::muted(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tool_messages_collapse_to_one_line_headers_in_medium_view() {
        let messages = vec![ConversationMessage::new(
            ConversationMessageKind::Tool,
            "cargo test\nline two\nline three",
            None,
            Some("tool-1".to_string()),
        )];

        let medium =
            format_conversation_lines_for_view(&messages, ConversationViewMode::Medium, false);
        let medium_text = medium.iter().map(line_text).collect::<Vec<_>>().join("\n");
        assert!(medium_text.contains("› "));
        assert!(medium_text.contains("◆ "));
        assert!(medium_text.contains("cargo test"));
        assert!(medium_text.contains("3 lines"));
        assert!(!medium_text.contains("line two"));

        let detail =
            format_conversation_lines_for_view(&messages, ConversationViewMode::Detail, false);
        let detail_text = detail.iter().map(line_text).collect::<Vec<_>>().join("\n");
        assert!(detail_text.contains("▼ "));
        assert!(detail_text.contains("line two"));
        assert!(detail_text.contains("line three"));
    }

    #[test]
    fn transcript_body_renders_basic_markdown_markers() {
        let messages = vec![ConversationMessage::new(
            ConversationMessageKind::Agent,
            "# Summary\n**Changed** and `cargo test`",
            Some("final_answer".to_string()),
            Some("agent-1".to_string()),
        )];

        let lines =
            format_conversation_lines_for_view(&messages, ConversationViewMode::Medium, false);

        assert_eq!(line_text(&lines[1]), "  Summary");
        assert_eq!(lines[1].spans[1].style, AkraTheme::markdown_heading());
        assert_eq!(line_text(&lines[2]), "  Changed and cargo test");
        assert_eq!(lines[2].spans[1].style, AkraTheme::markdown_emphasis());
        assert_eq!(lines[2].spans[3].style, AkraTheme::markdown_inline_code());
    }

    #[test]
    fn transcript_body_colors_markdown_block_markers() {
        let messages = vec![ConversationMessage::new(
            ConversationMessageKind::Agent,
            "**수정사항**\n- 변경한 파일 없음\n1. 다음 단계\n> 참고",
            Some("final_answer".to_string()),
            Some("agent-1".to_string()),
        )];

        let lines =
            format_conversation_lines_for_view(&messages, ConversationViewMode::Medium, false);

        assert_eq!(line_text(&lines[1]), "  수정사항");
        assert_eq!(lines[1].spans[1].style, AkraTheme::markdown_emphasis());
        assert_eq!(line_text(&lines[2]), "  - 변경한 파일 없음");
        assert_eq!(lines[2].spans[1].style, AkraTheme::markdown_marker());
        assert_eq!(line_text(&lines[3]), "  1. 다음 단계");
        assert_eq!(lines[3].spans[1].style, AkraTheme::markdown_marker());
        assert_eq!(line_text(&lines[4]), "  > 참고");
        assert_eq!(lines[4].spans[1].style, AkraTheme::markdown_quote());
        assert!(
            lines[4].spans[2]
                .style
                .add_modifier
                .contains(Modifier::ITALIC)
        );
    }

    #[test]
    fn transcript_body_preserves_markdown_inside_code_fences() {
        let messages = vec![ConversationMessage::new(
            ConversationMessageKind::Agent,
            "```rust\nlet literal = \"**keep markers**\";\n```",
            Some("final_answer".to_string()),
            Some("agent-1".to_string()),
        )];

        let lines =
            format_conversation_lines_for_view(&messages, ConversationViewMode::Medium, false);

        assert_eq!(line_text(&lines[1]), "  ```rust");
        assert_eq!(lines[1].spans[1].style, AkraTheme::markdown_fence());
        assert_eq!(
            line_text(&lines[2]),
            "  let literal = \"**keep markers**\";"
        );
        assert_eq!(lines[2].spans[1].style, AkraTheme::markdown_code_block());
        assert_eq!(line_text(&lines[3]), "  ```");
        assert_eq!(lines[3].spans[1].style, AkraTheme::markdown_fence());
    }

    fn line_text(line: &Line<'_>) -> String {
        line.spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect::<String>()
    }
}
