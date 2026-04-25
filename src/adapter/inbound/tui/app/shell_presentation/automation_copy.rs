use super::{AkraTheme, ConversationState, Line, NativeTuiApp, OverlayListView};

pub(in super::super) fn build_automation_key_lines(app: &NativeTuiApp) -> Vec<Line<'static>> {
    if app.is_max_auto_turns_editing() {
        return vec![
            AkraTheme::key_line("Type the new max-turn value directly. Backspace deletes."),
            AkraTheme::key_line("Enter: save max turns    Esc/Ctrl+C: cancel edit"),
            AkraTheme::key_line("Use a whole number greater than 0, or type infinite."),
        ];
    }

    if app.is_stop_keyword_editing() {
        return vec![
            AkraTheme::key_line("Type the new stop keyword directly. Backspace deletes."),
            AkraTheme::key_line("Enter: save stop keyword    Esc/Ctrl+C: cancel edit"),
            AkraTheme::key_line("Use letters, numbers, or underscores only."),
        ];
    }

    vec![
        AkraTheme::key_line("PageUp/PageDown or Ctrl+u/Ctrl+d: scroll preview"),
        AkraTheme::key_line(
            "Ctrl+a: automation on/off    Ctrl+l: edit max turns    Ctrl+g: edit stop keyword",
        ),
        AkraTheme::key_line(
            "Ctrl+k: stop rule on/off    Ctrl+n: no-file stop    Ctrl+b: planner detail",
        ),
        AkraTheme::key_line("Enter/Esc/Ctrl+C: close"),
    ]
}

pub(in super::super) fn build_automation_list_view(app: &NativeTuiApp) -> OverlayListView {
    match &app.conversation_state {
        ConversationState::Loading => OverlayListView {
            message_lines: Some(vec![Line::from("conversation is still loading")]),
            items: Vec::new(),
            selected_index: None,
        },
        ConversationState::Failed(message) => OverlayListView {
            message_lines: Some(vec![Line::from(message.clone())]),
            items: Vec::new(),
            selected_index: None,
        },
        ConversationState::Ready(_) => OverlayListView {
            message_lines: Some(vec![
                Line::from("automation follows the planning queue only"),
                Line::from("no legacy automation catalogs or workspace prompt files are used"),
            ]),
            items: Vec::new(),
            selected_index: None,
        },
    }
}
