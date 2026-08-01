use crate::adapter::inbound::tui::app::conversation_model::{
    ProgressiveActivityExpandState, tool_message_digest, tool_message_fact,
    tool_message_is_expandable, tool_message_title,
};
use crate::adapter::inbound::tui::conversation_text::conversation_message_label;

use super::overlays::build_inline_diff_preview;
use super::{
    AkraTheme, ConversationMessage, ConversationMessageKind, ConversationViewMode, Line, Modifier,
    Span, Style,
};

const INLINE_DIFF_PREVIEW_ROWS: u16 = 8;
const INLINE_TOOL_DETAIL_ROWS: usize = 8;
#[cfg(test)]
const DEFAULT_TRANSCRIPT_WIDTH: u16 = 120;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in super::super) struct ConversationTranscriptCardRow {
    pub(in super::super) line_index: usize,
    pub(in super::super) digest: [u8; 32],
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(in super::super) struct ConversationTranscriptView {
    pub(in super::super) lines: Vec<Line<'static>>,
    pub(in super::super) card_rows: Vec<ConversationTranscriptCardRow>,
}

#[cfg(test)]
pub(in super::super) fn format_conversation_lines_for_view(
    messages: &[ConversationMessage],
    view_mode: ConversationViewMode,
    show_debug_details: bool,
) -> Vec<Line<'static>> {
    format_conversation_projection_uncapped(
        messages,
        view_mode,
        show_debug_details,
        None,
        DEFAULT_TRANSCRIPT_WIDTH,
    )
    .lines
}

#[cfg(test)]
pub(in super::super) fn format_fullscreen_conversation_lines_with_expand_at_width(
    messages: &[ConversationMessage],
    view_mode: ConversationViewMode,
    show_debug_details: bool,
    expand_state: Option<&ProgressiveActivityExpandState>,
    width: u16,
) -> Vec<Line<'static>> {
    format_conversation_projection_uncapped(
        messages,
        view_mode,
        show_debug_details,
        expand_state,
        width,
    )
    .lines
}

pub(in super::super) fn format_fullscreen_conversation_transcript_view(
    messages: &[ConversationMessage],
    view_mode: ConversationViewMode,
    show_debug_details: bool,
    expand_state: &ProgressiveActivityExpandState,
    width: u16,
) -> ConversationTranscriptView {
    format_conversation_projection_uncapped(
        messages,
        view_mode,
        show_debug_details,
        Some(expand_state),
        width,
    )
}

// Project logical conversation messages into terminal transcript lines.
// Each message becomes a styled label, indented body/debug lines, and a blank separator so history reads as blocks.
fn format_conversation_projection_uncapped(
    messages: &[ConversationMessage],
    view_mode: ConversationViewMode,
    show_debug_details: bool,
    expand_state: Option<&ProgressiveActivityExpandState>,
    width: u16,
) -> ConversationTranscriptView {
    let mut projection = ConversationTranscriptView {
        lines: Vec::new(),
        card_rows: Vec::new(),
    };
    append_conversation_messages(
        &mut projection,
        messages,
        view_mode,
        show_debug_details,
        expand_state,
        width,
    );
    append_empty_transcript_message(&mut projection.lines, messages.len(), view_mode);
    projection
}

fn append_conversation_messages(
    projection: &mut ConversationTranscriptView,
    messages: &[ConversationMessage],
    view_mode: ConversationViewMode,
    show_debug_details: bool,
    expand_state: Option<&ProgressiveActivityExpandState>,
    width: u16,
) {
    let lines = &mut projection.lines;

    for message in messages {
        if !view_mode.includes_message(message) {
            continue;
        }

        if message.kind == ConversationMessageKind::Tool {
            let line_index = lines.len();
            let digest = tool_message_digest(message.item_id.as_deref(), &message.text);
            if tool_message_is_expandable(&message.text) {
                projection
                    .card_rows
                    .push(ConversationTranscriptCardRow { line_index, digest });
            }
            lines.extend(format_tool_card_lines(
                message,
                view_mode,
                expand_state,
                width,
            ));
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
        let mut markdown_code_fence = None;
        for text_line in message.text.lines() {
            if let Some(line) = format_markdown_body_line(text_line, &mut markdown_code_fence) {
                lines.push(line);
            }
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

        // Separators participate in the same wrapped-row calculation used by the viewport.
        lines.push(Line::from(""));
    }
}

fn append_empty_transcript_message(
    lines: &mut Vec<Line<'static>>,
    source_message_count: usize,
    view_mode: ConversationViewMode,
) {
    if lines.is_empty() {
        let empty_message = if source_message_count == 0 {
            "No messages in this thread yet.".to_string()
        } else {
            format!("No messages visible in {} view.", view_mode.label())
        };
        lines.push(Line::from(empty_message));
    }
}

fn format_tool_card_lines(
    message: &ConversationMessage,
    view_mode: ConversationViewMode,
    expand_state: Option<&ProgressiveActivityExpandState>,
    width: u16,
) -> Vec<Line<'static>> {
    let expandable = tool_message_is_expandable(&message.text);
    let digest = tool_message_digest(message.item_id.as_deref(), &message.text);
    // Detail view expands multi-line tool cards by default; Medium keeps them collapsed
    // unless the operator toggled the card. Single-line tools stay header-only.
    let expanded = expandable
        && (expand_state.is_some_and(|state| state.is_tool_expanded(digest))
            || matches!(view_mode, ConversationViewMode::Detail));

    let indicator = if !expandable {
        AkraTheme::non_expandable_indicator()
    } else if expanded {
        AkraTheme::expanded_indicator()
    } else {
        AkraTheme::collapsed_indicator()
    };
    let title = tool_message_title(&message.text);
    let label = message
        .display_label
        .as_deref()
        .filter(|label| !label.trim().is_empty())
        .unwrap_or("tool");
    let fact = if matches!(label, "read" | "list" | "search" | "explore") {
        String::new()
    } else {
        tool_message_fact(&message.text)
    };

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

    if expanded && label == "patch" && tool_message_has_unified_diff(&message.text) {
        let detail = message.text.lines().skip(1).collect::<Vec<_>>().join("\n");
        let preview = build_inline_diff_preview(&detail, width, INLINE_DIFF_PREVIEW_ROWS);
        lines.extend(preview.lines);
        if preview.has_more {
            lines.push(Line::styled(
                "  ... more diff detail in :activity",
                AkraTheme::muted(),
            ));
        }
    } else if expanded {
        let mut markdown_code_fence = None;
        let mut body_lines = Vec::new();
        for text_line in message.text.lines().skip(1) {
            let Some(mut body) = format_markdown_body_line(text_line, &mut markdown_code_fence)
            else {
                continue;
            };
            // Dim every non-indent span so multi-span markdown tool bodies stay consistent.
            for span in body.spans.iter_mut().skip(1) {
                span.style = span.style.patch(AkraTheme::tool_card_body());
            }
            body_lines.push(body);
        }
        let has_more = body_lines.len() > INLINE_TOOL_DETAIL_ROWS;
        lines.extend(body_lines.into_iter().take(INLINE_TOOL_DETAIL_ROWS));
        if has_more {
            lines.push(Line::styled(
                "  ... more tool detail in :activity",
                AkraTheme::muted(),
            ));
        }
    }

    lines
}

fn tool_message_has_unified_diff(text: &str) -> bool {
    text.lines().skip(1).any(|line| {
        line.starts_with("diff --git ") || line.starts_with("@@ -") || line.starts_with("--- ")
    })
}

// Normalize tabs before ratatui width/layout calculations so transcript alignment is terminal-independent.
fn expand_tui_tabs(text: &str) -> String {
    text.replace('\t', "    ")
}

#[derive(Clone, Copy)]
struct MarkdownCodeFence {
    delimiter: char,
    minimum_len: usize,
    indent: usize,
}

fn format_markdown_body_line(
    text: &str,
    markdown_code_fence: &mut Option<MarkdownCodeFence>,
) -> Option<Line<'static>> {
    let expanded = expand_tui_tabs(text);
    let mut spans = vec![Span::raw("  ")];
    if let Some(open_fence) = *markdown_code_fence {
        if is_markdown_code_fence_close(&expanded, open_fence) {
            *markdown_code_fence = None;
            return None;
        }
        let removable_indent = expanded
            .as_bytes()
            .iter()
            .take_while(|byte| **byte == b' ')
            .count()
            .min(open_fence.indent);
        spans.push(Span::styled(
            expanded[removable_indent..].to_string(),
            AkraTheme::markdown_code_block(),
        ));
        return Some(Line::from(spans));
    }
    if let Some(open_fence) = markdown_code_fence_open(&expanded) {
        *markdown_code_fence = Some(open_fence);
        return None;
    }
    let block = markdown_block_line(&expanded);
    spans.extend(block.prefix_spans);
    spans.extend(format_markdown_inline_spans(block.body, block.body_style));
    Some(Line::from(spans))
}

fn markdown_code_fence_open(text: &str) -> Option<MarkdownCodeFence> {
    let (indent, candidate) = markdown_fence_candidate(text)?;
    let delimiter = candidate.chars().next()?;
    if !matches!(delimiter, '`' | '~') {
        return None;
    }
    let minimum_len = candidate.chars().take_while(|ch| *ch == delimiter).count();
    if minimum_len < 3 {
        return None;
    }
    let info = &candidate[minimum_len..];
    if delimiter == '`' && info.contains('`') {
        return None;
    }
    Some(MarkdownCodeFence {
        delimiter,
        minimum_len,
        indent,
    })
}

fn is_markdown_code_fence_close(text: &str, open_fence: MarkdownCodeFence) -> bool {
    let Some((_, candidate)) = markdown_fence_candidate(text) else {
        return false;
    };
    let delimiter_len = candidate
        .chars()
        .take_while(|ch| *ch == open_fence.delimiter)
        .count();
    delimiter_len >= open_fence.minimum_len
        && candidate[delimiter_len..]
            .chars()
            .all(|character| character == ' ')
}

fn markdown_fence_candidate(text: &str) -> Option<(usize, &str)> {
    let indent = text
        .as_bytes()
        .iter()
        .take_while(|byte| **byte == b' ')
        .count();
    (indent <= 3).then(|| (indent, &text[indent..]))
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
    fn structured_read_card_is_quiet_when_collapsed_and_exact_when_expanded() {
        let messages = vec![
            ConversationMessage::new(
                ConversationMessageKind::Tool,
                "Read src/lib.rs\n1. Read src/lib.rs\n   path: C:/dev/akra/src/lib.rs",
                None,
                Some("read-1".to_string()),
            )
            .with_display_label("read"),
        ];

        let medium =
            format_conversation_lines_for_view(&messages, ConversationViewMode::Medium, false);
        let medium_text = medium.iter().map(line_text).collect::<Vec<_>>().join("\n");
        assert!(
            medium_text.contains("read      Read src/lib.rs"),
            "{medium_text}"
        );
        assert!(!medium_text.contains("path:"), "{medium_text}");
        assert!(!medium_text.contains("3 lines"), "{medium_text}");

        let detail =
            format_conversation_lines_for_view(&messages, ConversationViewMode::Detail, false);
        let detail_text = detail.iter().map(line_text).collect::<Vec<_>>().join("\n");
        assert!(detail_text.contains("1. Read src/lib.rs"), "{detail_text}");
        assert!(
            detail_text.contains("path: C:/dev/akra/src/lib.rs"),
            "{detail_text}"
        );
        assert_eq!(detail_text.matches("Read src/lib.rs").count(), 2);
    }

    #[test]
    fn repeated_identical_reads_expand_only_the_selected_item() {
        let text = "Read src/lib.rs\n1. Read src/lib.rs\n   path: C:/dev/akra/src/lib.rs";
        let messages = vec![
            ConversationMessage::new(
                ConversationMessageKind::Tool,
                text,
                None,
                Some("read-1".to_string()),
            )
            .with_display_label("read"),
            ConversationMessage::new(
                ConversationMessageKind::Tool,
                text,
                None,
                Some("read-2".to_string()),
            )
            .with_display_label("read"),
        ];
        let mut expand_state = ProgressiveActivityExpandState::default();
        expand_state.expand_tool(tool_message_digest(Some("read-1"), text));

        let rendered = format_fullscreen_conversation_lines_with_expand_at_width(
            &messages,
            ConversationViewMode::Medium,
            false,
            Some(&expand_state),
            DEFAULT_TRANSCRIPT_WIDTH,
        )
        .iter()
        .map(line_text)
        .collect::<Vec<_>>()
        .join("\n");

        assert_eq!(rendered.matches("path: C:/dev/akra/src/lib.rs").count(), 1);
    }

    #[test]
    fn expanded_patch_card_reuses_numbered_semantic_diff_bands() {
        let messages = vec![
            ConversationMessage::new(
                ConversationMessageKind::Tool,
                concat!(
                    "file change: update src/lib.rs\n",
                    "[update] src/lib.rs\n",
                    "--- a/src/lib.rs\n",
                    "+++ b/src/lib.rs\n",
                    "@@ -7,2 +7,2 @@\n",
                    "-old_value\n",
                    "+new_value\n"
                ),
                None,
                Some("patch-1".to_string()),
            )
            .with_display_label("patch"),
        ];

        let detail =
            format_conversation_lines_for_view(&messages, ConversationViewMode::Detail, false);
        let rendered = detail.iter().map(line_text).collect::<Vec<_>>().join("\n");
        assert!(rendered.contains("Edited src/lib.rs"), "{rendered}");
        assert!(rendered.contains("7 -old_value"), "{rendered}");
        assert!(rendered.contains("7 +new_value"), "{rendered}");

        let deletion = detail
            .iter()
            .find(|line| line_text(line).contains("old_value"))
            .expect("deletion row");
        let addition = detail
            .iter()
            .find(|line| line_text(line).contains("new_value"))
            .expect("addition row");
        assert!(
            deletion
                .spans
                .iter()
                .any(|span| span.style == AkraTheme::diff_deletion())
        );
        assert!(
            addition
                .spans
                .iter()
                .any(|span| span.style == AkraTheme::diff_addition())
        );
    }

    #[test]
    fn expanded_tool_card_bounds_inline_detail_and_points_to_activity() {
        let detail = (1..=10)
            .map(|line| format!("detail line {line}"))
            .collect::<Vec<_>>()
            .join("\n");
        let messages = vec![
            ConversationMessage::new(
                ConversationMessageKind::Tool,
                format!("Read ten records\n{detail}"),
                None,
                Some("read-many".to_string()),
            )
            .with_display_label("read"),
        ];

        let rendered =
            format_conversation_lines_for_view(&messages, ConversationViewMode::Detail, false)
                .iter()
                .map(line_text)
                .collect::<Vec<_>>()
                .join("\n");

        assert!(rendered.contains("detail line 8"), "{rendered}");
        assert!(!rendered.contains("detail line 9"), "{rendered}");
        assert!(
            rendered.contains("more tool detail in :activity"),
            "{rendered}"
        );
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
    fn transcript_body_hides_code_fences_and_preserves_code_style() {
        let messages = vec![ConversationMessage::new(
            ConversationMessageKind::Agent,
            "```rust\nlet literal = \"**keep markers**\";\n```",
            Some("final_answer".to_string()),
            Some("agent-1".to_string()),
        )];

        let lines =
            format_conversation_lines_for_view(&messages, ConversationViewMode::Medium, false);

        assert_eq!(
            line_text(&lines[1]),
            "  let literal = \"**keep markers**\";"
        );
        assert_eq!(lines[1].spans[1].style, AkraTheme::markdown_code_block());
        assert_eq!(line_text(&lines[2]), "");
        assert!(
            lines
                .iter()
                .all(|line| !line_text(line).contains("```rust"))
        );
    }

    #[test]
    fn transcript_body_closes_only_matching_code_fences() {
        let messages = vec![ConversationMessage::new(
            ConversationMessageKind::Agent,
            "````rust\none\n```\n~~~\ntwo\n`````\nafter",
            Some("final_answer".to_string()),
            Some("agent-1".to_string()),
        )];

        let lines =
            format_conversation_lines_for_view(&messages, ConversationViewMode::Medium, false);

        assert_eq!(line_text(&lines[1]), "  one");
        assert_eq!(line_text(&lines[2]), "  ```");
        assert_eq!(line_text(&lines[3]), "  ~~~");
        assert_eq!(line_text(&lines[4]), "  two");
        for line in &lines[1..=4] {
            assert_eq!(line.spans[1].style, AkraTheme::markdown_code_block());
        }
        assert_eq!(line_text(&lines[5]), "  after");
        assert_eq!(lines[5].spans[1].style, Style::default());
    }

    #[test]
    fn transcript_body_renders_unclosed_streaming_code_fence() {
        let messages = vec![ConversationMessage::new(
            ConversationMessageKind::Agent,
            "~~~rust\nfn streaming() {}",
            Some("agent_message".to_string()),
            Some("agent-1".to_string()),
        )];

        let lines =
            format_conversation_lines_for_view(&messages, ConversationViewMode::Medium, false);

        assert_eq!(line_text(&lines[1]), "  fn streaming() {}");
        assert_eq!(lines[1].spans[1].style, AkraTheme::markdown_code_block());
        assert_eq!(line_text(&lines[2]), "");
    }

    #[test]
    fn transcript_body_applies_commonmark_fence_indentation() {
        let messages = vec![ConversationMessage::new(
            ConversationMessageKind::Agent,
            "  ```rust\n  fn main() {}\n  ```",
            Some("final_answer".to_string()),
            Some("agent-1".to_string()),
        )];

        let lines =
            format_conversation_lines_for_view(&messages, ConversationViewMode::Medium, false);

        assert_eq!(line_text(&lines[1]), "  fn main() {}");
        assert!(markdown_code_fence_open("    ```rust").is_none());
        assert!(!is_markdown_code_fence_close(
            "    ```",
            MarkdownCodeFence {
                delimiter: '`',
                minimum_len: 3,
                indent: 0,
            }
        ));
    }

    fn line_text(line: &Line<'_>) -> String {
        line.spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect::<String>()
    }
}
