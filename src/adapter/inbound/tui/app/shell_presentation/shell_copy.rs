use super::*;

pub(super) fn build_shell_header_lines_with_context(
    context: &ShellCorePresentationContext<'_>,
) -> Vec<Line<'static>> {
    match context.conversation_state {
        ShellConversationState::Loading => vec![
            Line::from(vec![
                Span::styled("Conversation Shell", Style::default().fg(Color::Cyan)),
                Span::raw(" / loading thread"),
            ]),
            Line::from("Reading thread history from codex app-server."),
        ],
        ShellConversationState::Ready(conversation) => vec![
            Line::from(vec![
                Span::styled("Conversation Shell", Style::default().fg(Color::Cyan)),
                Span::raw(" / "),
                Span::raw(conversation.title.clone()),
            ]),
            Line::from(vec![
                Span::raw(format!(
                    "thread: {}  |  input: ",
                    if conversation.has_active_thread() {
                        conversation.thread_id.as_str()
                    } else {
                        "not started yet"
                    }
                )),
                Span::styled(
                    conversation.input_state.label(),
                    input_state_style(conversation.input_state),
                ),
                Span::raw("  |  startup: "),
                Span::styled(
                    context.shell_action_availability.status_text(),
                    startup_state_style_for_availability(context.shell_action_availability),
                ),
            ]),
        ],
        ShellConversationState::Failed(message) => vec![
            Line::from(vec![
                Span::styled("Conversation Shell", Style::default().fg(Color::Red)),
                Span::raw(" / failed"),
            ]),
            Line::from(message.to_string()),
        ],
    }
}

pub(super) fn build_shell_title() -> Line<'static> {
    Line::from("Shell / Ctrl+t new draft / Ctrl+C back / Ctrl+q quit")
}

pub(super) fn build_transcript_title_with_context(
    _context: &ShellCorePresentationContext<'_>,
) -> Line<'static> {
    Line::from("Transcript / live scrollback")
}

pub(in super::super) fn build_status_title() -> Line<'static> {
    Line::from("Controls / shell shortcuts and live status")
}

pub(super) fn build_input_title_with_context(
    context: &ShellCorePresentationContext<'_>,
) -> Line<'static> {
    match context.conversation_state {
        ShellConversationState::Loading => {
            Line::from(vec![Span::raw("Prompt"), Span::raw(" / loading")])
        }
        ShellConversationState::Failed(_) => {
            Line::from(vec![Span::raw("Prompt"), Span::raw(" / unavailable")])
        }
        ShellConversationState::Ready(conversation) => {
            let submit_hint = build_primary_submit_hint_with_context(context);
            Line::from(vec![
                Span::raw("Prompt"),
                Span::raw(" / "),
                Span::styled(
                    conversation.input_state.label(),
                    input_state_style(conversation.input_state),
                ),
                Span::raw(" / "),
                Span::raw(submit_hint),
                Span::raw(" / Ctrl+j newline"),
            ])
        }
    }
}

pub(super) fn build_frontend_summary_line() -> Line<'static> {
    Line::from(
        "frontend: inline main buffer  |  history: host terminal scrollback  |  tail: prompt anchored",
    )
}

fn build_primary_submit_hint_with_context(
    context: &ShellCorePresentationContext<'_>,
) -> &'static str {
    match context.conversation_state {
        ShellConversationState::Ready(conversation) if conversation.startup_submit_armed => {
            "queued until ready"
        }
        ShellConversationState::Ready(conversation) if conversation.has_running_turn() => {
            "Enter send when idle"
        }
        ShellConversationState::Ready(_) if !context.shell_action_availability.allows_actions() => {
            "Enter send when ready"
        }
        ShellConversationState::Ready(_) => "Enter send",
        _ => "",
    }
}

fn startup_state_style_for_availability(
    shell_action_availability: ShellActionAvailability,
) -> Style {
    match shell_action_availability {
        ShellActionAvailability::Ready => Style::default().fg(Color::Green),
        ShellActionAvailability::Pending => Style::default().fg(Color::Yellow),
        ShellActionAvailability::Blocked => Style::default().fg(Color::Red),
    }
}
