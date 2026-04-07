use super::*;

pub(super) fn build_conversation_lines(app: &NativeTuiApp) -> Vec<Line<'static>> {
    match &app.conversation_state {
        ConversationState::Loading => vec![Line::from("Loading thread history...")],
        ConversationState::Failed(message) => vec![Line::from(message.clone())],
        ConversationState::Ready(conversation) => conversation.cached_conversation_lines.clone(),
    }
}

pub(super) fn format_conversation_lines(messages: &[ConversationMessage]) -> Vec<Line<'static>> {
    let mut lines = Vec::new();

    for message in messages {
        let label = message.kind.label(message.phase.as_deref());
        lines.push(Line::from(Span::styled(
            format!("{label}:"),
            label_style(message.kind),
        )));
        for text_line in message.text.lines() {
            lines.push(Line::from(format!("  {text_line}")));
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

pub(super) fn build_shell_footer_lines(app: &NativeTuiApp) -> Vec<Line<'static>> {
    match &app.conversation_state {
        ConversationState::Loading => vec![
            Line::from(format!(
                "startup: {}  |  sessions: {}",
                shell_action_availability_label(app),
                recent_session_status_label(app)
            )),
            Line::from("conversation state: loading thread metadata"),
            Line::from("status: waiting for thread history from codex app-server"),
        ],
        ConversationState::Failed(message) => vec![
            Line::from(format!(
                "startup: {}  |  sessions: {}",
                shell_action_availability_label(app),
                recent_session_status_label(app)
            )),
            Line::from("conversation state: failed"),
            Line::from(format!("status: {message}")),
        ],
        ConversationState::Ready(conversation) => {
            let skip_summary = conversation
                .last_auto_followup_skip
                .as_ref()
                .map(|skip| skip.reason.label())
                .unwrap_or("none");
            let skip_detail = conversation
                .last_auto_followup_skip
                .as_ref()
                .map(|skip| skip.detail.as_str())
                .unwrap_or("none");
            let warning_summary = if conversation.warnings.is_empty() {
                "warnings: none".to_string()
            } else {
                format!("warnings: {}", conversation.warnings.len())
            };

            vec![
                Line::from(format!(
                    "startup: {}  |  sessions: {}  |  turn: {}  |  input: {}",
                    shell_action_availability_label(app),
                    recent_session_status_label(app),
                    turn_status_label(conversation),
                    conversation.input_state.label(),
                )),
                Line::from(format!(
                    "thread: {}  |  auto: {} ({})  |  template: {}",
                    if conversation.has_active_thread() {
                        conversation.thread_id.as_str()
                    } else {
                        "new draft"
                    },
                    conversation.auto_follow_state.status_label(),
                    conversation.auto_follow_state.progress_label(),
                    conversation.auto_follow_state.template_label()
                )),
                Line::from(format!(
                    "status: {}  |  file changes: {}  |  {}",
                    conversation.status_text,
                    conversation
                        .turn_activity
                        .last_completed_file_change_count(),
                    warning_summary,
                )),
                Line::from(format!(
                    "input detail: {}  |  template slot: {}/{}",
                    conversation.input_state.detail(),
                    conversation.auto_follow_state.selected_template_index() + 1,
                    conversation.auto_follow_state.template_count(),
                )),
                Line::from(format!(
                    "template source: {}  |  last skip: {}  |  detail: {}",
                    conversation.auto_follow_state.template_source_label(),
                    skip_summary,
                    skip_detail,
                )),
            ]
        }
    }
}

pub(super) fn build_input_lines(app: &NativeTuiApp) -> Vec<Line<'static>> {
    match &app.conversation_state {
        ConversationState::Loading => vec![
            Line::from("Thread is still loading."),
            Line::from("Input becomes available when the shell reaches ready state."),
        ],
        ConversationState::Failed(message) => vec![Line::from(message.clone())],
        ConversationState::Ready(conversation) => {
            build_ready_input_lines(conversation, app.shell_action_availability())
        }
    }
}

pub(super) fn build_ready_input_lines(
    conversation: &ConversationViewModel,
    shell_action_availability: ShellActionAvailability,
) -> Vec<Line<'static>> {
    let mut lines = Vec::new();

    if conversation.input_buffer.is_empty() {
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

    lines.extend(
        conversation
            .input_buffer
            .lines()
            .map(|line| Line::from(line.to_string())),
    );

    if let Some(command) = InlineShellCommand::parse(&conversation.input_buffer) {
        lines.push(Line::from(command.buffered_hint()));
        return lines;
    }

    match (conversation.input_state, shell_action_availability) {
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

pub(super) fn build_shell_title() -> Line<'static> {
    Line::from("Shell / Ctrl+t new draft / Ctrl+C back / Ctrl+q quit")
}

pub(super) fn build_transcript_title(app: &NativeTuiApp) -> Line<'static> {
    Line::from(vec![
        Span::raw("Transcript / "),
        Span::raw(app.transcript_viewport_status_label()),
        Span::raw(" / PageUp PageDown / Home End"),
    ])
}

pub(super) fn build_status_title() -> Line<'static> {
    Line::from(
        "Status / Ctrl+o sessions / Ctrl+d diag / Ctrl+p templ / Ctrl+a auto / Ctrl+k stop / Ctrl+n no-files / Ctrl+g edit",
    )
}

pub(super) fn build_input_title(app: &NativeTuiApp) -> Line<'static> {
    let submit_hint = build_primary_submit_hint(app);

    match &app.conversation_state {
        ConversationState::Loading => Line::from("Composer / loading"),
        ConversationState::Failed(_) => Line::from("Composer / unavailable"),
        ConversationState::Ready(conversation) => Line::from(vec![
            Span::raw("Composer / "),
            Span::styled(
                conversation.input_state.label().to_string(),
                input_state_style(conversation.input_state),
            ),
            Span::raw(" / startup "),
            Span::styled(
                shell_action_availability_label(app).to_string(),
                startup_state_style(app),
            ),
            Span::raw(format!(" / {submit_hint} / Ctrl+j newline")),
        ]),
    }
}

fn build_primary_submit_hint(app: &NativeTuiApp) -> &'static str {
    match &app.conversation_state {
        ConversationState::Ready(conversation) if conversation.has_running_turn() => {
            "Enter send when idle"
        }
        ConversationState::Ready(_) if !app.shell_action_availability().allows_actions() => {
            "Enter send when ready"
        }
        ConversationState::Ready(_) => "Enter send",
        _ => "",
    }
}

pub(super) fn shell_action_availability_label(app: &NativeTuiApp) -> &'static str {
    app.shell_action_availability().status_text()
}

pub(super) fn startup_state_style(app: &NativeTuiApp) -> Style {
    match app.shell_action_availability() {
        ShellActionAvailability::Ready => Style::default().fg(Color::Green),
        ShellActionAvailability::Pending => Style::default().fg(Color::Yellow),
        ShellActionAvailability::Blocked => Style::default().fg(Color::Red),
    }
}

fn recent_session_status_label(app: &NativeTuiApp) -> String {
    if !app.can_open_session_list() {
        return match &app.startup_state {
            StartupState::Loading => "waiting for startup checks".to_string(),
            StartupState::Ready(_) | StartupState::Failed(_) => {
                "blocked by startup diagnostics".to_string()
            }
            StartupState::Idle => "not requested yet".to_string(),
        };
    }

    match &app.session_state {
        SessionState::Idle => "ready to load".to_string(),
        SessionState::Loading => "loading from codex app-server".to_string(),
        SessionState::Failed(_) => "load failed".to_string(),
        SessionState::Ready(recent_sessions) => format!("{} loaded", recent_sessions.items.len()),
    }
}

fn turn_status_label(conversation: &ConversationViewModel) -> &'static str {
    if conversation.has_running_turn() {
        "running"
    } else {
        "idle"
    }
}

pub(super) fn input_state_style(input_state: ConversationInputState) -> Style {
    match input_state {
        ConversationInputState::DraftReady | ConversationInputState::ReadyToContinue => {
            Style::default().fg(Color::Green)
        }
        ConversationInputState::SubmittingTurn => Style::default().fg(Color::Yellow),
        ConversationInputState::StreamingTurn => Style::default().fg(Color::Cyan),
    }
}

fn label_style(kind: ConversationMessageKind) -> Style {
    match kind {
        ConversationMessageKind::User => Style::default().fg(Color::Yellow),
        ConversationMessageKind::Agent => Style::default().fg(Color::Cyan),
        ConversationMessageKind::Tool => Style::default().fg(Color::Magenta),
        ConversationMessageKind::Status => Style::default().fg(Color::Red),
    }
}
