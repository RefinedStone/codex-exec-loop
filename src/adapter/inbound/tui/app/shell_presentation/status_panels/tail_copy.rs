use ratatui::text::{Line, Span};

use super::super::capability_copy::{
    startup_initializing_status_line, startup_preparing_status_line,
    thread_history_loading_status_line,
};
use super::super::planning::build_planning_worker_panel_lines;
use super::super::planning::status_projection::build_planning_status_surface_projection;
use super::super::prompt_composer::{build_prompt_buffer_view, build_shell_command_palette_lines};
use super::super::{
    AkraTheme, ConversationComposerScreenModel, ConversationInputState,
    ConversationLiveTranscriptScreenModel, ConversationScreenModel, ConversationViewModel,
    INLINE_TAIL_NOTICE_DETAIL_LIMIT, INLINE_TAIL_PLANNING_DETAIL_LIMIT,
    INLINE_TAIL_STATUS_DETAIL_LIMIT, InlineShellCommand, InlineShellCommandAvailability,
    InlineShellCommandCapabilitySet, InlineShellCommandInput, Modifier, QueueMutationTailState,
    ShellActionAvailability, ShellConversationState, ShellOverlay, StartupState, TuiLanguage,
    build_working_line, compact_inline_detail,
};
use super::operator_ribbon::{build_operator_attention_line, build_operator_ribbon_line};
#[cfg(test)]
use super::operator_ribbon::{build_operator_diagnostic_lines, should_show_auto_follow_status};
use super::parallel_working_copy::build_parallel_slot_working_line;
use super::tail_shared::{
    OperatorNoticeKind, build_operator_notice, parallel_mode_alert_line, parallel_mode_summary_line,
};

use crate::adapter::inbound::tui::conversation_text::conversation_message_kind_label;

pub(super) const QUEUE_RECEIPT_UNDO_ACTION_LABEL: &str = "[ Undo queue ]";

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum InlineTailPriority {
    Pinned,
    Terminal,
    Warning,
    LiveActivity,
    RecentActivity,
    Identity,
    Detail,
}

#[derive(Clone)]
pub(super) struct InlineTailLine {
    pub(super) line: Line<'static>,
    pub(super) priority: InlineTailPriority,
}

impl InlineTailLine {
    fn new(priority: InlineTailPriority, line: Line<'static>) -> Self {
        Self { line, priority }
    }
}

/* The inline tail is the compact operational dashboard below the transcript. It
 * keeps high-priority state visible in this order: startup readiness, conversation
 * turn state, parallel/planning health, recent transcript context, then prompt
 * affordances. The order matters because this view is scanned repeatedly while a
 * turn is streaming or while startup checks are blocking submission.
 */
#[cfg(test)]
pub(super) fn build_inline_tail_lines_with_context(
    screen_model: &ConversationScreenModel<'_>,
    github_review_recent_changes_summary: Option<String>,
    notice_detail_limit: usize,
) -> Vec<Line<'static>> {
    build_inline_tail_content_with_context(
        screen_model,
        github_review_recent_changes_summary,
        notice_detail_limit,
        notice_detail_limit.saturating_add("notice: ".len()) as u16,
    )
    .into_iter()
    .map(|entry| entry.line)
    .collect()
}

pub(super) fn build_inline_tail_content_with_context(
    screen_model: &ConversationScreenModel<'_>,
    github_review_recent_changes_summary: Option<String>,
    notice_detail_limit: usize,
    content_width: u16,
) -> Vec<InlineTailLine> {
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
            INLINE_TAIL_PLANNING_DETAIL_LIMIT,
            INLINE_TAIL_NOTICE_DETAIL_LIMIT,
            false,
        )
    });
    let planning_worker_panel_lines = build_planning_worker_panel_lines(
        screen_model.planning_worker_shows_debug_details,
        &screen_model.planning_worker_panel_state,
        INLINE_TAIL_NOTICE_DETAIL_LIMIT,
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
                build_inline_startup_screen_lines_with_context(screen_model, content_width)
            } else {
                build_inline_startup_overlay_tail_lines_with_context(screen_model, content_width)
            }
            .into_iter()
            .map(|line| InlineTailLine::new(InlineTailPriority::Detail, line))
            .collect::<Vec<_>>();
        if screen_model.recent_session_status_requires_attention {
            lines.push(InlineTailLine::new(
                InlineTailPriority::Warning,
                Line::from(format!(
                    "session: {}",
                    screen_model.recent_session_status_label
                )),
            ));
        }
        lines.extend(
            build_inline_tail_prompt_lines_with_context(screen_model, content_width)
                .into_iter()
                .map(|line| InlineTailLine::new(InlineTailPriority::Pinned, line)),
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
            lines.push(InlineTailLine::new(
                InlineTailPriority::Identity,
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
            lines.push(InlineTailLine::new(
                InlineTailPriority::Warning,
                Line::from(format!(
                    "runtime: {runtime_status}{}  |  flow: terminal main buffer",
                    github_status.unwrap_or_default(),
                )),
            ));
            lines.push(InlineTailLine::new(InlineTailPriority::Detail, status_line));
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
            lines.push(InlineTailLine::new(
                InlineTailPriority::Identity,
                build_ready_status_ribbon_line(conversation, screen_model, content_width),
            ));
            if screen_model.recent_session_status_requires_attention {
                lines.push(InlineTailLine::new(
                    InlineTailPriority::Warning,
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
                        InlineTailPriority::Detail
                    } else {
                        InlineTailPriority::Warning
                    };
                lines.push(InlineTailLine::new(priority, status_detail_line));
            }
            if let Some(completion_line) = build_completion_alert_line(conversation) {
                lines.push(InlineTailLine::new(
                    InlineTailPriority::Pinned,
                    completion_line,
                ));
            }
            if let Some(queue_mutation_line) = build_queue_mutation_line(
                screen_model.queue_mutation_tail_state,
                screen_model.tui_language,
            ) {
                lines.push(InlineTailLine::new(
                    InlineTailPriority::Pinned,
                    queue_mutation_line,
                ));
            }
            if let Some(attention_line) =
                build_operator_attention_line(screen_model, Some(conversation), content_width)
            {
                lines.push(InlineTailLine::new(
                    InlineTailPriority::Warning,
                    attention_line,
                ));
            }
            if let Some(turn_options_summary) = screen_model.turn_options_summary.as_deref() {
                lines.push(InlineTailLine::new(
                    InlineTailPriority::Detail,
                    Line::from(format!("turn options: {turn_options_summary}")),
                ));
            }
            if let Some(parallel_summary_line) = parallel_mode_summary_line(screen_model) {
                lines.push(InlineTailLine::new(
                    InlineTailPriority::Detail,
                    Line::from(parallel_summary_line),
                ));
            }

            if let Some(parallel_mode_alert_line) = parallel_mode_alert_line(screen_model) {
                lines.push(InlineTailLine::new(
                    InlineTailPriority::Warning,
                    Line::from(parallel_mode_alert_line),
                ));
            }
            let working_detail_limit =
                INLINE_TAIL_STATUS_DETAIL_LIMIT.min(notice_detail_limit.saturating_sub(9));
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
                lines.push(InlineTailLine::new(
                    InlineTailPriority::LiveActivity,
                    working_line,
                ));
            }
            if let Some(planning_projection) = planning_status_projection.as_ref() {
                if let Some(planning_line) = planning_projection.summary_line.as_deref() {
                    let priority = if planning_projection.summary_is_warning {
                        InlineTailPriority::Warning
                    } else {
                        InlineTailPriority::Detail
                    };
                    lines.push(InlineTailLine::new(
                        priority,
                        Line::from(planning_line.to_string()),
                    ));
                }
                lines.extend(planning_projection.queue_framing_lines.iter().cloned().map(
                    |entry| {
                        let priority = if entry.has_blocker {
                            InlineTailPriority::Warning
                        } else {
                            InlineTailPriority::Detail
                        };
                        InlineTailLine::new(priority, entry.line)
                    },
                ));
                if let Some(planning_notice_line) = planning_projection.notice_line.as_deref() {
                    lines.push(InlineTailLine::new(
                        InlineTailPriority::Warning,
                        Line::from(planning_notice_line.to_string()),
                    ));
                }
            } else {
                lines.push(InlineTailLine::new(
                    InlineTailPriority::Warning,
                    Line::from(format!(
                        "planning: unavailable  |  startup: {}",
                        screen_model.shell_action_availability.status_text()
                    )),
                ));
            }
            if let Some(parallel_working_line) = build_parallel_slot_working_line(screen_model) {
                lines.push(InlineTailLine::new(
                    InlineTailPriority::LiveActivity,
                    parallel_working_line,
                ));
            }

            lines.extend(
                planning_worker_panel_lines
                    .into_iter()
                    .map(|line| InlineTailLine::new(InlineTailPriority::Detail, Line::from(line))),
            );
            let live_transcript = screen_model
                .live_transcript()
                .expect("ready conversation must retain its live transcript projection");
            lines.extend(
                build_recent_transcript_summary_lines(live_transcript)
                    .into_iter()
                    .map(|line| InlineTailLine::new(InlineTailPriority::Detail, line)),
            );
            if let Some(notice) = build_operator_notice(
                github_review_recent_changes_summary.as_deref(),
                conversation,
                INLINE_TAIL_NOTICE_DETAIL_LIMIT,
                notice_detail_limit,
            ) {
                lines.push(InlineTailLine::new(
                    operator_notice_priority(notice.kind),
                    Line::from(format!("notice: {}", notice.text)),
                ));
            }
        }
    }

    lines.extend(
        build_inline_tail_prompt_lines_with_context(screen_model, content_width)
            .into_iter()
            .map(|line| InlineTailLine::new(InlineTailPriority::Pinned, line)),
    );
    lines
}

fn operator_notice_priority(kind: OperatorNoticeKind) -> InlineTailPriority {
    match kind {
        OperatorNoticeKind::RequiredAction => InlineTailPriority::Pinned,
        OperatorNoticeKind::TerminalActivity => InlineTailPriority::Terminal,
        OperatorNoticeKind::Activity => InlineTailPriority::RecentActivity,
        OperatorNoticeKind::Detail => InlineTailPriority::Detail,
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
            compact_inline_detail(status, INLINE_TAIL_STATUS_DETAIL_LIMIT)
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

fn build_recent_transcript_summary_lines(
    live_transcript: &ConversationLiveTranscriptScreenModel<'_>,
) -> Vec<Line<'static>> {
    live_transcript
        .recent_tail_messages
        .into_iter()
        .flatten()
        .map(|message| {
            let label = conversation_message_kind_label(message.kind, message.phase.as_deref())
                .to_ascii_lowercase();
            let summary = message
                .text
                .lines()
                .find(|line| !line.trim().is_empty())
                .map(|line| compact_inline_detail(line, INLINE_TAIL_NOTICE_DETAIL_LIMIT))
                .unwrap_or_else(|| "(blank)".to_string());
            Line::from(format!("recent {label}: {summary}"))
        })
        .collect()
}

fn build_inline_startup_screen_lines_with_context(
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

fn build_inline_startup_overlay_tail_lines_with_context(
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

pub(super) fn build_inline_tail_prompt_lines_with_context(
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
    if screen_model
        .composer()
        .is_some_and(|composer| composer.viewport_transcript_handoff_pending)
        && (screen_model.shell_overlay != ShellOverlay::Hidden || screen_model.dialog_visible())
    {
        return vec![
            Line::styled(
                screen_model.tui_language.composer_dialog_hold_status(),
                AkraTheme::subtle(),
            ),
            composer_action_line(screen_model.tui_language.composer_dialog_resume_action()),
        ];
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
            build_inline_ready_prompt_lines(
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

fn build_inline_ready_prompt_lines(
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

#[cfg(test)]
mod coverage_tests {
    use super::*;
    use crate::adapter::inbound::tui::app::conversation_model::RecordedAutoFollowActivity;
    use crate::adapter::inbound::tui::app::language::TuiLanguage;
    use crate::adapter::inbound::tui::app::queue_overlay_ui::QueueMutationKind;
    use crate::adapter::inbound::tui::app::test_helpers::test_native_tui_app;
    use crate::adapter::inbound::tui::app::{
        ConversationState, InlineHistoryRenderMode, NativeTuiApp,
    };
    use crate::core::app::{
        ActiveTurnPhase, ActiveTurnSnapshot, AutoFollowPhase, CorePromptOrigin,
        PostTurnAuthoritySnapshot, PostTurnEvaluationCorrelation, QueueMutationCorrelation,
        QueueMutationIntent, StartupReadySnapshot, TurnSubmissionCorrelation,
    };
    use crate::domain::conversation::{ConversationMessage, ConversationMessageKind};
    use crate::domain::planning::{
        PlanningQueueMutationKind, PlanningQueueMutationReceipt, PlanningQueueMutationReceiptEntry,
        TaskStatus,
    };
    use crate::domain::startup_diagnostics::StartupDiagnostics;
    use crate::domain::terminal_bridge_attachment::TerminalBridgeAttachmentProfile;
    use std::time::Instant;

    fn set_input_state(
        conversation: &mut ConversationViewModel,
        input_state: ConversationInputState,
    ) {
        let mut snapshot = conversation.runtime_snapshot().clone();
        snapshot.active_turn = match input_state {
            ConversationInputState::DraftReady => {
                conversation.thread_id.clear();
                None
            }
            ConversationInputState::ReadyToContinue => {
                if conversation.thread_id.is_empty() {
                    conversation.thread_id = "thread-fixture".to_string();
                }
                None
            }
            ConversationInputState::SubmittingTurn | ConversationInputState::StreamingTurn => {
                Some(ActiveTurnSnapshot {
                    correlation: TurnSubmissionCorrelation::new(1),
                    phase: if input_state == ConversationInputState::SubmittingTurn {
                        ActiveTurnPhase::Submitting
                    } else {
                        ActiveTurnPhase::Running
                    },
                    workspace_directory: conversation.cwd.clone(),
                    turn_id: (input_state == ConversationInputState::StreamingTurn)
                        .then(|| "turn-fixture".to_string()),
                    prompt_origin: CorePromptOrigin::Manual,
                    started_at: Instant::now(),
                })
            }
        };
        conversation.apply_runtime_snapshot(snapshot);
    }

    fn set_running_turn(conversation: &mut ConversationViewModel, turn_id: &str) {
        let mut snapshot = conversation.runtime_snapshot().clone();
        snapshot.active_turn = Some(ActiveTurnSnapshot {
            correlation: TurnSubmissionCorrelation::new(1),
            phase: ActiveTurnPhase::Running,
            workspace_directory: conversation.cwd.clone(),
            turn_id: Some(turn_id.to_string()),
            prompt_origin: CorePromptOrigin::Manual,
            started_at: Instant::now(),
        });
        conversation.apply_runtime_snapshot(snapshot);
        conversation.record_turn_started(turn_id.to_string());
    }

    fn begin_post_turn_evaluation(conversation: &mut ConversationViewModel, turn_id: &str) {
        let workspace_directory = conversation.cwd.clone();
        let mut snapshot = conversation.runtime_snapshot().clone();
        snapshot.active_turn = None;
        snapshot.post_turn = PostTurnAuthoritySnapshot::Evaluating {
            correlation: PostTurnEvaluationCorrelation::new(
                1,
                conversation.thread_id.clone(),
                turn_id,
                workspace_directory.clone(),
                workspace_directory,
            ),
            started_at: Instant::now(),
        };
        conversation.apply_runtime_snapshot(snapshot);
        conversation.begin_post_turn_settlement(turn_id);
    }

    fn ready_conversation(app: &NativeTuiApp) -> &ConversationViewModel {
        let ConversationState::Ready(conversation) = &app.conversation.lifecycle.conversation_state
        else {
            panic!("expected ready conversation");
        };
        conversation
    }

    fn ready_conversation_mut(app: &mut NativeTuiApp) -> &mut ConversationViewModel {
        let ConversationState::Ready(conversation) =
            &mut app.conversation.lifecycle.conversation_state
        else {
            panic!("expected ready conversation");
        };
        conversation
    }

    fn rendered(lines: Vec<Line<'static>>) -> String {
        lines
            .into_iter()
            .map(|line| line.to_string())
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn rendered_ready_prompt(
        conversation: &ConversationViewModel,
        availability: ShellActionAvailability,
        language: TuiLanguage,
    ) -> String {
        rendered(build_inline_ready_prompt_lines(
            &ConversationComposerScreenModel::from_conversation(conversation),
            &InlineShellCommandCapabilitySet::default(),
            availability,
            language,
            120,
        ))
    }

    fn render_tail(app: &NativeTuiApp, recent_changes: Option<&str>) -> String {
        let mut screen_model = ConversationScreenModel::from_app(app);
        screen_model.github_review_recent_changes_summary = recent_changes.map(str::to_string);
        rendered(build_inline_tail_lines_with_context(
            &screen_model,
            screen_model.github_review_recent_changes_summary.clone(),
            INLINE_TAIL_NOTICE_DETAIL_LIMIT,
        ))
    }

    fn render_recent_transcript_tail(app: &NativeTuiApp) -> String {
        let screen_model = ConversationScreenModel::from_app(app);
        let live_transcript = screen_model
            .live_transcript()
            .expect("ready conversation must retain its live transcript projection");
        rendered(build_recent_transcript_summary_lines(live_transcript))
    }

    fn startup_ready_snapshot(can_continue: bool) -> Box<StartupReadySnapshot> {
        let account_detail = if can_continue {
            "ok"
        } else {
            "missing account"
        };
        Box::new(StartupReadySnapshot::from_diagnostics(StartupDiagnostics {
            cwd: "/tmp/root".to_string(),
            codex_binary_ok: true,
            codex_binary_detail: "codex".to_string(),
            workspace_ok: true,
            workspace_path: "/tmp/root".to_string(),
            workspace_detail: "workspace found".to_string(),
            attachment_profile: TerminalBridgeAttachmentProfile::codex_app_server(),
            initialize_ok: true,
            initialize_detail: "app-server initialize ok".to_string(),
            account_ok: can_continue,
            account_detail: account_detail.to_string(),
            warnings: vec!["first warning should stay visible".to_string()],
            schema_snapshot: "schema".to_string(),
        }))
    }

    fn context_for<'a>(
        startup_state: &'a StartupState,
        shell_action_availability: ShellActionAvailability,
        conversation_state: ShellConversationState<'a>,
    ) -> ConversationScreenModel<'a> {
        ConversationScreenModel::from_test_parts(
            startup_state,
            shell_action_availability,
            conversation_state,
        )
    }

    #[test]
    fn composer_projection_exists_only_for_ready_conversations() {
        let startup_state = StartupState::Idle;
        let conversation = ConversationViewModel::new_draft("/tmp/root".to_string());
        let failed_message = "startup failed".to_string();

        for state in [
            ShellConversationState::Loading,
            ShellConversationState::Failed(&failed_message),
        ] {
            let screen_model = context_for(&startup_state, ShellActionAvailability::Pending, state);
            assert!(screen_model.composer().is_none());
        }

        let screen_model = context_for(
            &startup_state,
            ShellActionAvailability::Ready,
            ShellConversationState::Ready(&conversation),
        );
        let composer = screen_model
            .composer()
            .expect("ready conversation should project composer state");
        assert!(std::ptr::eq(composer.state, &conversation.composer));
    }

    #[test]
    fn runtime_status_projection_exists_only_for_ready_conversations() {
        let startup_state = StartupState::Idle;
        let conversation = ConversationViewModel::new_draft("/tmp/root".to_string());
        let failed_message = "startup failed".to_string();

        for state in [
            ShellConversationState::Loading,
            ShellConversationState::Failed(&failed_message),
        ] {
            let screen_model = context_for(&startup_state, ShellActionAvailability::Pending, state);
            assert!(screen_model.runtime_status().is_none());
        }

        let screen_model = context_for(
            &startup_state,
            ShellActionAvailability::Ready,
            ShellConversationState::Ready(&conversation),
        );
        let runtime_status = screen_model
            .runtime_status()
            .expect("ready conversation should project runtime status");
        assert_eq!(
            runtime_status.input_state,
            ConversationInputState::DraftReady
        );
        assert!(runtime_status.working_started_at.is_none());
    }

    #[test]
    fn shell_loading_and_failed_tails_keep_prompt_copy_without_ready_conversation() {
        let mut app = test_native_tui_app();

        app.conversation.lifecycle.conversation_state = ConversationState::Loading;
        app.shell.chrome.startup_state = StartupState::Loading;
        let loading = render_tail(&app, None);
        assert!(loading.contains("thread: loading"));
        assert!(loading.contains("runtime: loading thread history"));
        assert_eq!(
            loading.lines().nth(2),
            Some(thread_history_loading_status_line())
        );
        assert!(loading.contains("Preparing the prompt"));
        assert!(loading.contains("Wait for shell readiness"));

        app.conversation.lifecycle.conversation_state =
            ConversationState::Failed("session catalog unavailable".to_string());
        let failed = render_tail(&app, None);
        assert!(failed.contains("thread: unavailable"));
        assert!(failed.contains("runtime: unavailable"));
        assert_eq!(
            failed.lines().nth(2),
            Some("status: session catalog unavailable")
        );
        assert!(failed.contains("Prompt unavailable"));
        assert!(failed.contains("Ctrl+D diagnostics"));
    }

    #[test]
    fn startup_tail_covers_masthead_overlay_state_and_starter_variants() {
        let mut app = test_native_tui_app();

        app.shell.chrome.startup_state = StartupState::Idle;
        let idle = render_tail(&app, None);
        assert!(idle.contains("Akra / root"));
        assert!(idle.contains("preparing startup checks"));
        assert!(idle.contains("Describe a task"));
        assert!(!idle.contains("workspace: /tmp/root"));

        app.shell.chrome.startup_state = StartupState::Loading;
        let loading = render_tail(&app, None);
        assert!(loading.contains("initializing codex shell"));
        assert!(!loading.contains("opening codex app-server"));

        app.shell.chrome.startup_state = StartupState::Ready(startup_ready_snapshot(true));
        let ready = render_tail(&app, None);
        assert!(ready.contains("Akra / root"));
        assert!(ready.contains("DEGRADED"));
        assert!(ready.contains("Ctrl+D details"));
        assert!(!ready.contains("first warning should stay visible"));
        assert!(ready.contains("Type a task"));
        assert!(!ready.contains("diagnostics:"));
        assert!(!ready.contains("examples:"));
        assert!(!ready.contains("████"));

        app.shell.chrome.startup_state = StartupState::Failed("codex missing".to_string());
        let failed = render_tail(&app, None);
        assert!(failed.contains("codex missing"));

        ready_conversation_mut(&mut app).composer.input_buffer =
            "queued startup prompt".to_string();
        let overlay = render_tail(&app, None);
        assert!(overlay.contains("Akra"));
        assert!(!overlay.contains("████"));

        app.shell.chrome.startup_state = StartupState::Ready(startup_ready_snapshot(true));
        let context = ConversationScreenModel::from_app(&app);
        let compact = rendered(build_inline_startup_screen_lines_with_context(&context, 80));
        assert!(compact.contains("Akra / root"));
        assert!(!compact.contains("draft: opening prompt buffered below"));

        app.conversation.lifecycle.conversation_state = ConversationState::Loading;
    }

    #[test]
    fn ready_status_helpers_cover_auto_follow_completion_and_warnings() {
        let startup_state = StartupState::Loading;
        let mut conversation = ConversationViewModel::new_draft("/tmp/root".to_string());
        conversation.thread_id = "thread-1".to_string();
        conversation.status_text =
            "a very long status line that should be compacted inside the inline tail".to_string();

        assert!(!should_show_auto_follow_status(&conversation));
        let mut snapshot = conversation.runtime_snapshot().clone();
        snapshot.auto_follow.completed_auto_turns = 2;
        conversation.apply_runtime_snapshot(snapshot);
        assert!(should_show_auto_follow_status(&conversation));
        let mut snapshot = conversation.runtime_snapshot().clone();
        snapshot.auto_follow.completed_auto_turns = 0;
        snapshot.auto_follow.continuation_paused = true;
        conversation.apply_runtime_snapshot(snapshot);
        assert!(should_show_auto_follow_status(&conversation));
        let mut snapshot = conversation.runtime_snapshot().clone();
        snapshot.auto_follow.max_auto_turns = 5;
        snapshot.auto_follow.phase = AutoFollowPhase::Queued {
            started_at: Instant::now(),
            turn_index: 3,
        };
        conversation.apply_runtime_snapshot(snapshot);

        let context = context_for(
            &startup_state,
            ShellActionAvailability::Blocked,
            ShellConversationState::Ready(&conversation),
        );
        let ribbon = build_ready_status_ribbon_line(&conversation, &context, 160).to_string();
        assert!(ribbon.contains("auto:"));
        assert!(ribbon.contains("done:"));
        let detail = build_ready_status_detail_line(&conversation, &context)
            .expect("non-benign status detail")
            .to_string();
        assert!(detail.contains("startup: startup diagnostics need attention"));
        assert!(detail.contains("gh: polling"));

        let mut running_draft = ConversationViewModel::new_draft("/tmp/root".to_string());
        set_running_turn(&mut running_draft, "turn-1");
        running_draft.status_text = "new thread draft".to_string();
        let running_context = context_for(
            &startup_state,
            ShellActionAvailability::Ready,
            ShellConversationState::Ready(&running_draft),
        );
        let running_detail = build_ready_status_detail_line(&running_draft, &running_context)
            .expect("github polling remains visible")
            .to_string();
        assert!(!running_detail.contains("new thread draft"));

        assert!(build_completion_alert_line(&conversation).is_none());
        conversation.last_auto_follow_activity = Some(RecordedAutoFollowActivity {
            summary: "complete: planning queue drained".to_string(),
            detail: "all done".to_string(),
        });
        assert!(
            build_completion_alert_line(&conversation)
                .expect("completion line")
                .to_string()
                .contains("all planning tasks complete")
        );
    }

    #[test]
    fn operator_attention_aggregates_causes_and_keeps_raw_payload_in_diagnostics() {
        let mut ready = startup_ready_snapshot(true);
        ready.warnings.clear();
        let startup_state = StartupState::Ready(ready);
        let mut conversation = ConversationViewModel::new_draft("/tmp/root".to_string());
        let warning = "app-server sent notification `remoteControl:secret-payload`".to_string();
        let notice = "parallel cleanup remains unsettled".to_string();
        conversation.base_warnings.push(warning.clone());
        conversation.runtime_notices.push(notice.clone());
        let mut context = context_for(
            &startup_state,
            ShellActionAvailability::Ready,
            ShellConversationState::Ready(&conversation),
        );
        context.global_runtime_notices.push(notice.clone());

        for width in [80, 120, 160] {
            let line = build_operator_attention_line(&context, Some(&conversation), width)
                .expect("warning and notice should project an attention row");
            let text = line.to_string();
            assert!(text.contains("warning 1"), "{width}: {text}");
            assert!(text.contains("runtime notice 1"), "{width}: {text}");
            assert!(text.contains("Ctrl+D details"), "{width}: {text}");
            assert!(!text.contains("secret-payload"), "{width}: {text}");
            assert!(line.width() <= usize::from(width), "{width}: {text}");
        }

        let diagnostics = build_operator_diagnostic_lines(&context)
            .into_iter()
            .map(|line| line.to_string())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(diagnostics.contains(&warning));
        assert_eq!(diagnostics.matches(&notice).count(), 1);
    }

    #[test]
    fn recent_transcript_projection_prioritizes_human_copy_and_falls_back_to_status() {
        let mut app = test_native_tui_app();
        app.shell.inline_history_render_mode = InlineHistoryRenderMode::HostScrollback;
        ready_conversation_mut(&mut app).messages = vec![
            ConversationMessage::new(
                ConversationMessageKind::Agent,
                "oldest agent",
                Some("final_answer".to_string()),
                None,
            ),
            ConversationMessage::new(ConversationMessageKind::Tool, "tool noise", None, None),
            ConversationMessage::new(ConversationMessageKind::User, "first user", None, None),
            ConversationMessage::new(ConversationMessageKind::Status, "status noise", None, None),
            ConversationMessage::new(
                ConversationMessageKind::Agent,
                "\nsecond agent line",
                Some("final_answer".to_string()),
                None,
            ),
        ];
        assert!(render_recent_transcript_tail(&app).is_empty());

        app.shell.inline_history_render_mode = InlineHistoryRenderMode::ViewportReplay;
        let transcript = render_recent_transcript_tail(&app);
        assert_eq!(
            transcript,
            "recent you: first user\nrecent codex: second agent line"
        );

        ready_conversation_mut(&mut app).messages = vec![
            ConversationMessage::new(ConversationMessageKind::Tool, "tool noise", None, None),
            ConversationMessage::new(ConversationMessageKind::Status, "   ", None, None),
        ];
        let fallback = render_recent_transcript_tail(&app);
        assert!(fallback.contains("recent status: (blank)"));
    }

    #[test]
    fn ready_prompt_copy_covers_empty_buffer_commands_and_buffered_states() {
        for (state, availability, expected) in [
            (
                ConversationInputState::DraftReady,
                ShellActionAvailability::Pending,
                "submission waits for startup",
            ),
            (
                ConversationInputState::DraftReady,
                ShellActionAvailability::Blocked,
                "Ctrl+D diagnostics",
            ),
            (
                ConversationInputState::DraftReady,
                ShellActionAvailability::Ready,
                "Type a task",
            ),
            (
                ConversationInputState::ReadyToContinue,
                ShellActionAvailability::Ready,
                "Type a task",
            ),
            (
                ConversationInputState::SubmittingTurn,
                ShellActionAvailability::Ready,
                "wait for turn start",
            ),
            (
                ConversationInputState::StreamingTurn,
                ShellActionAvailability::Ready,
                "Type a follow-up",
            ),
        ] {
            let mut conversation = ConversationViewModel::new_draft("/tmp/root".to_string());
            set_input_state(&mut conversation, state);
            let prompt = rendered_ready_prompt(&conversation, availability, TuiLanguage::English);
            assert!(
                prompt.contains(expected),
                "expected `{expected}` in `{prompt}`"
            );
        }

        let mut palette = ConversationViewModel::new_draft("/tmp/root".to_string());
        palette.composer.input_buffer = ":".to_string();
        palette.composer.sync_inline_shell_command_palette();
        palette
            .composer
            .move_inline_shell_command_palette_selection(3);
        let palette_prompt = rendered_ready_prompt(
            &palette,
            ShellActionAvailability::Ready,
            TuiLanguage::English,
        );
        assert!(palette_prompt.contains("palette 4/19"));
        assert!(palette_prompt.contains("↑/↓ or Tab select"));
        assert!(palette_prompt.contains("Enter run"));
        assert!(palette_prompt.contains("Esc close"));
        assert!(palette_prompt.contains(":parallel"));
        assert!(palette_prompt.contains("READY"));
        assert!(palette_prompt.contains("inspect retained activity"));

        let korean_palette_prompt = rendered_ready_prompt(
            &palette,
            ShellActionAvailability::Ready,
            TuiLanguage::Korean,
        );
        assert!(
            korean_palette_prompt
                .contains(&TuiLanguage::Korean.inline_command_palette_header(4, 19))
        );
        assert!(korean_palette_prompt.contains("↑/↓ 또는 Tab 선택"));
        assert!(korean_palette_prompt.contains("Enter 실행"));
        assert!(korean_palette_prompt.contains("Esc 닫기"));
        assert!(korean_palette_prompt.contains(":activity  READY"));
        assert!(korean_palette_prompt.contains("보존된 활동 보기"));

        let mut korean_command = ConversationViewModel::new_draft("/tmp/root".to_string());
        korean_command.composer.input_buffer = ":reset queue".to_string();
        let korean_command_prompt = rendered_ready_prompt(
            &korean_command,
            ShellActionAvailability::Ready,
            TuiLanguage::Korean,
        );
        let korean_hint = InlineShellCommandInput::parse(":reset queue")
            .expect("reset command should parse")
            .localized_buffered_hint(TuiLanguage::Korean);
        assert!(korean_command_prompt.contains(&korean_hint));
        assert!(!korean_command_prompt.contains("Press Enter"));

        let mut command = ConversationViewModel::new_draft("/tmp/root".to_string());
        command.composer.input_buffer = ":reset queue".to_string();
        let command_prompt = rendered_ready_prompt(
            &command,
            ShellActionAvailability::Ready,
            TuiLanguage::English,
        );
        assert!(command_prompt.contains("Press Enter to reset queue-side planning state."));

        let mut busy = ConversationViewModel::new_draft("/tmp/root".to_string());
        busy.composer.input_buffer = "next prompt".to_string();
        let mut snapshot = busy.runtime_snapshot().clone();
        snapshot.auto_follow.phase = AutoFollowPhase::Queued {
            turn_index: 1,
            started_at: Instant::now(),
        };
        busy.apply_runtime_snapshot(snapshot);
        let busy_prompt =
            rendered_ready_prompt(&busy, ShellActionAvailability::Ready, TuiLanguage::English);
        assert!(busy_prompt.contains("auto-follow busy"));

        busy.composer.clear_input_buffer();
        let empty_busy_prompt =
            rendered_ready_prompt(&busy, ShellActionAvailability::Ready, TuiLanguage::English);
        assert!(empty_busy_prompt.contains("Type now  |  Enter when idle"));

        let mut armed = ConversationViewModel::new_draft("/tmp/root".to_string());
        armed.composer.input_buffer = "queued".to_string();
        armed.composer.startup_submit_armed = true;
        let armed_prompt = rendered_ready_prompt(
            &armed,
            ShellActionAvailability::Pending,
            TuiLanguage::English,
        );
        assert!(armed_prompt.contains("editing cancels send"));

        for (state, availability, expected) in [
            (
                ConversationInputState::DraftReady,
                ShellActionAvailability::Ready,
                "Enter send",
            ),
            (
                ConversationInputState::ReadyToContinue,
                ShellActionAvailability::Blocked,
                "Ctrl+D diagnostics",
            ),
            (
                ConversationInputState::SubmittingTurn,
                ShellActionAvailability::Ready,
                "queues after start",
            ),
            (
                ConversationInputState::StreamingTurn,
                ShellActionAvailability::Ready,
                "Tab steer",
            ),
        ] {
            let mut conversation = ConversationViewModel::new_draft("/tmp/root".to_string());
            conversation.composer.input_buffer = "buffered".to_string();
            set_input_state(&mut conversation, state);
            let prompt = rendered_ready_prompt(&conversation, availability, TuiLanguage::English);
            assert!(
                prompt.contains(expected),
                "expected `{expected}` in `{prompt}`"
            );
        }

        assert!(!parallel_loading_prompt_indicator_frame(0).is_empty());
    }

    #[test]
    fn planning_settlement_truth_overrides_idle_and_enter_send_copy() {
        let mut conversation = ConversationViewModel::new_draft("/tmp/root".to_string());
        begin_post_turn_evaluation(&mut conversation, "turn-1");
        let mut snapshot = conversation.runtime_snapshot().clone();
        snapshot.auto_follow.max_auto_turns = 0;
        snapshot.auto_follow.phase = AutoFollowPhase::Queued {
            turn_index: 1,
            started_at: Instant::now(),
        };
        conversation.apply_runtime_snapshot(snapshot);

        assert!(!conversation.can_accept_manual_prompt());
        let startup_state = StartupState::Idle;
        let screen_model = context_for(
            &startup_state,
            ShellActionAvailability::Ready,
            ShellConversationState::Ready(&conversation),
        );
        assert!(
            build_working_line(
                screen_model
                    .runtime_status()
                    .expect("ready conversation should retain runtime status"),
                40,
                80,
                Instant::now(),
            )
            .is_some_and(|line| line.to_string().contains("settling planning queue"))
        );
        let empty_prompt = rendered_ready_prompt(
            &conversation,
            ShellActionAvailability::Ready,
            TuiLanguage::English,
        );
        assert!(empty_prompt.contains("Enter when settled"));
        assert!(!empty_prompt.contains("Enter send"));
        assert!(!empty_prompt.contains("Enter when ready"));

        conversation.composer.input_buffer = "next request".to_string();
        let buffered_prompt = rendered_ready_prompt(
            &conversation,
            ShellActionAvailability::Ready,
            TuiLanguage::English,
        );
        assert!(buffered_prompt.contains("Enter when settled"));
        assert!(!buffered_prompt.contains("Enter when ready"));
        assert!(buffered_prompt.contains("Draft ready"));
    }

    #[test]
    fn settlement_tail_shows_one_phase_truth_without_auto_or_idle_noise() {
        let mut app = test_native_tui_app();
        app.shell.chrome.startup_state = StartupState::Ready(startup_ready_snapshot(true));
        let conversation = ready_conversation_mut(&mut app);
        conversation.thread_id = "thread-settlement".to_string();
        begin_post_turn_evaluation(conversation, "turn-1");
        let mut snapshot = conversation.runtime_snapshot().clone();
        snapshot.auto_follow.max_auto_turns = 0;
        snapshot.auto_follow.completed_auto_turns = 1;
        conversation.apply_runtime_snapshot(snapshot);
        conversation
            .runtime_notices
            .push("bridge attached".to_string());

        let tail = render_tail(&app, None);

        assert_eq!(tail.matches("settling planning queue").count(), 1);
        assert!(!tail.contains("auto:"));
        assert!(!tail.contains("done:"));
        assert!(!tail.contains("status: turn completed"));
        assert!(!tail.contains("warn: none"));
        assert!(!tail.contains("Enter send"));
        assert!(!tail.contains("Enter when ready"));
    }

    #[test]
    fn pending_queue_mutation_replaces_undo_action_with_non_clickable_correlation_status() {
        let mut app = test_native_tui_app();
        app.shell.chrome.startup_state = StartupState::Ready(startup_ready_snapshot(true));
        let receipt = PlanningQueueMutationReceipt {
            completed_turn_id: "turn-queue-op".to_string(),
            planning_revision: 9,
            entries: vec![PlanningQueueMutationReceiptEntry {
                task_id: "queued-task".to_string(),
                task_title: "Undo after authority acknowledgement".to_string(),
                mutation_kind: PlanningQueueMutationKind::Created,
                before_status: None,
                after_status: TaskStatus::Ready,
                after_updated_at: "2026-07-16T00:00:00Z".to_string(),
                unchanged_since_mutation: true,
            }],
        };
        let conversation = ready_conversation_mut(&mut app);
        conversation.thread_id = "thread-queue-op".to_string();
        conversation.latest_queue_mutation_receipt = Some(receipt.clone());
        let context = app.current_queue_mutation_context();
        app.planning
            .queue_mutation_ui_state
            .record_started(QueueMutationCorrelation::new(
                1,
                QueueMutationIntent {
                    workspace_directory: context.workspace_directory,
                    active_thread_id: context.active_thread_id,
                    kind: QueueMutationKind::UndoLatestRegistration,
                    expected_planning_revision: 9,
                    targets: Vec::new(),
                    receipt_at_start: Some(receipt),
                },
            ));

        let screen_model = ConversationScreenModel::from_app(&app);
        let tail_view = super::super::live_status_layout::build_inline_tail_view(&screen_model, 96);
        let tail = rendered(tail_view.lines);

        assert!(
            tail.contains("queue: op-1  |  authority acknowledgement pending"),
            "{tail}"
        );
        assert!(!tail.contains(QUEUE_RECEIPT_UNDO_ACTION_LABEL), "{tail}");
        assert!(tail_view.queue_receipt_undo_hit_area.is_none());

        app.shell.tui_language = TuiLanguage::Korean;
        let korean_tail = render_tail(&app, None);
        assert!(korean_tail.contains("큐: op-1  |  권한 확인 대기 중"));
    }

    #[test]
    fn required_queue_authority_refresh_uses_localized_non_actionable_tail_copy() {
        let mut app = test_native_tui_app();
        app.shell.chrome.startup_state = StartupState::Ready(startup_ready_snapshot(true));
        app.shell.tui_language = TuiLanguage::Korean;
        ready_conversation_mut(&mut app).thread_id = "thread-refresh-required".to_string();
        app.planning
            .queue_mutation_ui_state
            .require_authority_refresh();

        let screen_model = ConversationScreenModel::from_app(&app);
        let tail_view = super::super::live_status_layout::build_inline_tail_view(&screen_model, 96);
        let tail = rendered(tail_view.lines);

        assert!(
            tail.contains("큐: 새로고침 필요  |  다시 시도하기 전에 :queue 열기"),
            "{tail}"
        );
        assert!(!tail.contains(QUEUE_RECEIPT_UNDO_ACTION_LABEL));
        assert!(tail_view.queue_receipt_undo_hit_area.is_none());
    }

    #[test]
    fn full_ready_tail_includes_runtime_warnings_planning_notice_and_operator_notice() {
        let mut app = test_native_tui_app();
        app.shell.chrome.startup_state = StartupState::Ready(startup_ready_snapshot(true));
        let conversation = ready_conversation_mut(&mut app);
        conversation.thread_id = "thread-1".to_string();
        conversation.base_warnings.push("warning one".to_string());
        conversation.warnings.push("warning one".to_string());
        conversation.runtime_notices.push("runtime one".to_string());
        conversation.composer.input_buffer = "buffered".to_string();

        let tail = render_tail(&app, Some("review changed"));

        assert!(tail.contains("DEGRADED"));
        assert!(tail.contains("w2"));
        assert!(tail.contains("n1"));
        assert!(tail.contains("Ctrl+D details"));
        assert!(!tail.contains("runtime one"));
        assert!(!tail.contains("warning one"));
        assert!(tail.contains("notice:"));
        assert!(tail.contains("review changed"));
        assert!(tail.contains("Enter send"));

        assert_eq!(ready_conversation(&app).thread_id, "thread-1");
    }
}
