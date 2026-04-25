use crate::adapter::inbound::tui::conversation_text::conversation_message_label;

use super::{
    AkraTheme, ConversationMessage, ConversationMessageKind, Line, MAX_CONVERSATION_HISTORY_LINES,
    Span, Style,
};

pub(in super::super) fn format_conversation_lines(
    messages: &[ConversationMessage],
) -> Vec<Line<'static>> {
    format_conversation_lines_with_debug(messages, false)
}

pub(in super::super) fn format_conversation_lines_with_debug(
    messages: &[ConversationMessage],
    show_debug_details: bool,
) -> Vec<Line<'static>> {
    let mut lines = Vec::new();

    for message in messages {
        let label = conversation_message_label(message);
        lines.push(Line::from(Span::styled(
            format!("{label}:"),
            label_style(message.kind),
        )));
        for text_line in message.text.lines() {
            lines.push(Line::from(format!("  {}", expand_tui_tabs(text_line))));
        }
        if show_debug_details && let Some(debug_detail) = message.debug_detail.as_deref() {
            for detail_line in debug_detail.lines() {
                lines.push(Line::from(Span::styled(
                    format!("  {}", expand_tui_tabs(detail_line)),
                    AkraTheme::muted(),
                )));
            }
        }
        lines.push(Line::from(""));
    }

    if lines.is_empty() {
        lines.push(Line::from("No messages in this thread yet."));
    }

    if lines.len() > MAX_CONVERSATION_HISTORY_LINES {
        lines.drain(0..lines.len() - MAX_CONVERSATION_HISTORY_LINES);
    }

    lines
}

fn expand_tui_tabs(text: &str) -> String {
    text.replace('\t', "    ")
}

fn label_style(kind: ConversationMessageKind) -> Style {
    match kind {
        ConversationMessageKind::User => AkraTheme::shortcut(),
        ConversationMessageKind::Agent => AkraTheme::brand(),
        ConversationMessageKind::Tool => AkraTheme::tool(),
        ConversationMessageKind::Status => AkraTheme::muted(),
    }
}
