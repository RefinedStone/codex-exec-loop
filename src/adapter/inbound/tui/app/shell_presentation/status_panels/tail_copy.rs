use ratatui::text::{Line, Span};

use super::super::capability_copy::{
    startup_initializing_status_line, startup_preparing_status_line,
    thread_history_loading_status_line,
};
use super::super::planning::build_planning_worker_panel_lines;
use super::super::planning::status_projection::build_planning_status_surface_projection;
use super::super::prompt_composer::{build_prompt_buffer_view, build_shell_command_palette_lines};
use super::super::{
    AkraTheme, ConversationComposerScreenModel, ConversationInputState, ConversationScreenModel,
    ConversationViewModel, InlineShellCommand, InlineShellCommandAvailability,
    InlineShellCommandCapabilitySet, InlineShellCommandInput, Modifier, QueueMutationTailState,
    SHELL_TAIL_NOTICE_DETAIL_LIMIT, SHELL_TAIL_PLANNING_DETAIL_LIMIT,
    SHELL_TAIL_STATUS_DETAIL_LIMIT, ShellActionAvailability, ShellConversationState, ShellOverlay,
    StartupState, TuiLanguage, build_working_line, compact_shell_detail,
};
use super::operator_ribbon::{build_operator_attention_line, build_operator_ribbon_line};
use super::parallel_working_copy::build_parallel_slot_working_line;
use super::tail_shared::{
    OperatorNoticeKind, build_operator_notice, parallel_mode_alert_line, parallel_mode_summary_line,
};

pub(super) const QUEUE_RECEIPT_UNDO_ACTION_LABEL: &str = "[ Undo queue ]";

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum ShellTailPriority {
    Pinned,
    Terminal,
    Warning,
    LiveActivity,
    RecentActivity,
    Identity,
    Detail,
}

#[derive(Clone)]
pub(super) struct ShellTailLine {
    pub(super) line: Line<'static>,
    pub(super) priority: ShellTailPriority,
}

impl ShellTailLine {
    fn new(priority: ShellTailPriority, line: Line<'static>) -> Self {
        Self { line, priority }
    }
}

/* The shell tail is the compact operational dashboard below the transcript. It
 * keeps high-priority state visible in this order: startup readiness, conversation
 * turn state, parallel/planning health, recent transcript context, then prompt
 * affordances. The order matters because this view is scanned repeatedly while a
 * turn is streaming or while startup checks are blocking submission.
 */
pub(super) fn build_shell_tail_content_with_context(
    screen_model: &ConversationScreenModel<'_>,
    github_review_recent_changes_summary: Option<String>,
    notice_detail_limit: usize,
    content_width: u16,
) -> Vec<ShellTailLine> {
    /*
    Planning projection is computed before state branching because both the ready
    tail and prompt affordance lines need the same compact limits. Keeping this
    projection renderer-adjacent prevents lower application services from knowing
    about terminal row budgets.
    */
    let planning_status_projection = screen_model.ready_conversation().map(|conversation| {
        build_planning_status_surface_projection(
            &screen_model.planning_runtime_projection,
            conversation,
            SHELL_TAIL_PLANNING_DETAIL_LIMIT,
            SHELL_TAIL_NOTICE_DETAIL_LIMIT,
            false,
        )
    });
    let planning_worker_panel_lines = build_planning_worker_panel_lines(
        screen_model.planning_worker_shows_debug_details,
        &screen_model.planning_worker_panel_state,
        SHELL_TAIL_NOTICE_DETAIL_LIMIT,
    );

    if screen_model.startup_screen_is_active() {
        let has_buffered_input = screen_model
            .composer()
            .is_some_and(|composer| !composer.state.input_buffer.is_empty());
        // The full startup masthead is useful only before the operator starts typing.
        // Once an overlay or buffered prompt exists, keep the tail compact so the
        // prompt remains close to its status line.
        let mut lines =
            if screen_model.shell_overlay == ShellOverlay::Hidden && !has_buffered_input {
                build_startup_screen_lines_with_context(screen_model, content_width)
            } else {
                build_startup_overlay_tail_lines_with_context(screen_model, content_width)
            }
            .into_iter()
            .map(|line| ShellTailLine::new(ShellTailPriority::Detail, line))
            .collect::<Vec<_>>();
        if screen_model.recent_session_status_requires_attention {
            lines.push(ShellTailLine::new(
                ShellTailPriority::Warning,
                Line::from(format!(
                    "session: {}",
                    screen_model.recent_session_status_label
                )),
            ));
        }
        lines.extend(
            build_shell_tail_prompt_lines_with_context(screen_model, content_width)
                .into_iter()
                .map(|line| ShellTailLine::new(ShellTailPriority::Pinned, line)),
        );
        return lines;
    }
    let mut lines = Vec::new();
    match screen_model.conversation_state {
        state @ (ShellConversationState::Loading | ShellConversationState::Failed(_)) => {
            /*
            Loading and failed states still render a full tail because the inline
            terminal layout needs stable prompt/status rows even before a thread
            snapshot exists. These branches avoid conversation-only helpers.
            */
            let (thread_status, runtime_status, status_line) = match state {
                ShellConversationState::Loading => (
                    "loading",
                    "loading thread history",
                    Line::from(thread_history_loading_status_line()),
                ),
                ShellConversationState::Failed(message) => (
                    "unavailable",
                    "unavailable",
                    Line::from(format!("status: {message}")),
                ),
                ShellConversationState::Ready(_) => {
                    unreachable!("ready conversation uses the ready tail branch")
                }
            };
            lines.push(ShellTailLine::new(
                ShellTailPriority::Identity,
                Line::from(format!(
                    "Akra  |  thread: {thread_status}  |  startup: {}  |  sessions: {}",
                    screen_model.shell_action_availability.status_text(),
                    screen_model.recent_session_status_label.as_str(),
                )),
            ));
            let github_status =
                (screen_model.github_review_polling_status_label != "off").then(|| {
                    format!(
                        "  |  gh: {}",
                        screen_model.github_review_polling_status_label.as_str()
                    )
                });
            lines.push(ShellTailLine::new(
                ShellTailPriority::Warning,
                Line::from(format!(
                    "runtime: {runtime_status}{}  |  flow: terminal main buffer",
                    github_status.unwrap_or_default(),
                )),
            ));
            lines.push(ShellTailLine::new(ShellTailPriority::Detail, status_line));
        }
        ShellConversationState::Ready(conversation) => {
            /*
            Ready-state ordering is intentionally dense: identity and turn status
            first, then operator health signals, parallel/planning summaries,
            worker detail, recent transcript context, and finally notices. This
            mirrors how operators scan the tail while a turn is active.
            */
            let runtime_status = screen_model
                .runtime_status()
                .expect("ready conversation must retain its runtime status projection");
            lines.push(ShellTailLine::new(
                ShellTailPriority::Identity,
                build_ready_status_ribbon_line(conversation, screen_model, content_width),
            ));
            if screen_model.recent_session_status_requires_attention {
                lines.push(ShellTailLine::new(
                    ShellTailPriority::Warning,
                    Line::from(format!(
                        "session: {}",
                        screen_model.recent_session_status_label
                    )),
                ));
            }
            if let Some(status_detail_line) =
                build_ready_status_detail_line(conversation, screen_model)
            {
                let priority =
                    if screen_model.shell_action_availability == ShellActionAvailability::Ready {
                        ShellTailPriority::Detail
                    } else {
                        ShellTailPriority::Warning
                    };
                lines.push(ShellTailLine::new(priority, status_detail_line));
            }
            if let Some(completion_line) = build_completion_alert_line(conversation) {
                lines.push(ShellTailLine::new(
                    ShellTailPriority::Pinned,
                    completion_line,
                ));
            }
            if let Some(queue_mutation_line) = build_queue_mutation_line(
                screen_model.queue_mutation_tail_state,
                screen_model.tui_language,
            ) {
                lines.push(ShellTailLine::new(
                    ShellTailPriority::Pinned,
                    queue_mutation_line,
                ));
            }
            if let Some(attention_line) =
                build_operator_attention_line(screen_model, Some(conversation), content_width)
            {
                lines.push(ShellTailLine::new(
                    ShellTailPriority::Warning,
                    attention_line,
                ));
            }
            if let Some(turn_options_summary) = screen_model.turn_options_summary.as_deref() {
                lines.push(ShellTailLine::new(
                    ShellTailPriority::Detail,
                    Line::from(format!("turn options: {turn_options_summary}")),
                ));
            }
            if let Some(parallel_summary_line) = parallel_mode_summary_line(screen_model) {
                lines.push(ShellTailLine::new(
                    ShellTailPriority::Detail,
                    Line::from(parallel_summary_line),
                ));
            }

            if let Some(parallel_mode_alert_line) = parallel_mode_alert_line(screen_model) {
                lines.push(ShellTailLine::new(
                    ShellTailPriority::Warning,
                    Line::from(parallel_mode_alert_line),
                ));
            }
            let working_detail_limit =
                SHELL_TAIL_STATUS_DETAIL_LIMIT.min(notice_detail_limit.saturating_sub(9));
            let activity_overlay_owns_exact_wait = screen_model.shell_overlay
                == ShellOverlay::Activity
                && runtime_status.wait_status.is_some();
            if !activity_overlay_owns_exact_wait
                && let Some(working_line) = build_working_line(
                    runtime_status,
                    working_detail_limit,
                    content_width,
                    screen_model.rendered_at,
                )
            {
                lines.push(ShellTailLine::new(
                    ShellTailPriority::LiveActivity,
                    working_line,
                ));
            }
            if let Some(planning_projection) = planning_status_projection.as_ref() {
                if let Some(planning_line) = planning_projection.summary_line.as_deref() {
                    let priority = if planning_projection.summary_is_warning {
                        ShellTailPriority::Warning
                    } else {
                        ShellTailPriority::Detail
                    };
                    lines.push(ShellTailLine::new(
                        priority,
                        Line::from(planning_line.to_string()),
                    ));
                }
                lines.extend(planning_projection.queue_framing_lines.iter().cloned().map(
                    |entry| {
                        let priority = if entry.has_blocker {
                            ShellTailPriority::Warning
                        } else {
                            ShellTailPriority::Detail
                        };
                        ShellTailLine::new(priority, entry.line)
                    },
                ));
                if let Some(planning_notice_line) = planning_projection.notice_line.as_deref() {
                    lines.push(ShellTailLine::new(
                        ShellTailPriority::Warning,
                        Line::from(planning_notice_line.to_string()),
                    ));
                }
            } else {
                lines.push(ShellTailLine::new(
                    ShellTailPriority::Warning,
                    Line::from(format!(
                        "planning: unavailable  |  startup: {}",
                        screen_model.shell_action_availability.status_text()
                    )),
                ));
            }
            if let Some(parallel_working_line) = build_parallel_slot_working_line(screen_model) {
                lines.push(ShellTailLine::new(
                    ShellTailPriority::LiveActivity,
                    parallel_working_line,
                ));
            }

            lines.extend(
                planning_worker_panel_lines
                    .into_iter()
                    .map(|line| ShellTailLine::new(ShellTailPriority::Detail, Line::from(line))),
            );
            if let Some(notice) = build_operator_notice(
                github_review_recent_changes_summary.as_deref(),
                conversation,
                SHELL_TAIL_NOTICE_DETAIL_LIMIT,
                notice_detail_limit,
            ) {
                lines.push(ShellTailLine::new(
                    operator_notice_priority(notice.kind),
                    Line::from(format!("notice: {}", notice.text)),
                ));
            }
        }
    }

    lines.extend(
        build_shell_tail_prompt_lines_with_context(screen_model, content_width)
            .into_iter()
            .map(|line| ShellTailLine::new(ShellTailPriority::Pinned, line)),
    );
    lines
}

fn operator_notice_priority(kind: OperatorNoticeKind) -> ShellTailPriority {
    match kind {
        OperatorNoticeKind::RequiredAction => ShellTailPriority::Pinned,
        OperatorNoticeKind::TerminalActivity => ShellTailPriority::Terminal,
        OperatorNoticeKind::Activity => ShellTailPriority::RecentActivity,
        OperatorNoticeKind::Detail => ShellTailPriority::Detail,
    }
}
fn build_ready_status_ribbon_line(
    conversation: &ConversationViewModel,
    screen_model: &ConversationScreenModel<'_>,
    content_width: u16,
) -> Line<'static> {
    build_operator_ribbon_line(screen_model, Some(conversation), content_width)
}

fn build_queue_mutation_line(
    state: QueueMutationTailState,
    language: TuiLanguage,
) -> Option<Line<'static>> {
    if let QueueMutationTailState::Pending(operation_id) = state {
        // This status intentionally omits the undo action label so layout cannot bind a stale mouse target.
        return Some(Line::from(vec![
            Span::styled(
                language.queue_mutation_tail_pending_label(operation_id),
                AkraTheme::warning(),
            ),
            Span::raw(language.queue_mutation_tail_pending_detail()),
        ]));
    }
    if state == QueueMutationTailState::RefreshRequired {
        return Some(Line::from(vec![
            Span::styled(
                language.queue_mutation_tail_refresh_label(),
                AkraTheme::warning(),
            ),
            Span::raw(language.queue_mutation_tail_refresh_detail()),
        ]));
    }

    match state {
        QueueMutationTailState::UndoAvailable(task_count) => {
            Some(build_queue_receipt_undo_action_line(task_count))
        }
        QueueMutationTailState::Idle
        | QueueMutationTailState::Pending(_)
        | QueueMutationTailState::RefreshRequired => None,
    }
}

fn build_queue_receipt_undo_action_line(queued_task_count: usize) -> Line<'static> {
    let task_label = if queued_task_count == 1 {
        "task"
    } else {
        "tasks"
    };
    Line::from(vec![
        Span::styled(QUEUE_RECEIPT_UNDO_ACTION_LABEL, AkraTheme::inline_action()),
        Span::raw(format!(
            "  click to cancel {queued_task_count} queued {task_label}  |  keyboard: :queue then u"
        )),
    ])
}

fn build_ready_status_detail_line(
    conversation: &ConversationViewModel,
    screen_model: &ConversationScreenModel<'_>,
) -> Option<Line<'static>> {
    let status = conversation.status_text_for_viewport().trim();
    let terminal_status_is_owned_by_notice = conversation.activity_rail_terminal_state.is_some()
        && status.eq_ignore_ascii_case("turn failed");
    let draft_status_is_redundant = status.eq_ignore_ascii_case("new thread draft");
    let settlement_status_is_transient =
        status.eq_ignore_ascii_case("turn completed / evaluating post-turn continuation");
    let mut parts = Vec::new();
    if !status.is_empty()
        && !status.eq_ignore_ascii_case("turn started")
        && !terminal_status_is_owned_by_notice
        && !draft_status_is_redundant
        && !settlement_status_is_transient
    {
        parts.push(format!(
            "status: {}",
            compact_shell_detail(status, SHELL_TAIL_STATUS_DETAIL_LIMIT)
        ));
    }
    if screen_model.shell_action_availability != ShellActionAvailability::Ready {
        parts.push(format!(
            "startup: {}",
            screen_model.shell_action_availability.status_text()
        ));
    }
    let github_status = screen_model.github_review_polling_status_label.as_str();
    if github_status != "off" {
        parts.push(format!("gh: {github_status}"));
    }

    (!parts.is_empty()).then(|| Line::from(parts.join("  |  ")))
}

fn build_completion_alert_line(conversation: &ConversationViewModel) -> Option<Line<'static>> {
    let activity = conversation.last_auto_follow_activity.as_ref()?;
    if activity.summary != "complete: planning queue drained" {
        return None;
    }

    Some(Line::from(vec![
        Span::styled(
            "COMPLETE".to_string(),
            AkraTheme::success().add_modifier(Modifier::BOLD),
        ),
        Span::raw("  |  all planning tasks complete  |  no actionable or proposed work remains"),
    ]))
}

fn build_startup_screen_lines_with_context(
    screen_model: &ConversationScreenModel<'_>,
    content_width: u16,
) -> Vec<Line<'static>> {
    /*
    The startup masthead is allowed to be taller than the steady-state tail
    because no transcript exists yet. Once the operator starts typing, callers
    switch to the compact startup overlay tail to keep the prompt close to hand.
    */
    let mut lines = vec![build_operator_ribbon_line(
        screen_model,
        screen_model.ready_conversation(),
        content_width,
    )];
    if let Some(attention_line) = build_operator_attention_line(
        screen_model,
        screen_model.ready_conversation(),
        content_width,
    ) {
        lines.push(attention_line);
    }
    match screen_model.startup_state {
        StartupState::Idle => {
            lines.push(Line::from(startup_preparing_status_line()));
        }
        StartupState::Loading => {
            lines.push(Line::from(startup_initializing_status_line()));
        }
        StartupState::Ready(_) => {}
        StartupState::Failed(message) => {
            lines.push(Line::from(
                screen_model.tui_language.startup_status_line(message),
            ));
        }
    }
    lines
}

fn build_startup_overlay_tail_lines_with_context(
    screen_model: &ConversationScreenModel<'_>,
    content_width: u16,
) -> Vec<Line<'static>> {
    /*
    Compact startup tail is deliberately a single operational axis row. It is used
    while overlays or buffered input need vertical space, so detailed diagnostics
    stay available through inspection instead of crowding the prompt.
    */
    let mut lines = vec![build_operator_ribbon_line(
        screen_model,
        screen_model.ready_conversation(),
        content_width,
    )];
    if let Some(attention_line) = build_operator_attention_line(
        screen_model,
        screen_model.ready_conversation(),
        content_width,
    ) {
        lines.push(attention_line);
    }
    lines
}

pub(super) fn build_shell_tail_prompt_lines_with_context(
    screen_model: &ConversationScreenModel<'_>,
    content_width: u16,
) -> Vec<Line<'static>> {
    /*
    Prompt copy is separated from the status body because layout code also uses
    this function to compute cursor offsets. Loading/failed states get static
    affordance rows; ready state delegates to the input-aware branch below.
    */
    if screen_model.shell_overlay == ShellOverlay::Approval {
        return vec![Line::styled(
            screen_model.tui_language.composer_approval_action(),
            AkraTheme::subtle(),
        )];
    }
    let mut lines = match screen_model.conversation_state {
        ShellConversationState::Loading => vec![
            Line::styled(
                screen_model.tui_language.composer_loading_status(),
                AkraTheme::subtle(),
            ),
            composer_action_line(screen_model.tui_language.composer_loading_action()),
        ],
        ShellConversationState::Failed(message) => vec![
            Line::styled(
                screen_model.tui_language.composer_unavailable_status(),
                AkraTheme::danger(),
            ),
            composer_action_line(&format!(
                "{}  |  {message}",
                screen_model.tui_language.composer_startup_blocked_action()
            )),
        ],
        ShellConversationState::Ready(_) => {
            let composer = screen_model
                .composer()
                .expect("ready conversation must project composer state");
            build_ready_prompt_lines(
                composer,
                &screen_model.inline_shell_command_capabilities,
                screen_model.shell_action_availability,
                screen_model.tui_language,
                content_width,
            )
        }
    };
    if screen_model.parallel_mode_loading_prompt_indicator_visible
        && let Some(first_line) = lines.first_mut()
    {
        first_line.spans.insert(
            0,
            Span::styled(
                format!(
                    "{} ",
                    parallel_loading_prompt_indicator_frame(screen_model.animation_elapsed_millis,)
                ),
                AkraTheme::brand(),
            ),
        );
        if let Some(action_line) = lines.last_mut() {
            *action_line =
                composer_action_line(screen_model.tui_language.composer_parallel_loading_action());
        }
    }
    lines
}

fn composer_action_line(copy: &str) -> Line<'static> {
    Line::styled(format!(" {copy} "), AkraTheme::shortcut())
}

fn parallel_loading_prompt_indicator_frame(animation_elapsed_millis: u128) -> &'static str {
    const FRAMES: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
    let tick = (animation_elapsed_millis / 120) as usize;
    FRAMES[tick % FRAMES.len()]
}

fn build_ready_prompt_lines(
    composer: &ConversationComposerScreenModel<'_>,
    inline_shell_command_capabilities: &InlineShellCommandCapabilitySet,
    shell_action_availability: ShellActionAvailability,
    language: TuiLanguage,
    content_width: u16,
) -> Vec<Line<'static>> {
    let prompt_buffer = build_prompt_buffer_view(composer);
    let mut lines = prompt_buffer.lines;
    if composer.state.input_buffer.is_empty() {
        lines[0].spans.push(Span::styled(
            language.composer_placeholder(),
            AkraTheme::subtle(),
        ));
    }

    // Empty prompt copy prioritizes what blocks or enables the next Enter press.
    // Buffered prompt copy instead explains what will happen to the typed text.
    if composer.state.input_buffer.is_empty() {
        /*
        Empty-buffer copy is command guidance rather than content preview. It
        must explain whether Enter can send immediately, is gated by startup, or
        is blocked by a running/paused automation state.
        */
        if composer.post_turn_settlement_in_flight {
            lines.push(composer_action_line("Type now  |  Enter when settled"));
            return lines;
        }
        if composer.auto_follow_has_live_activity {
            lines.push(composer_action_line("Type now  |  Enter when idle"));
            return lines;
        }
        let line = match (composer.input_state, shell_action_availability) {
            (_, ShellActionAvailability::Pending) if composer.input_state.can_submit_now() => {
                language.composer_startup_pending_action().to_string()
            }
            (_, ShellActionAvailability::Blocked) if composer.input_state.can_submit_now() => {
                language.composer_startup_blocked_action().to_string()
            }
            (ConversationInputState::DraftReady | ConversationInputState::ReadyToContinue, _) => {
                language.composer_empty_action().to_string()
            }
            (ConversationInputState::SubmittingTurn, _) => {
                language.turn_starting_prompt_hint(false).to_string()
            }
            (ConversationInputState::StreamingTurn, _) => {
                language.composer_streaming_empty_action().to_string()
            }
        };
        lines.push(composer_action_line(&line));
        return lines;
    }

    if composer
        .state
        .inline_shell_command_palette_state
        .is_active()
    {
        /*
        Palette copy takes precedence over raw command parsing because the
        operator is navigating an already-open menu; showing parse hints here
        would fight with Up/Down/Enter semantics.
        */
        let palette = &composer.state.inline_shell_command_palette_state;
        let selected = palette.selected_index().map_or(0, |index| index + 1);
        lines.push(Line::from(language.inline_command_palette_header(
            selected,
            palette.suggestions().len(),
        )));
        lines.extend(build_shell_command_palette_lines(
            composer,
            inline_shell_command_capabilities,
            language,
            content_width,
        ));
        let selected_command = palette.selected_command();
        let selected_availability = selected_command
            .map(|command| inline_shell_command_capabilities.availability(command))
            .unwrap_or(InlineShellCommandAvailability::Ready);
        lines.push(composer_action_line(language.composer_palette_action(
            !palette.suggestions().is_empty(),
            selected_availability.is_ready(),
            selected_command.is_some_and(InlineShellCommand::requires_argument),
        )));
        return lines;
    }

    if let Some(command) = InlineShellCommandInput::parse(&composer.state.input_buffer) {
        /*
        Parsed shell commands get a dedicated hint line before generic prompt
        guidance. That keeps destructive or overlay-opening commands legible
        while the text is still just buffered input.
        */
        lines.push(composer_action_line(
            &command.localized_buffered_hint(language),
        ));
        return lines;
    }

    if composer.post_turn_settlement_in_flight && composer.input_state.can_submit_now() {
        lines.push(composer_action_line(
            "Draft ready  |  Enter when settled  |  Ctrl+J newline",
        ));
        return lines;
    }

    if composer.auto_follow_has_live_activity && composer.input_state.can_submit_now() {
        /*
        Auto follow-up activity can make the shell appear idle enough to type into,
        but Enter would race the continuation. This line keeps the buffered prompt
        visible while making the idle gate explicit.
        */
        lines.push(composer_action_line(
            "Draft ready  |  auto-follow busy  |  Enter when idle",
        ));
        return lines;
    }

    let hint = match (composer.input_state, shell_action_availability) {
        (
            ConversationInputState::DraftReady | ConversationInputState::ReadyToContinue,
            ShellActionAvailability::Pending,
        ) if composer.state.startup_submit_armed => {
            /*
            The startup-armed path means Enter was already accepted while startup
            was pending. Editing the buffer should cancel that queued send, so the
            hint names the cancellation behavior instead of repeating normal send
            guidance.
            */
            "Queued until startup is ready  |  editing cancels send"
        }
        (
            ConversationInputState::DraftReady | ConversationInputState::ReadyToContinue,
            ShellActionAvailability::Ready,
        ) => language.composer_send_action(),
        (ConversationInputState::DraftReady | ConversationInputState::ReadyToContinue, _) => {
            language.composer_startup_blocked_action()
        }
        (ConversationInputState::SubmittingTurn, _) => language.turn_starting_prompt_hint(true),
        (ConversationInputState::StreamingTurn, _) => language.composer_streaming_buffered_action(),
    };
    lines.push(composer_action_line(hint));
    lines
}
