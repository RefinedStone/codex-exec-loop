use super::session_browser::build_session_browser_view;
use super::*;
use crate::domain::followup_template::FollowupTemplateDefinition;

pub(super) struct ConversationShellView {
    pub(super) shell_title: Line<'static>,
    pub(super) header_lines: Vec<Line<'static>>,
    pub(super) conversation_lines: Vec<Line<'static>>,
    pub(super) status_title: Line<'static>,
    pub(super) footer_lines: Vec<Line<'static>>,
    pub(super) input_title: Line<'static>,
    pub(super) input_lines: Vec<Line<'static>>,
}

pub(super) struct ConversationShellFrameView {
    pub(super) shell_title: Line<'static>,
    pub(super) header_lines: Vec<Line<'static>>,
    pub(super) header_area: Rect,
    pub(super) transcript_view: TranscriptPanelView,
    pub(super) transcript_area: Rect,
    pub(super) status_title: Line<'static>,
    pub(super) footer_lines: Vec<Line<'static>>,
    pub(super) footer_area: Rect,
    pub(super) input_title: Line<'static>,
    pub(super) input_lines: Vec<Line<'static>>,
    pub(super) input_area: Rect,
}

pub(super) struct TranscriptPanelView {
    pub(super) title: Line<'static>,
    pub(super) lines: Vec<Line<'static>>,
    pub(super) scroll_offset: u16,
}

pub(super) struct StartupOverlayView {
    pub(super) header_lines: Vec<Line<'static>>,
    pub(super) summary_lines: Vec<Line<'static>>,
    pub(super) check_items: Vec<ListItem<'static>>,
    pub(super) warning_lines: Vec<Line<'static>>,
    pub(super) key_lines: Vec<Line<'static>>,
}

pub(super) struct OverlayListEntryView {
    pub(super) lines: Vec<Line<'static>>,
}

pub(super) struct OverlayListView {
    pub(super) message_lines: Option<Vec<Line<'static>>>,
    pub(super) items: Vec<OverlayListEntryView>,
    pub(super) selected_index: Option<usize>,
}

pub(super) struct SessionOverlayView {
    pub(super) header_lines: Vec<Line<'static>>,
    pub(super) list_view: OverlayListView,
    pub(super) detail_lines: Vec<Line<'static>>,
    pub(super) warning_lines: Vec<Line<'static>>,
    pub(super) key_lines: Vec<Line<'static>>,
}

pub(super) struct FollowupTemplateOverlayView {
    pub(super) header_lines: Vec<Line<'static>>,
    pub(super) list_view: OverlayListView,
    pub(super) preview_lines: Vec<Line<'static>>,
    pub(super) status_lines: Vec<Line<'static>>,
    pub(super) key_lines: Vec<Line<'static>>,
}

pub(super) fn build_conversation_shell_view(
    app: &NativeTuiApp,
    mode: ShellFrontendMode,
) -> ConversationShellView {
    ConversationShellView {
        shell_title: build_shell_title(mode),
        header_lines: build_shell_header_lines(app),
        conversation_lines: build_conversation_lines(app),
        status_title: build_status_title(mode),
        footer_lines: build_shell_footer_lines(app),
        input_title: build_input_title(app, mode),
        input_lines: build_input_lines(app),
    }
}

pub(super) fn build_startup_overlay_view(app: &NativeTuiApp) -> StartupOverlayView {
    StartupOverlayView {
        header_lines: vec![
            Line::from(vec![
                Span::styled(
                    "Startup Diagnostics",
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw(" / shell overlay"),
            ]),
            Line::from("Inspect readiness without leaving the live shell."),
        ],
        summary_lines: match &app.startup_state {
            StartupState::Idle => vec![
                Line::from("status: idle"),
                Line::from("startup checks have not started yet"),
            ],
            StartupState::Loading => vec![
                Line::from(vec![
                    Span::styled("status: ", Style::default().fg(Color::Gray)),
                    Span::styled("running checks", Style::default().fg(Color::Yellow)),
                ]),
                Line::from("probing codex binary, app-server handshake, account state, and cwd"),
            ],
            StartupState::Ready(diagnostics) => vec![
                Line::from(vec![
                    Span::styled("status: ", Style::default().fg(Color::Gray)),
                    Span::styled(
                        if diagnostics.can_continue() {
                            "ready"
                        } else {
                            "needs attention"
                        },
                        Style::default().fg(if diagnostics.can_continue() {
                            Color::Green
                        } else {
                            Color::Yellow
                        }),
                    ),
                ]),
                Line::from(format!("cwd: {}", diagnostics.cwd)),
            ],
            StartupState::Failed(message) => vec![
                Line::from(vec![
                    Span::styled("status: ", Style::default().fg(Color::Gray)),
                    Span::styled("failed", Style::default().fg(Color::Red)),
                ]),
                Line::from(message.clone()),
            ],
        },
        check_items: build_startup_check_items(app),
        warning_lines: build_startup_warning_lines(app),
        key_lines: vec![
            Line::from("Esc/Ctrl+C: close    r: rerun checks"),
            Line::from("Ctrl+o: recent sessions"),
        ],
    }
}

pub(super) fn build_conversation_shell_frame_view(
    app: &mut NativeTuiApp,
    mode: ShellFrontendMode,
    area: Rect,
) -> ConversationShellFrameView {
    let shell_view = build_conversation_shell_view(app, mode);
    let ConversationShellView {
        shell_title,
        header_lines,
        conversation_lines,
        status_title,
        footer_lines,
        input_title,
        input_lines,
    } = shell_view;
    let header_height = block_height_for_lines(
        &header_lines,
        MIN_SHELL_HEADER_HEIGHT,
        MAX_SHELL_HEADER_HEIGHT,
    );
    let footer_height = build_shell_footer_height(&footer_lines);
    let input_height = build_input_block_height(&input_lines);

    let layout = Layout::default()
        .direction(Direction::Vertical)
        .margin(SHELL_FRAME_MARGIN)
        .constraints([
            Constraint::Length(header_height),
            Constraint::Min(MIN_TRANSCRIPT_PANEL_HEIGHT),
            Constraint::Length(footer_height),
            Constraint::Length(input_height),
        ])
        .split(area);
    let transcript_inner = Block::default().borders(Borders::ALL).inner(layout[1]);

    let transcript_view = build_transcript_panel_view(
        app,
        mode,
        conversation_lines,
        transcript_inner.width,
        transcript_inner.height,
    );

    ConversationShellFrameView {
        shell_title,
        header_lines,
        header_area: layout[0],
        transcript_view,
        transcript_area: layout[1],
        status_title,
        footer_lines,
        footer_area: layout[2],
        input_title,
        input_lines,
        input_area: layout[3],
    }
}

pub(super) fn build_transcript_panel_view(
    app: &mut NativeTuiApp,
    mode: ShellFrontendMode,
    lines: Vec<Line<'static>>,
    content_width: u16,
    visible_height: u16,
) -> TranscriptPanelView {
    let max_scroll_offset = build_conversation_scroll_offset(&lines, content_width, visible_height);
    let scroll_offset = app.sync_transcript_viewport_metrics(max_scroll_offset, visible_height);

    TranscriptPanelView {
        title: build_transcript_title(app, mode),
        lines,
        scroll_offset,
    }
}

pub(super) fn build_session_overlay_view(app: &NativeTuiApp) -> SessionOverlayView {
    let (list_view, detail_lines) = build_session_overlay_content(app);

    SessionOverlayView {
        header_lines: vec![
            Line::from(vec![
                Span::styled(
                    "Recent Sessions",
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw(" / shell overlay"),
            ]),
            Line::from("Resume a thread without leaving the shell view."),
        ],
        list_view,
        detail_lines,
        warning_lines: build_session_warning_lines(app),
        key_lines: vec![
            Line::from("Up/Down or j/k: move    Enter: open thread"),
            Line::from("n: new draft    r: reload    Esc/Ctrl+C: close    Ctrl+d: diagnostics"),
        ],
    }
}

pub(super) fn build_followup_template_overlay_view(
    app: &NativeTuiApp,
) -> FollowupTemplateOverlayView {
    FollowupTemplateOverlayView {
        header_lines: vec![
            Line::from(vec![
                Span::styled(
                    "Follow-Up Templates",
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw(" / shell overlay"),
            ]),
            Line::from("Inspect the selected strategy before the next auto follow-up turn."),
        ],
        list_view: build_followup_template_list_view(app),
        preview_lines: build_followup_template_preview_lines(app),
        status_lines: build_followup_template_status_lines(app),
        key_lines: build_followup_template_key_lines(app),
    }
}

fn build_conversation_lines(app: &NativeTuiApp) -> Vec<Line<'static>> {
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
            let turn_running = conversation.has_running_turn();
            let activity_scope = conversation
                .turn_activity
                .activity_scope_label(turn_running);
            let activity_summary = conversation
                .last_auto_followup_activity
                .as_ref()
                .map(|activity| activity.summary.as_str())
                .unwrap_or("none");
            let activity_detail = conversation
                .last_auto_followup_activity
                .as_ref()
                .map(|activity| activity.detail.as_str())
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
                    "status: {}  |  {}",
                    conversation.status_text, warning_summary,
                )),
                Line::from(format!(
                    "tool activity: {}  |  {activity_scope} commands: {}  |  {activity_scope} file changes: {}",
                    conversation.turn_activity.activity_summary(turn_running),
                    conversation
                        .turn_activity
                        .activity_command_count(turn_running),
                    conversation
                        .turn_activity
                        .activity_file_change_count(turn_running),
                )),
                Line::from(format!(
                    "input detail: {}  |  template slot: {}/{}",
                    conversation.input_state.detail(),
                    conversation.auto_follow_state.selected_template_index() + 1,
                    conversation.auto_follow_state.template_count(),
                )),
                Line::from(format!(
                    "template source: {}  |  auto activity: {}  |  detail: {}",
                    conversation.auto_follow_state.template_source_label(),
                    activity_summary,
                    activity_detail,
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

pub(super) fn build_followup_template_preview_lines(app: &NativeTuiApp) -> Vec<Line<'static>> {
    match &app.conversation_state {
        ConversationState::Loading => vec![Line::from("conversation is still loading")],
        ConversationState::Failed(message) => vec![Line::from(message.clone())],
        ConversationState::Ready(conversation) => {
            let template = conversation.auto_follow_state.selected_template();
            let preview_thread_id = if conversation.thread_id.trim().is_empty() {
                "draft-thread"
            } else {
                conversation.thread_id.as_str()
            };
            let latest_agent_message = conversation.latest_agent_message_text();
            let rendered_preview = conversation
                .auto_follow_state
                .render_prompt_preview(&conversation.thread_id, latest_agent_message);

            let mut lines = vec![
                Line::from(format!("selected: {}", template.label)),
                Line::from(format!("source: {}", template.source_label())),
                Line::from(format!("preview thread id: {preview_thread_id}")),
            ];

            if latest_agent_message.is_some() {
                lines.push(Line::from(
                    "preview last_message: using the latest non-empty agent reply",
                ));
            } else {
                lines.push(Line::from(
                    "preview last_message: placeholder until an agent reply exists",
                ));
            }

            lines.push(Line::from(""));
            lines.push(Line::from("Raw Template"));
            for body_line in template.body.lines() {
                lines.push(Line::from(body_line.to_string()));
            }

            lines.push(Line::from(""));
            lines.push(Line::from("Rendered Preview"));
            for preview_line in rendered_preview.lines() {
                lines.push(Line::from(preview_line.to_string()));
            }

            lines
        }
    }
}

pub(super) fn build_followup_template_status_lines(app: &NativeTuiApp) -> Vec<Line<'static>> {
    match &app.conversation_state {
        ConversationState::Loading => vec![Line::from("conversation is still loading")],
        ConversationState::Failed(message) => vec![Line::from(message.clone())],
        ConversationState::Ready(conversation) => {
            let turn_running = conversation.has_running_turn();
            let activity_scope = conversation
                .turn_activity
                .activity_scope_label(turn_running);
            let mut lines = vec![
                Line::from(format!(
                    "auto follow-up: {}",
                    conversation.auto_follow_state.status_label()
                )),
                Line::from(format!(
                    "progress: {}",
                    conversation.auto_follow_state.progress_label()
                )),
                Line::from(format!(
                    "max auto turns: {}",
                    conversation.auto_follow_state.max_auto_turns_value()
                )),
                Line::from(format!(
                    "stop keyword: {}",
                    conversation.auto_follow_state.stop_keyword_label()
                )),
                Line::from(format!(
                    "stop on no-file-change: {}",
                    conversation.auto_follow_state.no_file_change_stop_label()
                )),
                Line::from(format!(
                    "{activity_scope} commands: {}  |  {activity_scope} file changes: {}",
                    conversation
                        .turn_activity
                        .activity_command_count(turn_running),
                    conversation
                        .turn_activity
                        .activity_file_change_count(turn_running)
                )),
                Line::from(format!(
                    "{activity_scope} tool activity: {}",
                    conversation.turn_activity.activity_summary(turn_running)
                )),
            ];

            if app.is_max_auto_turns_editing() {
                lines.push(Line::from(format!(
                    "editing max auto turns: {}  |  Enter save  |  Esc/Ctrl+C cancel",
                    app.followup_overlay_ui_state.max_auto_turns_editor.buffer
                )));
            } else if app.is_stop_keyword_editing() {
                lines.push(Line::from(format!(
                    "editing stop keyword: {}  |  Enter save  |  Esc/Ctrl+C cancel",
                    app.followup_overlay_ui_state.stop_keyword_editor.buffer
                )));
            } else {
                lines.push(Line::from(
                    "edit controls: Ctrl+l max turns  |  Ctrl+g stop keyword",
                ));
            }
            lines.push(Line::from(Span::styled(
                format!("status: {}", conversation.status_text),
                Style::default().fg(Color::Yellow),
            )));

            lines
        }
    }
}

pub(super) fn build_followup_template_key_lines(app: &NativeTuiApp) -> Vec<Line<'static>> {
    if app.is_max_auto_turns_editing() {
        return vec![
            Line::from("Type the new max-turn value directly. Backspace deletes."),
            Line::from("Enter: save max turns    Esc/Ctrl+C: cancel edit"),
            Line::from("Use a whole number between 1 and 50."),
        ];
    }

    if app.is_stop_keyword_editing() {
        return vec![
            Line::from("Type the new stop keyword directly. Backspace deletes."),
            Line::from("Enter: save stop keyword    Esc/Ctrl+C: cancel edit"),
            Line::from("Use letters, numbers, or underscores only."),
        ];
    }

    vec![
        Line::from("Up/Down or j/k: change template    Ctrl+f: next template    r: reload"),
        Line::from("PageUp/PageDown or Ctrl+u/Ctrl+d: scroll preview"),
        Line::from("Ctrl+a: auto on/off    Ctrl+l: edit max turns    Ctrl+g: edit stop keyword"),
        Line::from("Ctrl+k: stop rule on/off    Ctrl+n: no-file stop    Enter/Esc/Ctrl+C: close"),
    ]
}

fn build_shell_header_lines(app: &NativeTuiApp) -> Vec<Line<'static>> {
    match &app.conversation_state {
        ConversationState::Loading => vec![
            Line::from(vec![
                Span::styled("Conversation Shell", Style::default().fg(Color::Cyan)),
                Span::raw(" / loading thread"),
            ]),
            Line::from("Reading thread history from codex app-server."),
        ],
        ConversationState::Ready(conversation) => vec![
            Line::from(vec![
                Span::styled("Conversation Shell", Style::default().fg(Color::Cyan)),
                Span::raw(format!(" / {}", conversation.title)),
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
                    shell_action_availability_label(app),
                    startup_state_style(app),
                ),
            ]),
        ],
        ConversationState::Failed(message) => vec![
            Line::from(vec![
                Span::styled("Conversation Shell", Style::default().fg(Color::Red)),
                Span::raw(" / failed"),
            ]),
            Line::from(message.clone()),
        ],
    }
}

fn build_shell_title(mode: ShellFrontendMode) -> Line<'static> {
    match mode {
        ShellFrontendMode::InlineMainBuffer => {
            Line::from("Inline Shell / Ctrl+t new draft / Ctrl+C back / Ctrl+q quit")
        }
        ShellFrontendMode::AlternateScreen => {
            Line::from("Shell / Ctrl+t new draft / Ctrl+C back / Ctrl+q quit")
        }
    }
}

pub(super) fn build_transcript_title(app: &NativeTuiApp, mode: ShellFrontendMode) -> Line<'static> {
    match mode {
        ShellFrontendMode::InlineMainBuffer => Line::from(vec![
            Span::raw("History / "),
            Span::raw(app.transcript_viewport_status_label()),
            Span::raw(" / scrollback-first"),
        ]),
        ShellFrontendMode::AlternateScreen => Line::from(vec![
            Span::raw("Transcript / "),
            Span::raw(app.transcript_viewport_status_label()),
            Span::raw(" / PageUp PageDown / Home End"),
        ]),
    }
}

pub(super) fn build_status_title(mode: ShellFrontendMode) -> Line<'static> {
    match mode {
        ShellFrontendMode::InlineMainBuffer => Line::from(
            "Inline Controls / Ctrl+o sessions / Ctrl+d diag / Ctrl+p templ / Ctrl+a auto / Ctrl+k stop / Ctrl+n no-files / Ctrl+g stop-edit / Ctrl+l limit",
        ),
        ShellFrontendMode::AlternateScreen => Line::from(
            "Status / Ctrl+o sessions / Ctrl+d diag / Ctrl+p templ / Ctrl+a auto / Ctrl+k stop / Ctrl+n no-files / Ctrl+g stop-edit / Ctrl+l limit",
        ),
    }
}

pub(super) fn build_input_title(app: &NativeTuiApp, mode: ShellFrontendMode) -> Line<'static> {
    let prompt_label = match mode {
        ShellFrontendMode::InlineMainBuffer => "Prompt",
        ShellFrontendMode::AlternateScreen => "Composer",
    };

    match &app.conversation_state {
        ConversationState::Loading => {
            Line::from(vec![Span::raw(prompt_label), Span::raw(" / loading")])
        }
        ConversationState::Failed(_) => {
            Line::from(vec![Span::raw(prompt_label), Span::raw(" / unavailable")])
        }
        ConversationState::Ready(conversation) => {
            let submit_hint = build_primary_submit_hint(app);
            Line::from(vec![
                Span::raw(prompt_label),
                Span::raw(" / "),
                Span::styled(
                    conversation.input_state.label().to_string(),
                    input_state_style(conversation.input_state),
                ),
                Span::raw(" / startup "),
                Span::styled(
                    shell_action_availability_label(app).to_string(),
                    startup_state_style(app),
                ),
                Span::raw(" / "),
                Span::raw(submit_hint),
                Span::raw(" / Ctrl+j newline"),
            ])
        }
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

fn build_startup_check_items(app: &NativeTuiApp) -> Vec<ListItem<'static>> {
    match &app.startup_state {
        StartupState::Idle => vec![ListItem::new("startup check has not started")],
        StartupState::Loading => vec![
            ListItem::new("checking codex binary"),
            ListItem::new("opening codex app-server"),
            ListItem::new("reading account state"),
        ],
        StartupState::Ready(diagnostics) => vec![
            diagnostic_item(
                "codex binary",
                diagnostics.codex_binary_ok,
                &diagnostics.codex_binary_detail,
            ),
            diagnostic_item(
                "workspace",
                diagnostics.workspace_ok,
                &diagnostics.workspace_detail,
            ),
            diagnostic_item(
                "app-server initialize",
                diagnostics.initialize_ok,
                &diagnostics.initialize_detail,
            ),
            diagnostic_item(
                "account/read",
                diagnostics.account_ok,
                &diagnostics.account_detail,
            ),
            ListItem::new(format!("schema snapshot: {}", diagnostics.schema_snapshot)),
        ],
        StartupState::Failed(message) => vec![ListItem::new(format!("startup error: {message}"))],
    }
}

fn build_startup_warning_lines(app: &NativeTuiApp) -> Vec<Line<'static>> {
    match &app.startup_state {
        StartupState::Ready(diagnostics) if !diagnostics.warnings.is_empty() => diagnostics
            .warnings
            .iter()
            .cloned()
            .map(Line::from)
            .collect::<Vec<_>>(),
        StartupState::Failed(message) => vec![Line::from(message.clone())],
        _ => vec![Line::from("no warnings")],
    }
}

fn build_session_overlay_content(app: &NativeTuiApp) -> (OverlayListView, Vec<Line<'static>>) {
    match &app.session_state {
        SessionState::Idle => (
            OverlayListView {
                message_lines: Some(vec![Line::from(if app.can_open_session_list() {
                    "session list has not loaded yet"
                } else {
                    "recent sessions unlock after startup diagnostics pass"
                })]),
                items: Vec::new(),
                selected_index: None,
            },
            vec![Line::from(if app.can_open_session_list() {
                "session details are not available yet"
            } else {
                "startup diagnostics must pass before recent-session detail is available"
            })],
        ),
        SessionState::Loading => (
            OverlayListView {
                message_lines: Some(vec![Line::from(
                    "loading recent sessions from codex app-server",
                )]),
                items: Vec::new(),
                selected_index: None,
            },
            vec![Line::from("waiting for session list response")],
        ),
        SessionState::Failed(message) => (
            OverlayListView {
                message_lines: Some(vec![Line::from(message.clone())]),
                items: Vec::new(),
                selected_index: None,
            },
            vec![Line::from(message.clone())],
        ),
        SessionState::Ready(recent_sessions) if recent_sessions.items.is_empty() => (
            OverlayListView {
                message_lines: Some(vec![Line::from("(no sessions found)")]),
                items: Vec::new(),
                selected_index: None,
            },
            vec![Line::from("no session detail to show")],
        ),
        SessionState::Ready(recent_sessions) => {
            let browser_view = build_session_browser_view(
                recent_sessions,
                app.session_overlay_ui_state.browser_state(),
                recent_sessions
                    .items
                    .get(app.selected_session_index)
                    .map(|session| session.id.as_str()),
                app.selected_session_index,
            );
            if browser_view.visible_sessions.is_empty() {
                return (
                    OverlayListView {
                        message_lines: Some(vec![Line::from(
                            "no sessions match the current browser state",
                        )]),
                        items: Vec::new(),
                        selected_index: None,
                    },
                    vec![Line::from(
                        "no session detail to show for the current browser state",
                    )],
                );
            }

            let Some(selected_session) = browser_view.selected_session() else {
                return (
                    OverlayListView {
                        message_lines: None,
                        items: browser_view
                            .visible_sessions
                            .iter()
                            .map(|session| build_session_list_entry(session))
                            .collect(),
                        selected_index: None,
                    },
                    vec![Line::from(
                        "no session detail to show for the current browser state",
                    )],
                );
            };

            let mut lines = vec![
                Line::from(format!("id: {}", selected_session.id)),
                Line::from(format!("updated: {}", selected_session.updated_at_label())),
                Line::from(format!("workspace: {}", selected_session.cwd)),
                Line::from(format!("source: {}", selected_session.source)),
                Line::from(format!(
                    "model provider: {}",
                    selected_session.model_provider
                )),
                Line::from(format!("status: {}", selected_session.status_type)),
            ];

            if let Some(branch) = &selected_session.git_branch {
                lines.push(Line::from(format!("git branch: {branch}")));
            }

            lines.push(Line::from(format!(
                "browser: page {} of {}  |  matches: {}",
                browser_view.projection.page_index + 1,
                browser_view.projection.total_pages.max(1),
                browser_view.projection.filtered_session_count,
            )));

            if recent_sessions.next_cursor.is_some() {
                lines.push(Line::from("more threads are available in the next cursor"));
            }

            lines.push(Line::from(""));
            lines.push(Line::from("preview"));
            lines.push(Line::from(selected_session.preview_block()));
            lines.push(Line::from(""));
            lines.push(Line::from(format!("path: {}", selected_session.path)));
            (
                OverlayListView {
                    message_lines: None,
                    items: browser_view
                        .visible_sessions
                        .iter()
                        .map(|session| build_session_list_entry(session))
                        .collect(),
                    selected_index: browser_view.selected_index,
                },
                lines,
            )
        }
    }
}

fn build_session_warning_lines(app: &NativeTuiApp) -> Vec<Line<'static>> {
    match &app.session_state {
        SessionState::Ready(recent_sessions) if !recent_sessions.warnings.is_empty() => {
            recent_sessions
                .warnings
                .iter()
                .cloned()
                .map(Line::from)
                .collect::<Vec<_>>()
        }
        SessionState::Failed(message) => vec![Line::from(message.clone())],
        SessionState::Loading => vec![Line::from("waiting for app-server response")],
        SessionState::Idle if !app.can_open_session_list() => vec![Line::from(
            "recent sessions remain unavailable until startup diagnostics succeed",
        )],
        _ => vec![Line::from("no warnings")],
    }
}

fn build_followup_template_list_view(app: &NativeTuiApp) -> OverlayListView {
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
        ConversationState::Ready(conversation) => {
            let items = conversation
                .auto_follow_state
                .template_state
                .items
                .iter()
                .enumerate()
                .map(|(index, template)| build_followup_template_list_entry(index, template))
                .collect::<Vec<_>>();
            let selected_index = (!items.is_empty())
                .then_some(conversation.auto_follow_state.selected_template_index());

            OverlayListView {
                message_lines: None,
                items,
                selected_index,
            }
        }
    }
}

fn diagnostic_item(title: &str, ok: bool, detail: &str) -> ListItem<'static> {
    let marker = if ok { "[ok]" } else { "[warn]" };
    ListItem::new(format!("{marker} {title}: {detail}"))
}

fn build_session_list_entry(session: &SessionSummary) -> OverlayListEntryView {
    OverlayListEntryView {
        lines: vec![
            Line::from(format!(
                "{}  {}  {}",
                session.short_id(),
                session.updated_at_label(),
                session.workspace_label(),
            )),
            Line::from(format!(
                "{} [{} / {}]",
                session.title(),
                session.source,
                session.model_provider,
            )),
        ],
    }
}

fn build_followup_template_list_entry(
    index: usize,
    template: &FollowupTemplateDefinition,
) -> OverlayListEntryView {
    OverlayListEntryView {
        lines: vec![
            Line::from(format!("{}. {}", index + 1, template.label)),
            Line::from(format!("   {}", template.source_label())),
        ],
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
