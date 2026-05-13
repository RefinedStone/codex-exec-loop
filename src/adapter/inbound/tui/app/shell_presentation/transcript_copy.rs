use crate::adapter::inbound::tui::conversation_text::conversation_message_label;

use super::{
    AkraTheme, ConversationMessage, ConversationMessageKind, ConversationViewMode, Line,
    MAX_CONVERSATION_HISTORY_LINES, Span, Style,
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
        for text_line in message.text.lines() {
            lines.push(Line::from(format!("  {}", expand_tui_tabs(text_line))));
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

// Only speaker labels are styled; body text remains plain so content color does not fight syntax/log output.
fn label_style(kind: ConversationMessageKind) -> Style {
    match kind {
        ConversationMessageKind::User => AkraTheme::shortcut(),
        ConversationMessageKind::Agent => AkraTheme::brand(),
        ConversationMessageKind::Tool => AkraTheme::tool(),
        ConversationMessageKind::Status => AkraTheme::muted(),
    }
}
