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
    format_conversation_lines_capped(messages, view_mode, show_debug_details)
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
    format_conversation_lines_capped(messages, view_mode, show_debug_details)
}

pub(in super::super) fn format_conversation_scrollback_lines(
    messages: &[ConversationMessage],
    view_mode: ConversationViewMode,
    show_debug_details: bool,
) -> Vec<Line<'static>> {
    format_conversation_lines_uncapped(messages, view_mode, show_debug_details)
}

fn format_conversation_lines_capped(
    messages: &[ConversationMessage],
    view_mode: ConversationViewMode,
    show_debug_details: bool,
) -> Vec<Line<'static>> {
    let mut lines = format_conversation_lines_uncapped(messages, view_mode, show_debug_details);

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
) -> Vec<Line<'static>> {
    let mut lines = Vec::new();

    for message in messages {
        if !view_mode.includes_message(message) {
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

// Normalize tabs before ratatui width/layout calculations so transcript alignment is terminal-independent.
fn expand_tui_tabs(text: &str) -> String {
    text.replace('\t', "    ")
}

fn format_markdown_body_line(text: &str, in_markdown_code_fence: &mut bool) -> Line<'static> {
    let expanded = expand_tui_tabs(text);
    let mut spans = vec![Span::raw("  ")];
    if is_markdown_code_fence(&expanded) {
        *in_markdown_code_fence = !*in_markdown_code_fence;
        spans.push(Span::raw(expanded));
        return Line::from(spans);
    }
    if *in_markdown_code_fence {
        spans.push(Span::raw(expanded));
        return Line::from(spans);
    }
    let (body, base_style) = markdown_heading_body(&expanded)
        .map(|heading| (heading.to_string(), markdown_bold_style(Style::default())))
        .unwrap_or((expanded, Style::default()));
    spans.extend(format_markdown_inline_spans(&body, base_style));
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
        return Some(("`", AkraTheme::tool()));
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
    base_style.add_modifier(Modifier::BOLD)
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
        assert!(
            lines[1].spans[1]
                .style
                .add_modifier
                .contains(Modifier::BOLD)
        );
        assert_eq!(line_text(&lines[2]), "  Changed and cargo test");
        assert!(
            lines[2].spans[1]
                .style
                .add_modifier
                .contains(Modifier::BOLD)
        );
        assert_eq!(lines[2].spans[3].style, AkraTheme::tool());
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
        assert_eq!(
            line_text(&lines[2]),
            "  let literal = \"**keep markers**\";"
        );
        assert_eq!(line_text(&lines[3]), "  ```");
    }

    fn line_text(line: &Line<'_>) -> String {
        line.spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect::<String>()
    }
}
