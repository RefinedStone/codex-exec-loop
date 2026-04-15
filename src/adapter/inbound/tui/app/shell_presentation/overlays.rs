pub(super) struct StartupOverlayView {
    pub(super) header_lines: Vec<Line<'static>>,
    pub(super) summary_lines: Vec<Line<'static>>,
    pub(super) check_lines: Vec<Line<'static>>,
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

pub(super) struct QueueOverlayView {
    pub(super) header_lines: Vec<Line<'static>>,
    pub(super) summary_lines: Vec<Line<'static>>,
    pub(super) queue_lines: Vec<Line<'static>>,
    pub(super) proposal_lines: Vec<Line<'static>>,
    pub(super) note_lines: Vec<Line<'static>>,
    pub(super) key_lines: Vec<Line<'static>>,
}

pub(super) struct PlanningInitOverlayView {
    pub(super) header_lines: Vec<Line<'static>>,
    pub(super) summary_lines: Vec<Line<'static>>,
    pub(super) option_lines: Vec<Line<'static>>,
    pub(super) status_lines: Vec<Line<'static>>,
    pub(super) key_lines: Vec<Line<'static>>,
}

pub(super) struct DirectionsMaintenanceOverlayView {
    pub(super) header_lines: Vec<Line<'static>>,
    pub(super) summary_lines: Vec<Line<'static>>,
    pub(super) option_lines: Vec<Line<'static>>,
    pub(super) status_lines: Vec<Line<'static>>,
    pub(super) key_lines: Vec<Line<'static>>,
}

pub(super) struct PlanningDraftEditorOverlayView {
    pub(super) header_lines: Vec<Line<'static>>,
    pub(super) file_lines: Vec<Line<'static>>,
    pub(super) editor_title: String,
    pub(super) editor_lines: Vec<Line<'static>>,
    pub(super) editor_scroll: u16,
    pub(super) editor_cursor_offset: Option<(u16, u16)>,
    pub(super) status_lines: Vec<Line<'static>>,
    pub(super) key_lines: Vec<Line<'static>>,
}

pub(super) fn build_startup_banner_lines(
    app: &NativeTuiApp,
    max_height: Option<u16>,
) -> Option<Vec<Line<'static>>> {
    let context = ShellCorePresentationContext::from_app(app);
    if !startup_banner_is_active_in_context(&context) {
        return None;
    }

    let max_height = match max_height {
        Some(0) => return None,
        value => value,
    };

    Some(startup_ascii_art_lines(max_height))
}

pub(super) fn startup_screen_is_active(app: &NativeTuiApp) -> bool {
    startup_screen_is_active_in_context(&ShellCorePresentationContext::from_app(app))
}

fn startup_screen_is_active_in_context(context: &ShellCorePresentationContext<'_>) -> bool {
    let Some(conversation) = context.ready_conversation() else {
        return false;
    };

    !conversation.has_active_thread()
        && conversation.messages.is_empty()
        && conversation.active_turn_id.is_none()
        && conversation.live_agent_message.is_none()
}

fn startup_banner_is_active_in_context(context: &ShellCorePresentationContext<'_>) -> bool {
    context.show_startup_ascii_art && startup_screen_is_active_in_context(context)
}

#[cfg(test)]
pub(super) fn build_conversation_shell_view(
    app: &NativeTuiApp,
    mode: ShellFrontendMode,
) -> ConversationShellView {
    let _ = mode;
    let context = ShellCorePresentationContext::from_app(app);
    let plan_mode_indicator = current_plan_mode_indicator(app);
    let planning_summary_line = context.ready_conversation().and_then(|conversation| {
        build_planning_summary_line(app, conversation, FOOTER_PLANNING_DETAIL_LIMIT, false)
    });
    let planning_notice_line = context.ready_conversation().and_then(|conversation| {
        build_planning_notice_line(conversation, FOOTER_NOTICE_DETAIL_LIMIT)
    });
    let planner_panel_lines = build_planner_panel_lines(app, FOOTER_NOTICE_DETAIL_LIMIT);
    let mut header_lines = build_shell_header_lines_with_context(&context);
    header_lines.push(build_frontend_summary_line());
    let mut footer_lines = build_shell_footer_lines_with_context(
        &context,
        plan_mode_indicator,
        app.github_review_recent_changes_summary(FOOTER_NOTICE_DETAIL_LIMIT),
        planning_summary_line,
        planning_notice_line,
        planner_panel_lines,
    );
    if mode == ShellFrontendMode::InlineMainBuffer
        && let Some(live_agent_lines) = context
            .ready_conversation()
            .and_then(current_live_agent_lines)
    {
        footer_lines.extend(live_agent_lines);
    }

    ConversationShellView {
        shell_title: build_shell_title(),
        header_lines,
        conversation_lines: build_conversation_lines_with_context(&context),
        status_title: build_status_title(),
        footer_lines,
        input_title: build_input_title_with_context(&context),
        input_lines: build_input_lines_with_context(&context),
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
                Span::raw(" / shell inspection"),
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
        check_lines: build_startup_check_lines(app),
        warning_lines: build_startup_warning_lines(app),
        key_lines: vec![
            Line::from("Esc/Ctrl+C: close    r: rerun checks"),
            Line::from("Ctrl+o: recent sessions"),
        ],
    }
}

#[cfg(test)]
pub(super) fn build_conversation_shell_frame_view(
    app: &mut NativeTuiApp,
    mode: ShellFrontendMode,
    area: Rect,
) -> ConversationShellFrameView {
    let _ = mode;
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

#[cfg(test)]
pub(super) fn build_transcript_panel_view(
    app: &mut NativeTuiApp,
    lines: Vec<Line<'static>>,
    content_width: u16,
    visible_height: u16,
) -> TranscriptPanelView {
    let max_scroll_offset = build_conversation_scroll_offset(&lines, content_width, visible_height);
    let _ = visible_height;

    TranscriptPanelView {
        title: build_transcript_title_with_context(&ShellCorePresentationContext::from_app(app)),
        lines,
        scroll_offset: max_scroll_offset,
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
                Span::raw(" / shell inspection"),
            ]),
            Line::from("Resume a thread without leaving the shell view."),
        ],
        list_view,
        detail_lines,
        warning_lines: build_session_warning_lines(app),
        key_lines: build_session_key_lines(app),
    }
}

pub(super) fn build_queue_overlay_view(app: &NativeTuiApp) -> QueueOverlayView {
    let header_lines = vec![
        Line::from(vec![
            Span::styled(
                "Planning Queue",
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(" / shell inspection"),
        ]),
        Line::from("Review the next actionable work without opening raw planning files."),
    ];

    match &app.conversation_state {
        ConversationState::Loading => QueueOverlayView {
            header_lines,
            summary_lines: vec![Line::from("status: loading conversation planning state")],
            queue_lines: vec![Line::from(
                "Queue inspection becomes available after the thread loads.",
            )],
            proposal_lines: vec![Line::from("Proposal data is unavailable while loading.")],
            note_lines: vec![Line::from("No planner notes yet.")],
            key_lines: build_queue_overlay_key_lines(),
        },
        ConversationState::Failed(message) => QueueOverlayView {
            header_lines,
            summary_lines: vec![Line::from("status: conversation unavailable")],
            queue_lines: vec![Line::from(
                "Queue inspection is unavailable while the conversation failed to load.",
            )],
            proposal_lines: vec![Line::from(
                "Open a new draft or reload a session to restore planning state.",
            )],
            note_lines: vec![Line::from(format!(
                "conversation error: {}",
                compact_whitespace_detail(message, QUEUE_INSPECTION_NOTE_DETAIL_LIMIT)
            ))],
            key_lines: build_queue_overlay_key_lines(),
        },
        ConversationState::Ready(conversation) => {
            let snapshot = &conversation.planning_runtime_snapshot;
            let queue_snapshot = snapshot.queue_snapshot();
            let queue_lines = queue_snapshot
                .map(|queue_snapshot| {
                    build_queue_task_lines(
                        &queue_snapshot.active_tasks,
                        "No executable tasks in the current planning queue.",
                        QUEUE_INSPECTION_TASK_LIMIT,
                    )
                })
                .unwrap_or_else(|| match snapshot.queue_head() {
                    Some(queue_head) => build_queue_task_lines(
                        std::slice::from_ref(queue_head),
                        "No executable tasks in the current planning queue.",
                        1,
                    ),
                    None => vec![Line::from(
                        "No executable tasks in the current planning queue.",
                    )],
                });
            let proposal_lines = queue_snapshot
                .map(|queue_snapshot| {
                    build_queue_task_lines(
                        &queue_snapshot.proposed_tasks,
                        "No promotable proposals are queued right now.",
                        QUEUE_INSPECTION_PROPOSAL_LIMIT,
                    )
                })
                .unwrap_or_else(|| {
                    if let Some(summary) = snapshot.proposal_summary() {
                        vec![Line::from(format!(
                            "proposals: {}",
                            compact_whitespace_detail(summary, QUEUE_INSPECTION_NOTE_DETAIL_LIMIT)
                        ))]
                    } else {
                        vec![Line::from("No promotable proposals are queued right now.")]
                    }
                });

            let mut summary_segments = Vec::new();
            if let Some(queue_head) = snapshot.queue_head() {
                summary_segments.push(format!(
                    "next: {}",
                    compact_whitespace_detail(
                        queue_head.task_title.trim(),
                        QUEUE_INSPECTION_TITLE_DETAIL_LIMIT
                    )
                ));
            }
            if let Some(queue_summary) = snapshot.queue_summary() {
                summary_segments.push(format!(
                    "queue: {}",
                    compact_whitespace_detail(queue_summary, QUEUE_INSPECTION_NOTE_DETAIL_LIMIT)
                ));
                if snapshot.queue_head().is_none() {
                    summary_segments
                        .push(format!("policy: {}", snapshot.queue_idle_policy().label()));
                }
            }
            if let Some(proposal_summary) = snapshot.proposal_summary() {
                summary_segments.push(format!(
                    "proposals: {}",
                    compact_whitespace_detail(proposal_summary, QUEUE_INSPECTION_NOTE_DETAIL_LIMIT)
                ));
            }
            if summary_segments.is_empty() {
                summary_segments.push(format!("status: {}", snapshot.preview_status_label()));
            }
            let summary_lines = vec![Line::from(summary_segments.join("  |  "))];

            let mut note_lines = Vec::new();
            if let Some(detail) = snapshot.auto_followup_pause_reason() {
                note_lines.push(Line::from(format!(
                    "pause: {}",
                    compact_whitespace_detail(detail, QUEUE_INSPECTION_NOTE_DETAIL_LIMIT)
                )));
            } else if let Some(detail) = snapshot.failure_reason() {
                note_lines.push(Line::from(format!(
                    "blocking issue: {}",
                    compact_whitespace_detail(detail, QUEUE_INSPECTION_NOTE_DETAIL_LIMIT)
                )));
            }
            if let Some(summary) =
                conversation.planning_notice_summary(QUEUE_INSPECTION_NOTE_DETAIL_LIMIT)
            {
                note_lines.push(Line::from(format!("planning notice: {summary}")));
            }
            if let Some(queue_summary) =
                app.planner_worker_panel_state.last_queue_summary.as_deref()
            {
                note_lines.push(Line::from(format!(
                    "planner queue: {}",
                    compact_whitespace_detail(queue_summary, QUEUE_INSPECTION_NOTE_DETAIL_LIMIT)
                )));
            }
            if let Some(detail) = app.planner_worker_panel_state.last_host_detail.as_deref() {
                note_lines.push(Line::from(format!(
                    "planner host detail: {}",
                    compact_whitespace_detail(detail, QUEUE_INSPECTION_NOTE_DETAIL_LIMIT)
                )));
            }
            if let Some(detail) = queue_snapshot.and_then(|queue_snapshot| {
                build_skipped_queue_note_line(&queue_snapshot.skipped_tasks)
            }) {
                note_lines.push(detail);
            }
            if note_lines.is_empty() {
                note_lines.push(Line::from("No planner notices or skipped queue items."));
            } else {
                note_lines.truncate(2);
            }

            QueueOverlayView {
                header_lines,
                summary_lines,
                queue_lines,
                proposal_lines,
                note_lines,
                key_lines: build_queue_overlay_key_lines(),
            }
        }
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
                Span::raw(" / shell inspection"),
            ]),
            Line::from("Inspect the selected strategy before the next auto follow-up turn."),
        ],
        list_view: build_followup_template_list_view(app),
        preview_lines: build_followup_template_preview_lines(app),
        status_lines: build_followup_template_status_lines(app),
        key_lines: build_followup_template_key_lines(app),
    }
}

fn build_queue_overlay_key_lines() -> Vec<Line<'static>> {
    vec![Line::from(
        "Esc/Ctrl+C: close  |  :planning: update files  |  Ctrl+f/Ctrl+a: automation controls",
    )]
}

fn build_queue_task_lines(
    tasks: &[PriorityQueueTask],
    empty_message: &str,
    max_visible_tasks: usize,
) -> Vec<Line<'static>> {
    if tasks.is_empty() {
        return vec![Line::from(empty_message.to_string())];
    }

    let mut lines = Vec::new();
    for task in tasks.iter().take(max_visible_tasks) {
        lines.push(Line::from(format!(
            "#{} [{} / p{}] {}",
            task.rank,
            task.status.label(),
            task.combined_priority,
            compact_whitespace_detail(task.task_title.trim(), QUEUE_INSPECTION_TITLE_DETAIL_LIMIT)
        )));
    }

    let hidden_count = tasks.len().saturating_sub(max_visible_tasks);
    if hidden_count > 0 {
        lines.push(Line::from(format!(
            "+{hidden_count} more queue item{} hidden for readability",
            if hidden_count == 1 { "" } else { "s" }
        )));
    }

    lines
}

fn build_skipped_queue_note_line(
    skipped_tasks: &[PriorityQueueSkippedTask],
) -> Option<Line<'static>> {
    let first_skipped = skipped_tasks.first()?;
    Some(Line::from(format!(
        "skipped tasks: {} / {}",
        skipped_tasks.len(),
        compact_whitespace_detail(
            first_skipped.reason.as_str(),
            QUEUE_INSPECTION_NOTE_DETAIL_LIMIT
        )
    )))
}

pub(super) fn build_directions_maintenance_overlay_view(
    app: &NativeTuiApp,
) -> DirectionsMaintenanceOverlayView {
    match app.directions_maintenance_overlay_ui_state.step() {
        DirectionsMaintenanceOverlayStep::Overview => {
            let summary = app.directions_maintenance_overlay_ui_state.summary();
            let parse_error = summary.and_then(|summary| summary.parse_error.as_deref());
            let missing_doc_count = summary
                .map(|summary| summary.missing_detail_doc_count)
                .unwrap_or_default();
            let broken_doc_count = summary
                .map(|summary| summary.broken_detail_doc_count)
                .unwrap_or_default();
            let total_direction_count =
                summary.map(|summary| summary.directions.len()).unwrap_or(0);
            let queue_idle_policy = summary
                .map(|summary| summary.queue_idle_policy.label().to_string())
                .unwrap_or_else(|| "unknown".to_string());
            let queue_idle_prompt = summary
                .and_then(|summary| summary.queue_idle_prompt_path.as_deref())
                .map(|path| compact_whitespace_detail(path, QUEUE_INSPECTION_NOTE_DETAIL_LIMIT))
                .unwrap_or_else(|| "<none>".to_string());
            let queue_idle_prompt_status = summary
                .map(|summary| summary.queue_idle_prompt_status.label())
                .unwrap_or("unknown");

            DirectionsMaintenanceOverlayView {
                header_lines: vec![
                    Line::from(vec![
                        Span::styled(
                            "Directions Maintenance",
                            Style::default()
                                .fg(Color::Cyan)
                                .add_modifier(Modifier::BOLD),
                        ),
                        Span::raw(" / shell inspection"),
                    ]),
                    Line::from(
                        "Review operator-owned planning directions and queue-idle policy without editing raw files first.",
                    ),
                ],
                summary_lines: vec![
                    Line::from(
                        "Use Enter to open the staged editor, `d` to create a detail-doc mapping, or `p` to create/edit the queue-idle prompt.",
                    ),
                    Line::from(
                        "The active planning files do not change until you promote the staged draft.",
                    ),
                ],
                option_lines: vec![
                    planning_init_option_line(
                        "Enter",
                        "edit directions",
                        "open directions.toml and any existing queue-idle prompt in the staged editor",
                        false,
                        false,
                    ),
                    planning_init_option_line(
                        "D",
                        "repair detail docs",
                        "choose one direction with a missing or broken doc mapping and stage a markdown file",
                        false,
                        parse_error.is_some() || (missing_doc_count == 0 && broken_doc_count == 0),
                    ),
                    planning_init_option_line(
                        "P",
                        "edit queue-idle prompt",
                        "stage the queue-idle review prompt markdown and create or repair prompt_path if needed",
                        false,
                        parse_error.is_some(),
                    ),
                ],
                status_lines: vec![
                    Line::from(format!(
                        "directions: {total_direction_count} total / {missing_doc_count} missing docs / {broken_doc_count} broken docs"
                    )),
                    Line::from(format!(
                        "queue idle: policy {queue_idle_policy} / prompt {queue_idle_prompt_status} / {queue_idle_prompt}"
                    )),
                    Line::from(match parse_error {
                        Some(error) => format!(
                            "directions parse error: {}",
                            compact_whitespace_detail(error, QUEUE_INSPECTION_NOTE_DETAIL_LIMIT)
                        ),
                        None => "directions parsing: ok".to_string(),
                    }),
                ],
                key_lines: vec![
                    Line::from(
                        "Enter: edit directions    d: create or repair detail doc    p: edit queue-idle prompt",
                    ),
                    Line::from("r: reload summary    Esc/Ctrl+C: close"),
                ],
            }
        }
        DirectionsMaintenanceOverlayStep::DetailDocSelection => {
            let actionable_directions = app
                .directions_maintenance_overlay_ui_state
                .actionable_detail_doc_directions();
            let selected_direction = app
                .directions_maintenance_overlay_ui_state
                .selected_actionable_detail_doc_direction();
            let option_lines = if actionable_directions.is_empty() {
                vec![Line::from(
                    "Every direction already has a healthy detail-doc mapping.",
                )]
            } else {
                actionable_directions
                    .iter()
                    .map(|direction| {
                        let selected = selected_direction.is_some_and(|selected_direction| {
                            selected_direction.id == direction.id
                        });
                        let style = if selected {
                            Style::default().fg(Color::Black).bg(Color::Cyan)
                        } else {
                            Style::default().fg(Color::White)
                        };
                        let marker = if selected { ">>" } else { "  " };
                        Line::from(vec![
                            Span::styled(format!("{marker} "), style),
                            Span::styled(
                                direction.title.clone(),
                                style.add_modifier(Modifier::BOLD),
                            ),
                            Span::styled(
                                format!(
                                    "  id={} / status={} / path={}",
                                    direction.id,
                                    direction.detail_doc_status.label(),
                                    direction.detail_doc_path.as_deref().unwrap_or("<unset>")
                                ),
                                style,
                            ),
                        ])
                    })
                    .collect()
            };

            DirectionsMaintenanceOverlayView {
                header_lines: vec![
                    Line::from(vec![
                        Span::styled(
                            "Directions Maintenance",
                            Style::default()
                                .fg(Color::Cyan)
                                .add_modifier(Modifier::BOLD),
                        ),
                        Span::raw(" / detail docs"),
                    ]),
                    Line::from(
                        "Choose a direction whose detail-doc mapping should be created or repaired.",
                    ),
                ],
                summary_lines: vec![
                    Line::from(
                        "Generated docs follow `.codex-exec-loop/planning/directions/<direction-id>.md`.",
                    ),
                    Line::from(
                        "The file and `detail_doc_path` mapping are staged first and only become active after promote.",
                    ),
                ],
                option_lines,
                status_lines: vec![Line::from(format!(
                    "selected: {}",
                    selected_direction
                        .map(|direction| direction.title.as_str())
                        .unwrap_or("none")
                ))],
                key_lines: vec![
                    Line::from("Up/Down or j/k: move selection"),
                    Line::from("Enter: continue    Backspace/Left: back    Esc/Ctrl+C: close"),
                ],
            }
        }
        DirectionsMaintenanceOverlayStep::DetailDocConfirm => {
            let pending = app
                .directions_maintenance_overlay_ui_state
                .pending_detail_doc_creation();
            let direction_id = pending
                .map(|pending| pending.direction_id())
                .unwrap_or("unknown");
            let direction_title = pending
                .map(|pending| pending.direction_title())
                .unwrap_or("unknown");

            DirectionsMaintenanceOverlayView {
                header_lines: vec![
                    Line::from(vec![
                        Span::styled(
                            "Directions Maintenance",
                            Style::default()
                                .fg(Color::Cyan)
                                .add_modifier(Modifier::BOLD),
                        ),
                        Span::raw(" / confirm detail doc"),
                    ]),
                    Line::from(
                        "Open a staged detail document for the selected direction and repair the mapping if needed?",
                    ),
                ],
                summary_lines: vec![
                    Line::from(format!("direction: {direction_title}")),
                    Line::from(format!(
                        "default repair path: .codex-exec-loop/planning/directions/{direction_id}.md"
                    )),
                ],
                option_lines: vec![
                    planning_init_option_line(
                        "1",
                        "yes",
                        "stage a markdown file and open it with directions.toml for creation or repair",
                        app.directions_maintenance_overlay_ui_state
                            .detail_doc_confirm_choice()
                            == DetailDocConfirmChoice::Yes,
                        false,
                    ),
                    planning_init_option_line(
                        "2",
                        "no",
                        "return without changing the active or staged planning files",
                        app.directions_maintenance_overlay_ui_state
                            .detail_doc_confirm_choice()
                            == DetailDocConfirmChoice::No,
                        false,
                    ),
                ],
                status_lines: vec![Line::from("confirmation: generate a staged doc file now")],
                key_lines: vec![
                    Line::from("Up/Down or j/k: change selection"),
                    Line::from("Enter: act    Backspace/Left: back    Esc/Ctrl+C: close"),
                ],
            }
        }
        DirectionsMaintenanceOverlayStep::ManualEditor => DirectionsMaintenanceOverlayView {
            header_lines: vec![
                Line::from(vec![
                    Span::styled(
                        "Directions Maintenance",
                        Style::default()
                            .fg(Color::Cyan)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::raw(" / staged editor"),
                ]),
                Line::from("Edit the staged directions draft and save to re-run validation."),
            ],
            summary_lines: vec![Line::from(
                "This state renders through the dedicated draft editor view.",
            )],
            option_lines: vec![Line::from(
                "Use Tab to switch files and Ctrl+S to save + validate.",
            )],
            status_lines: vec![Line::from("editor ready")],
            key_lines: vec![Line::from("Esc/Ctrl+C: close")],
        },
    }
}

pub(super) fn build_planning_init_overlay_view(app: &NativeTuiApp) -> PlanningInitOverlayView {
    match app.planning_init_overlay_ui_state.step() {
        PlanningInitOverlayStep::ExistingWorkspace => {
            let workspace_directory = app.planning_workspace_directory();
            let snapshot = match &app.conversation_state {
                ConversationState::Ready(conversation) => {
                    conversation.planning_runtime_snapshot.clone()
                }
                ConversationState::Loading | ConversationState::Failed(_) => {
                    app.load_planning_runtime_snapshot(&workspace_directory)
                }
            };
            let plan_state_label = if snapshot.plan_enabled() {
                format!("Plan on / {}", plan_runtime_substate_label(&snapshot))
            } else {
                "Plan off".to_string()
            };
            let queue_summary = snapshot
                .queue_summary()
                .map(|summary| compact_inline_detail(summary, FOOTER_NOTICE_DETAIL_LIMIT))
                .unwrap_or_else(|| "queue state unavailable".to_string());
            let failure_summary = snapshot
                .failure_reason()
                .map(|summary| compact_inline_detail(summary, FOOTER_NOTICE_DETAIL_LIMIT));
            let mut status_lines = if snapshot.plan_enabled() {
                vec![
                    Line::from("Enter opens queue inspection for the existing planning workspace."),
                    Line::from("Press D to maintain directions, or O to turn Plan off."),
                ]
            } else {
                vec![
                    Line::from("Enter turns Plan on and resumes the existing planning workspace."),
                    Line::from("Directions maintenance stays blocked while Plan off."),
                ]
            };

            PlanningInitOverlayView {
                header_lines: vec![
                    Line::from(vec![
                        Span::styled(
                            "Planning Controls",
                            Style::default()
                                .fg(Color::Cyan)
                                .add_modifier(Modifier::BOLD),
                        ),
                        Span::raw(" / existing workspace"),
                    ]),
                    Line::from(
                        "This workspace already has active planning files. Manage the current runtime instead of restaging a bootstrap scaffold.",
                    ),
                ],
                summary_lines: vec![
                    Line::from(
                        "Use :directions only after Plan on. Hidden planner sessions still update task-ledger.json only.",
                    ),
                    Line::from(
                        "Turning Plan off keeps the workspace files on disk and blocks directions maintenance until planning resumes.",
                    ),
                ],
                option_lines: vec![
                    Line::from(format!("workspace: {workspace_directory}")),
                    Line::from(format!("state: {plan_state_label}")),
                    Line::from(format!("queue: {queue_summary}")),
                    Line::from(format!("policy: {}", snapshot.queue_idle_policy().label())),
                ],
                status_lines: {
                    if let Some(failure_summary) = failure_summary {
                        status_lines.push(Line::from(format!("failure: {failure_summary}")));
                    }
                    status_lines
                },
                key_lines: vec![
                    Line::from("Enter: open queue or resume Plan on"),
                    Line::from("Q: queue inspection    D: directions maintenance"),
                    Line::from("O: toggle Plan on/off    Esc/Ctrl+C: close"),
                ],
            }
        }
        PlanningInitOverlayStep::ModeSelection => PlanningInitOverlayView {
            header_lines: vec![
                Line::from(vec![
                    Span::styled(
                        "Planning Initialization",
                        Style::default()
                            .fg(Color::Cyan)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::raw(" / shell guidance"),
                ]),
                Line::from("Pick the planning entry path before any files are staged."),
            ],
            summary_lines: vec![
                Line::from(
                    "Every guided path stages draft files under .codex-exec-loop/planning/drafts/.",
                ),
                Line::from(
                    "Simple mode keeps one generic active direction; detail mode prepares richer direction authoring.",
                ),
            ],
            option_lines: vec![
                planning_init_option_line(
                    "A",
                    "simple mode",
                    "stage one generic direction and an empty task ledger",
                    app.planning_init_overlay_ui_state.selected_mode()
                        == PlanningInitModeSelection::Simple,
                    false,
                ),
                planning_init_option_line(
                    "B",
                    "detail mode",
                    "branch into manual or future llm-assisted authoring",
                    app.planning_init_overlay_ui_state.selected_mode()
                        == PlanningInitModeSelection::Detail,
                    false,
                ),
            ],
            status_lines: vec![
                Line::from(format!(
                    "selected: {}",
                    match app.planning_init_overlay_ui_state.selected_mode() {
                        PlanningInitModeSelection::Simple => "simple mode",
                        PlanningInitModeSelection::Detail => "detail mode",
                    }
                )),
                Line::from("simple mode is the low-ceremony path for planning-aware execution."),
            ],
            key_lines: vec![
                Line::from("A/B or arrows: move selection"),
                Line::from("Enter: continue    Esc/Ctrl+C: cancel"),
            ],
        },
        PlanningInitOverlayStep::DetailSelection => PlanningInitOverlayView {
            header_lines: vec![
                Line::from(vec![
                    Span::styled(
                        "Planning Initialization",
                        Style::default()
                            .fg(Color::Cyan)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::raw(" / detail mode"),
                ]),
                Line::from("Choose how detail-mode drafts should be prepared."),
            ],
            summary_lines: vec![
                Line::from("Manual opens the staged draft editor inside the shell."),
                Line::from("LLM-assisted remains visible for the target UX but is still disabled."),
            ],
            option_lines: vec![
                planning_init_option_line(
                    "A",
                    "manual",
                    "stage the detail scaffold and keep editing inside the shell",
                    app.planning_init_overlay_ui_state.selected_detail()
                        == PlanningInitDetailSelection::Manual,
                    false,
                ),
                planning_init_option_line(
                    "B",
                    "llm-assisted",
                    "future guided drafting flow (not supported yet)",
                    app.planning_init_overlay_ui_state.selected_detail()
                        == PlanningInitDetailSelection::LlmAssisted,
                    true,
                ),
            ],
            status_lines: vec![
                Line::from(format!(
                    "selected: {}",
                    match app.planning_init_overlay_ui_state.selected_detail() {
                        PlanningInitDetailSelection::Manual => "manual",
                        PlanningInitDetailSelection::LlmAssisted => "llm-assisted (disabled)",
                    }
                )),
                Line::from("Enter on manual opens the embedded draft editor."),
            ],
            key_lines: vec![
                Line::from("A/B or arrows: move selection"),
                Line::from("Backspace/Left: back    Enter: act    Esc/Ctrl+C: cancel"),
            ],
        },
        PlanningInitOverlayStep::SimpleReview => {
            let simple_review = app.planning_init_overlay_ui_state.simple_review();
            let draft_name = simple_review
                .map(|review| review.draft_name().to_string())
                .unwrap_or_else(|| "unknown".to_string());
            let draft_directory = simple_review
                .map(|review| review.draft_directory().to_string())
                .unwrap_or_else(|| "unknown".to_string());
            let staged_file_count = simple_review
                .map(|review| review.staged_file_count())
                .unwrap_or_default();
            let validation_report = simple_review.map(|review| review.validation_report());
            let validation_ok = validation_report.is_none_or(|report| report.is_valid());
            let first_error = validation_report
                .and_then(|report| report.errors().into_iter().next())
                .map(|issue| {
                    compact_inline_detail(issue.message.as_str(), FOOTER_NOTICE_DETAIL_LIMIT)
                });

            PlanningInitOverlayView {
                header_lines: vec![
                    Line::from(vec![
                        Span::styled(
                            "Planning Initialization",
                            Style::default()
                                .fg(Color::Cyan)
                                .add_modifier(Modifier::BOLD),
                        ),
                        Span::raw(" / simple mode"),
                    ]),
                    Line::from(
                        "Review the staged generic scaffold before it becomes active planning.",
                    ),
                ],
                summary_lines: vec![
                    Line::from(
                        "Simple mode keeps the direction catalog generic and leaves the task ledger empty.",
                    ),
                    Line::from(
                        "It also stages a default queue-idle review prompt so the first reply can seed justified follow-up work.",
                    ),
                    Line::from(
                        "No active planning files change until you explicitly promote this staged draft.",
                    ),
                ],
                option_lines: vec![
                    Line::from(format!("draft: {draft_name}")),
                    Line::from(format!("draft dir: {draft_directory}")),
                    Line::from(format!("staged files: {staged_file_count}")),
                    Line::from(
                        "Use Ctrl+E if you want to inspect or edit the staged files before promote.",
                    ),
                ],
                status_lines: {
                    let mut lines = vec![
                        Line::from(format!(
                            "validation: {}",
                            if validation_ok {
                                "ok"
                            } else {
                                "needs attention"
                            }
                        )),
                        Line::from(format!(
                            "max auto turns: {}",
                            app.current_max_auto_turns_value()
                        )),
                    ];
                    if app.is_max_auto_turns_editing() {
                        lines.push(Line::from(format!(
                            "editing max auto turns: {}  |  Enter save  |  Esc/Ctrl+C cancel",
                            app.followup_overlay_ui_state.max_auto_turns_editor.buffer
                        )));
                    } else {
                        lines.push(Line::from(
                            "next: Enter or Ctrl+P promotes the staged simple scaffold.",
                        ));
                        lines.push(Line::from(
                            "next: Esc closes this review and leaves the staged draft on disk.",
                        ));
                    }
                    if let Some(first_error) = first_error {
                        lines.push(Line::from(format!("first validation error: {first_error}")));
                    }
                    lines
                },
                key_lines: if app.is_max_auto_turns_editing() {
                    vec![
                        Line::from("Type the new max-turn value directly. Backspace deletes."),
                        Line::from("Enter: save max turns    Esc/Ctrl+C: cancel edit"),
                        Line::from("Use a whole number between 1 and 50."),
                    ]
                } else {
                    vec![
                        Line::from("Enter/Ctrl+P: promote staged scaffold"),
                        Line::from("Ctrl+L: edit max auto turns    Ctrl+E: inspect/edit draft"),
                        Line::from("Esc/Ctrl+C: close review"),
                    ]
                },
            }
        }
        PlanningInitOverlayStep::ManualEditor => PlanningInitOverlayView {
            header_lines: vec![
                Line::from(vec![
                    Span::styled(
                        "Planning Draft Editor",
                        Style::default()
                            .fg(Color::Cyan)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::raw(" / staged detail draft"),
                ]),
                Line::from("Edit the staged planning draft and save to re-run validation."),
            ],
            summary_lines: vec![Line::from(
                "This state renders through the dedicated planning draft editor view.",
            )],
            option_lines: vec![Line::from(
                "Use Tab to switch files and Ctrl+S to save + validate.",
            )],
            status_lines: vec![Line::from("editor ready")],
            key_lines: vec![Line::from("Esc/Ctrl+C: close")],
        },
    }
}

pub(super) fn build_planning_draft_editor_overlay_view(
    app: &NativeTuiApp,
    editor_height: u16,
) -> Option<PlanningDraftEditorOverlayView> {
    let buffers = app.planning_draft_editor_ui_state.buffers()?;
    let selected_index = app.planning_draft_editor_ui_state.selected_file_index()?;
    let selected_buffer = app.planning_draft_editor_ui_state.selected_buffer()?;
    let dirty_labels = app.planning_draft_editor_ui_state.dirty_file_labels();
    let validation_report = app.planning_draft_editor_ui_state.validation_report()?;
    let pending_close_risk = app.planning_draft_editor_ui_state.pending_close_risk();
    let close_risk = pending_close_risk.or_else(|| app.planning_draft_editor_ui_state.close_risk());
    let next_action = if !dirty_labels.is_empty() {
        "next action: Ctrl+S re-runs validation, or Ctrl+P saves current edits and promotes if valid"
    } else if validation_report.is_valid() {
        "next action: Ctrl+P promotes this draft into active planning files"
    } else {
        "next action: fix validation errors before promoting this draft"
    };

    let file_lines = buffers
        .iter()
        .enumerate()
        .map(|(index, buffer)| {
            let selected = index == selected_index;
            let dirty_suffix = if buffer.is_dirty() { " *dirty" } else { "" };
            let style = if selected {
                Style::default().fg(Color::Black).bg(Color::Cyan)
            } else if buffer.is_dirty() {
                Style::default().fg(Color::Yellow)
            } else {
                Style::default().fg(Color::White)
            };
            let marker = if selected { ">>" } else { "  " };
            Line::from(vec![
                Span::styled(format!("{marker} "), style),
                Span::styled(buffer.file_label(), style.add_modifier(Modifier::BOLD)),
                Span::styled(dirty_suffix.to_string(), style),
            ])
        })
        .collect::<Vec<_>>();

    let editor_lines = selected_buffer
        .lines()
        .iter()
        .map(|line| Line::from(line.clone()))
        .collect::<Vec<_>>();
    let editor_height = editor_height.max(1) as usize;
    let max_editor_scroll = selected_buffer
        .lines()
        .len()
        .saturating_sub(editor_height)
        .min(u16::MAX as usize) as u16;
    let editor_scroll = selected_buffer.editor_scroll().min(max_editor_scroll);
    let editor_cursor_offset = Some((
        selected_buffer.cursor_column().min(u16::MAX as usize) as u16,
        selected_buffer
            .cursor_line_index()
            .saturating_sub(editor_scroll as usize)
            .min(u16::MAX as usize) as u16,
    ));

    let mut status_lines = vec![
        Line::from(format!(
            "draft: {}",
            app.planning_draft_editor_ui_state
                .draft_name()
                .unwrap_or("unknown")
        )),
        Line::from(format!(
            "file: {} ({}/{})",
            selected_buffer.active_path(),
            selected_index + 1,
            buffers.len()
        )),
        Line::from(vec![
            Span::styled("validation: ", Style::default().fg(Color::Gray)),
            Span::styled(
                if validation_report.is_valid() {
                    "ok"
                } else {
                    "needs attention"
                },
                Style::default().fg(if validation_report.is_valid() {
                    Color::Green
                } else {
                    Color::Yellow
                }),
            ),
        ]),
    ];
    if let Some(issue) = validation_report.issues.first() {
        status_lines.push(Line::from(vec![
            Span::styled(
                match issue.severity {
                    PlanningValidationSeverity::Error => "error: ",
                    PlanningValidationSeverity::Warning => "warning: ",
                },
                Style::default().fg(match issue.severity {
                    PlanningValidationSeverity::Error => Color::Red,
                    PlanningValidationSeverity::Warning => Color::Yellow,
                }),
            ),
            Span::raw(compact_inline_detail(
                &issue.message,
                FOOTER_NOTICE_DETAIL_LIMIT,
            )),
        ]));
    } else {
        status_lines.push(Line::from(format!(
            "staged path: {}",
            compact_inline_detail(selected_buffer.staged_path(), FOOTER_NOTICE_DETAIL_LIMIT)
        )));
    }
    status_lines.push(Line::from(format!(
        "dirty: {}",
        if dirty_labels.is_empty() {
            "none".to_string()
        } else {
            compact_inline_detail(&dirty_labels.join(", "), FOOTER_NOTICE_DETAIL_LIMIT)
        }
    )));
    if !dirty_labels.is_empty() {
        status_lines.push(Line::from(
            "validation note: the status above reflects the last saved draft until Ctrl+S re-runs checks",
        ));
    }
    status_lines.push(Line::from(next_action));
    if let Some(risk) = close_risk {
        status_lines.push(Line::from(vec![
            Span::styled(
                if pending_close_risk.is_some() {
                    "close pending: "
                } else {
                    "close guard: "
                },
                Style::default().fg(if pending_close_risk.is_some() {
                    Color::Red
                } else {
                    Color::Yellow
                }),
            ),
            Span::raw(planning_draft_close_guard_detail(
                risk,
                pending_close_risk.is_some(),
            )),
        ]));
    }

    Some(PlanningDraftEditorOverlayView {
        header_lines: vec![
            Line::from(vec![
                Span::styled(
                    "Planning Draft Editor",
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw(match app.planning_init_overlay_ui_state.selected_mode() {
                    PlanningInitModeSelection::Simple => " / simple scaffold editor",
                    PlanningInitModeSelection::Detail => " / detail draft editor",
                }),
            ]),
            Line::from(format!(
                "draft dir: {}",
                app.planning_draft_editor_ui_state
                    .draft_directory()
                    .unwrap_or("unknown")
            )),
        ],
        file_lines,
        editor_title: selected_buffer.file_label(),
        editor_lines,
        editor_scroll,
        editor_cursor_offset,
        status_lines,
        key_lines: vec![
            Line::from("Tab/BackTab: switch file    arrows: move cursor"),
            Line::from("Enter: newline    Backspace: delete    Ctrl+W: delete previous word"),
            Line::from("Ctrl+S: save + validate    Ctrl+P: save + promote active planning"),
            planning_draft_editor_close_key_line(close_risk, pending_close_risk.is_some()),
        ],
    })
}

fn planning_draft_close_guard_detail(
    risk: super::planning_draft_editor_ui::PlanningDraftEditorCloseRisk,
    confirmation_pending: bool,
) -> String {
    match (
        risk.has_dirty_buffers(),
        risk.has_invalid_staged_draft(),
        confirmation_pending,
    ) {
        (true, true, true) => {
            "discard unsaved edits or keep editing; the invalid staged draft will remain on disk"
                .to_string()
        }
        (true, false, true) => "discard unsaved edits or press n to keep editing".to_string(),
        (false, true, true) => {
            "close now or press n to keep editing; the invalid staged draft will remain on disk"
                .to_string()
        }
        (true, true, false) => {
            "unsaved edits and an invalid staged draft require confirmation before close"
                .to_string()
        }
        (true, false, false) => "unsaved edits require confirmation before close".to_string(),
        (false, true, false) => {
            "an invalid staged draft requires confirmation before close".to_string()
        }
        (false, false, _) => "close is available immediately".to_string(),
    }
}

fn planning_draft_editor_close_key_line(
    close_risk: Option<super::planning_draft_editor_ui::PlanningDraftEditorCloseRisk>,
    confirmation_pending: bool,
) -> Line<'static> {
    if confirmation_pending {
        return Line::from("Enter/Esc/Ctrl+C: confirm close    n: keep editing");
    }

    if close_risk.is_some() {
        return Line::from("Esc/Ctrl+C: review close");
    }

    Line::from("Esc/Ctrl+C: close")
}

fn planning_init_option_line(
    shortcut: &str,
    label: &str,
    detail: &str,
    selected: bool,
    disabled: bool,
) -> Line<'static> {
    let style = if disabled {
        Style::default().fg(Color::DarkGray)
    } else if selected {
        Style::default().fg(Color::Black).bg(Color::Cyan)
    } else {
        Style::default().fg(Color::White)
    };
    let marker = if selected { ">>" } else { "  " };

    Line::from(vec![
        Span::styled(format!("{marker} {shortcut}. "), style),
        Span::styled(label.to_string(), style.add_modifier(Modifier::BOLD)),
        Span::styled(format!("  {detail}"), style),
    ])
}
