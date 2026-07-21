use ratatui::text::{Line, Span};

use super::super::capability_copy::{
    startup_attachment_summary_line, startup_diagnostics_summary_line,
    startup_initializing_status_line, startup_preparing_status_line,
    thread_history_loading_status_line,
};
use super::super::planning::build_planning_worker_panel_lines;
use super::super::planning::status_projection::build_planning_status_surface_projection;
use super::super::prompt_composer::{build_prompt_buffer_view, build_shell_command_palette_lines};
use super::super::{
    AkraTheme, ConversationComposerScreenModel, ConversationInputState, ConversationScreenModel,
    ConversationViewModel, INLINE_TAIL_AUTO_FOLLOW_DETAIL_LIMIT, INLINE_TAIL_NOTICE_DETAIL_LIMIT,
    INLINE_TAIL_PLANNING_DETAIL_LIMIT, INLINE_TAIL_RUNTIME_NOTICE_DETAIL_LIMIT,
    INLINE_TAIL_STATUS_DETAIL_LIMIT, INLINE_TAIL_WARNING_DETAIL_LIMIT, InlineHistoryRenderMode,
    InlineShellCommandInput, Modifier, QueueMutationTailState, ShellActionAvailability,
    ShellConversationState, ShellOverlay, StartupState, TuiLanguage, build_working_line,
    compact_inline_detail,
};
use super::parallel_working_copy::build_parallel_slot_working_line;
use super::tail_shared::{
    OperatorNoticeKind, build_operator_notice, compact_auto_follow_status_summary,
    compact_inline_summary_label, inline_thread_label, parallel_mode_alert_line,
    parallel_mode_summary_line,
};

use crate::adapter::inbound::tui::conversation_text::conversation_message_kind_label;
use crate::domain::conversation::{ConversationMessage, ConversationMessageKind};

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
    )
    .into_iter()
    .map(|entry| entry.line)
    .collect()
}

pub(super) fn build_inline_tail_content_with_context(
    screen_model: &ConversationScreenModel<'_>,
    github_review_recent_changes_summary: Option<String>,
    notice_detail_limit: usize,
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
                build_inline_startup_screen_lines_with_context(screen_model)
            } else {
                build_inline_startup_overlay_tail_lines_with_context(screen_model)
            }
            .into_iter()
            .map(|line| InlineTailLine::new(InlineTailPriority::Detail, line))
            .collect::<Vec<_>>();
        lines.extend(
            build_inline_tail_prompt_lines_with_context(screen_model)
                .into_iter()
                .map(|line| InlineTailLine::new(InlineTailPriority::Pinned, line)),
        );
        return lines;
    }
    let mut lines = Vec::new();
    match screen_model.conversation_state {
        ShellConversationState::Loading => {
            /*
            Loading and failed states still render a full tail because the inline
            terminal layout needs stable prompt/status rows even before a thread
            snapshot exists. These branches avoid conversation-only helpers.
            */
            lines.push(InlineTailLine::new(
                InlineTailPriority::Identity,
                Line::from(format!(
                    "Akra  |  thread: loading  |  startup: {}  |  sessions: {}",
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
                    "runtime: loading thread history{}  |  flow: terminal main buffer",
                    github_status.unwrap_or_default(),
                )),
            ));
            lines.push(InlineTailLine::new(
                InlineTailPriority::Detail,
                Line::from(format!("status: {}", thread_history_loading_status_line())),
            ));
        }
        ShellConversationState::Failed(message) => {
            lines.push(InlineTailLine::new(
                InlineTailPriority::Identity,
                Line::from(format!(
                    "Akra  |  thread: unavailable  |  startup: {}  |  sessions: {}",
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
                    "runtime: unavailable{}  |  flow: terminal main buffer",
                    github_status.unwrap_or_default(),
                )),
            ));
            lines.push(InlineTailLine::new(
                InlineTailPriority::Detail,
                Line::from(format!("status: {message}")),
            ));
        }
        ShellConversationState::Ready(conversation) => {
            /*
            Ready-state ordering is intentionally dense: identity and turn status
            first, then operator health signals, parallel/planning summaries,
            worker detail, recent transcript context, and finally notices. This
            mirrors how operators scan the tail while a turn is active.
            */
            let warning_summary = compact_inline_summary_label(
                &conversation.warning_summary(INLINE_TAIL_WARNING_DETAIL_LIMIT),
            );
            let runtime_notice_summary = conversation
                .runtime_notice_summary(INLINE_TAIL_RUNTIME_NOTICE_DETAIL_LIMIT)
                .map(|summary| compact_inline_summary_label(&summary));

            lines.push(InlineTailLine::new(
                InlineTailPriority::Identity,
                build_ready_status_ribbon_line(conversation),
            ));
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
            if let Some(runtime_notice_summary) = runtime_notice_summary {
                let mut runtime_line = format!("runtime: {runtime_notice_summary}");
                if warning_summary_has_signal(&warning_summary) {
                    runtime_line.push_str(&format!("  |  {warning_summary}"));
                }
                lines.push(InlineTailLine::new(
                    InlineTailPriority::Warning,
                    Line::from(runtime_line),
                ));
            } else if warning_summary_has_signal(&warning_summary) {
                lines.push(InlineTailLine::new(
                    InlineTailPriority::Warning,
                    Line::from(warning_summary),
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
            if let Some(working_line) =
                build_working_line(conversation, working_detail_limit, screen_model.rendered_at)
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
            let renders_viewport_handoff = matches!(
                screen_model.inline_history_render_mode,
                InlineHistoryRenderMode::ViewportReplay
            ) && conversation
                .has_pending_viewport_transcript_handoff();
            if !renders_viewport_handoff {
                lines.extend(
                    build_recent_transcript_summary_lines(
                        screen_model.inline_history_render_mode,
                        conversation,
                    )
                    .into_iter()
                    .map(|line| InlineTailLine::new(InlineTailPriority::Detail, line)),
                );
            }
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
        build_inline_tail_prompt_lines_with_context(screen_model)
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
fn build_ready_status_ribbon_line(conversation: &ConversationViewModel) -> Line<'static> {
    /*
    The ribbon anchors thread identity. Working state and input actions have
    dedicated rows below it, so repeating them here would consume compact rows
    without adding operator information. Auto-follow details are only added
    while an automatic chain has useful state to report.
    */
    let mut parts = vec![
        "Akra".to_string(),
        format!("thread: {}", inline_thread_label(conversation)),
    ];
    if should_show_auto_follow_status(conversation) {
        parts.push(format!(
            "auto: {}",
            compact_auto_follow_status_summary(conversation, INLINE_TAIL_AUTO_FOLLOW_DETAIL_LIMIT,)
        ));
        parts.push(format!(
            "done: {}",
            conversation.auto_follow_state.progress_label()
        ));
    }

    Line::from(parts.join("  |  "))
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

fn should_show_auto_follow_status(conversation: &ConversationViewModel) -> bool {
    !conversation.has_post_turn_settlement_in_flight()
        && (conversation.auto_follow_state.has_live_activity()
            || conversation
                .auto_follow_state
                .post_turn_continuation_paused()
            || conversation.auto_follow_state.completed_auto_turns > 0)
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

fn warning_summary_has_signal(warning_summary: &str) -> bool {
    !matches!(warning_summary.trim(), "warn: none" | "none")
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
    render_mode: InlineHistoryRenderMode,
    conversation: &ConversationViewModel,
) -> Vec<Line<'static>> {
    /*
    Recent transcript mirroring is only needed for render modes that do not keep
    host scrollback visible. The tail becomes a small continuity buffer, so it
    selects human-authored user/assistant content before falling back to status.
    */
    if !render_mode.mirrors_recent_transcript_in_tail() {
        return Vec::new();
    }
    let recent_messages = recent_transcript_messages(conversation);
    if recent_messages.is_empty() {
        return Vec::new();
    }

    recent_messages
        .into_iter()
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

fn recent_transcript_messages(conversation: &ConversationViewModel) -> Vec<&ConversationMessage> {
    // Ignore tool/status noise first; fall back to status rows only when there is no
    // user/assistant content so viewport replay still gives the operator context.
    /*
    The reverse/take/reverse pattern keeps selection cheap while preserving chronological
    display order. Tail replay should read like the last two human-visible messages,
    not like an implementation stack.
    */
    let mut recent_messages = conversation
        .messages
        .iter()
        .rev()
        .filter(|message| {
            message.kind != ConversationMessageKind::Tool
                && message.kind != ConversationMessageKind::Status
        })
        .take(2)
        .collect::<Vec<_>>();
    if recent_messages.is_empty() {
        /*
        Status rows are fallback context, not first-choice transcript content. They
        become useful for startup/loading streams where app-server has emitted
        lifecycle messages but no user or assistant text yet.
        */
        recent_messages = conversation
            .messages
            .iter()
            .rev()
            .filter(|message| message.kind != ConversationMessageKind::Tool)
            .take(2)
            .collect::<Vec<_>>();
    }
    recent_messages.reverse();
    recent_messages
}

fn build_inline_startup_screen_lines_with_context(
    screen_model: &ConversationScreenModel<'_>,
) -> Vec<Line<'static>> {
    /*
    The startup masthead is allowed to be taller than the steady-state tail
    because no transcript exists yet. Once the operator starts typing, callers
    switch to the compact startup overlay tail to keep the prompt close to hand.
    */
    let mut lines = if matches!(screen_model.startup_state, StartupState::Ready(_)) {
        Vec::new()
    } else {
        startup_masthead_lines()
    };
    lines.push(Line::from(vec![
        ratatui::text::Span::styled("Akra", AkraTheme::brand()),
        ratatui::text::Span::raw(
            screen_model.tui_language.startup_axis_row(
                screen_model
                    .tui_language
                    .startup_axis_status(screen_model.shell_action_availability),
                screen_model.recent_session_status_label.as_str(),
                &screen_model
                    .tui_language
                    .github_review_polling_status(&screen_model.github_review_polling_status_label),
            ),
        ),
    ]));
    match screen_model.startup_state {
        StartupState::Idle => {
            lines.push(Line::from(startup_preparing_status_line()));
            if let Some(conversation) = screen_model.ready_conversation() {
                lines.push(Line::from(
                    screen_model
                        .tui_language
                        .startup_workspace_line(&conversation.cwd),
                ));
            }
        }
        StartupState::Loading => {
            lines.push(Line::from(startup_initializing_status_line()));
            lines.extend(super::super::build_startup_check_lines_from_state(
                screen_model.startup_state,
            ));
        }
        StartupState::Ready(ready) => {
            lines.push(Line::from(
                screen_model.tui_language.startup_workspace_line(&ready.cwd),
            ));
            lines.push(Line::from(startup_diagnostics_summary_line(
                ready,
                screen_model.tui_language,
            )));
            lines.push(Line::from(startup_attachment_summary_line(
                ready,
                screen_model.tui_language,
            )));
            if let Some(first_warning) = ready.warnings.first() {
                lines.push(Line::from(screen_model.tui_language.startup_warning_line(
                    &compact_inline_detail(first_warning, INLINE_TAIL_NOTICE_DETAIL_LIMIT),
                )));
            }
            lines.push(Line::from(
                screen_model.tui_language.startup_ready_action_line(),
            ));
            if startup_prompt_buffered_in_context(screen_model) {
                lines.push(Line::from(
                    screen_model.tui_language.startup_buffered_prompt_line(),
                ));
            } else {
                lines.push(Line::from(
                    screen_model.tui_language.startup_examples_line(),
                ));
            }
            lines.push(Line::from(
                screen_model.tui_language.startup_shortcuts_line(),
            ));
        }
        StartupState::Failed(message) => {
            lines.push(Line::from(
                screen_model.tui_language.startup_status_line(message),
            ));
            for warning_line in
                super::super::build_startup_warning_lines_from_state(screen_model.startup_state)
                    .into_iter()
                    .filter(|line| !line.to_string().eq_ignore_ascii_case("no warnings"))
            {
                lines.push(Line::from(screen_model.tui_language.startup_warning_line(
                    &compact_inline_detail(
                        &warning_line.to_string(),
                        INLINE_TAIL_NOTICE_DETAIL_LIMIT,
                    ),
                )));
            }
        }
    }

    lines.push(Line::from(""));
    lines
}

fn build_inline_startup_overlay_tail_lines_with_context(
    screen_model: &ConversationScreenModel<'_>,
) -> Vec<Line<'static>> {
    /*
    Compact startup tail is deliberately a single operational axis row. It is used
    while overlays or buffered input need vertical space, so detailed diagnostics
    stay available through inspection instead of crowding the prompt.
    */
    vec![Line::from(vec![
        ratatui::text::Span::styled("Akra", AkraTheme::brand()),
        ratatui::text::Span::raw(
            screen_model.tui_language.startup_axis_row(
                screen_model
                    .tui_language
                    .startup_axis_status(screen_model.shell_action_availability),
                screen_model.recent_session_status_label.as_str(),
                &screen_model
                    .tui_language
                    .github_review_polling_status(&screen_model.github_review_polling_status_label),
            ),
        ),
    ])]
}

fn startup_masthead_lines() -> Vec<Line<'static>> {
    vec![
        Line::from(Span::styled(
            " █████╗ ██╗  ██╗██████╗  █████╗",
            AkraTheme::brand(),
        )),
        Line::from(Span::styled(
            "██╔══██╗██║ ██╔╝██╔══██╗██╔══██╗",
            AkraTheme::brand(),
        )),
        Line::from(Span::styled(
            "███████║█████╔╝ ██████╔╝███████║",
            AkraTheme::brand(),
        )),
        Line::from(Span::styled(
            "██╔══██║██╔═██╗ ██╔══██╗██╔══██║",
            AkraTheme::brand(),
        )),
        Line::from(Span::styled(
            "██║  ██║██║  ██╗██║  ██║██║  ██║",
            AkraTheme::brand(),
        )),
        Line::from(Span::styled(
            "╚═╝  ╚═╝╚═╝  ╚═╝╚═╝  ╚═╝╚═╝  ╚═╝",
            AkraTheme::brand(),
        )),
    ]
}

fn startup_prompt_buffered_in_context(screen_model: &ConversationScreenModel<'_>) -> bool {
    screen_model
        .composer()
        .is_some_and(|composer| !composer.state.input_buffer.trim().is_empty())
}

pub(super) fn build_inline_tail_prompt_lines_with_context(
    screen_model: &ConversationScreenModel<'_>,
) -> Vec<Line<'static>> {
    /*
    Prompt copy is separated from the status body because layout code also uses
    this function to compute cursor offsets. Loading/failed states get static
    affordance rows; ready state delegates to the input-aware branch below.
    */
    if screen_model.shell_overlay == ShellOverlay::Approval {
        return vec![Line::from(
            "prompt: paused while an approval decision is pending",
        )];
    }
    if screen_model
        .composer()
        .is_some_and(|composer| composer.viewport_transcript_handoff_pending)
        && (screen_model.shell_overlay != ShellOverlay::Hidden || screen_model.dialog_visible())
    {
        return vec![Line::from("prompt: response held while the dialog is open")];
    }
    let mut lines = match screen_model.conversation_state {
        ShellConversationState::Loading => vec![Line::from("prompt: waiting for shell readiness")],
        ShellConversationState::Failed(message) => {
            vec![Line::from(format!("prompt: unavailable  |  {message}"))]
        }
        ShellConversationState::Ready(_) => {
            let composer = screen_model
                .composer()
                .expect("ready conversation must project composer state");
            build_inline_ready_prompt_lines(
                composer,
                screen_model.shell_action_availability,
                screen_model.tui_language,
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
    }
    lines
}

fn parallel_loading_prompt_indicator_frame(animation_elapsed_millis: u128) -> &'static str {
    const FRAMES: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
    let tick = (animation_elapsed_millis / 120) as usize;
    FRAMES[tick % FRAMES.len()]
}

fn build_inline_ready_prompt_lines(
    composer: &ConversationComposerScreenModel<'_>,
    shell_action_availability: ShellActionAvailability,
    language: TuiLanguage,
) -> Vec<Line<'static>> {
    let prompt_buffer = build_prompt_buffer_view(composer);
    let mut lines = prompt_buffer.lines;

    // Empty prompt copy prioritizes what blocks or enables the next Enter press.
    // Buffered prompt copy instead explains what will happen to the typed text.
    if composer.state.input_buffer.is_empty() {
        /*
        Empty-buffer copy is command guidance rather than content preview. It
        must explain whether Enter can send immediately, is gated by startup, or
        is blocked by a running/paused automation state.
        */
        if composer.post_turn_settlement_in_flight {
            lines.push(Line::from("prompt: type now  |  Enter when settled"));
            return lines;
        }
        if composer.auto_follow_has_live_activity {
            lines.push(Line::from("prompt: type now  |  Enter when idle"));
            return lines;
        }
        let line = match (composer.input_state, shell_action_availability) {
            (_, ShellActionAvailability::Pending) if composer.input_state.can_submit_now() => {
                "prompt: waiting for startup  |  type now, Enter sends when ready".to_string()
            }
            (_, ShellActionAvailability::Blocked) if composer.input_state.can_submit_now() => {
                "prompt: blocked by startup diagnostics  |  Ctrl+d inspect".to_string()
            }
            (ConversationInputState::DraftReady, _) => {
                "prompt: new thread ready  |  Enter send  |  Ctrl+j nl  |  :help".to_string()
            }
            (ConversationInputState::ReadyToContinue, _) => {
                "prompt: session ready  |  Enter send  |  Ctrl+j nl  |  :help".to_string()
            }
            (ConversationInputState::SubmittingTurn, _) => {
                language.turn_starting_prompt_hint(false).to_string()
            }
            (ConversationInputState::StreamingTurn, _) => {
                language.running_prompt_hint(false).to_string()
            }
        };
        lines.push(Line::from(line));
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
        if palette.suggestions().is_empty() {
            lines.push(Line::from(language.inline_command_palette_empty_key_line()));
        } else {
            lines.extend(
                language
                    .inline_command_palette_key_lines()
                    .into_iter()
                    .map(Line::from),
            );
        }
        lines.extend(build_shell_command_palette_lines(composer, language));
        return lines;
    }

    if let Some(command) = InlineShellCommandInput::parse(&composer.state.input_buffer) {
        /*
        Parsed shell commands get a dedicated hint line before generic prompt
        guidance. That keeps destructive or overlay-opening commands legible
        while the text is still just buffered input.
        */
        lines.push(Line::from(format!(
            "command: {}",
            command.localized_buffered_hint(language)
        )));
        return lines;
    }

    if composer.post_turn_settlement_in_flight && composer.input_state.can_submit_now() {
        lines.push(Line::from("buffered  |  Enter when settled  |  Ctrl+j nl"));
        return lines;
    }

    if composer.auto_follow_has_live_activity && composer.input_state.can_submit_now() {
        /*
        Auto follow-up activity can make the shell appear idle enough to type into,
        but Enter would race the continuation. This line keeps the buffered prompt
        visible while making the idle gate explicit.
        */
        lines.push(Line::from(
            "buffered prompt  |  auto-follow busy  |  Enter when idle",
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
            "queued until startup is ready  |  editing cancels the queued send"
        }
        (
            ConversationInputState::DraftReady | ConversationInputState::ReadyToContinue,
            ShellActionAvailability::Ready,
        ) => "buffered prompt  |  Enter send  |  Ctrl+j nl",
        (ConversationInputState::DraftReady | ConversationInputState::ReadyToContinue, _) => {
            "buffered prompt  |  Enter when ready  |  Ctrl+j nl"
        }
        (ConversationInputState::SubmittingTurn, _) => language.turn_starting_prompt_hint(true),
        (ConversationInputState::StreamingTurn, _) => language.running_prompt_hint(true),
    };
    lines.push(Line::from(hint));
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
        AutoFollowRuntimePhase, ConversationState, InlineShellCommand, NativeTuiApp,
    };
    use crate::core::app::{QueueMutationCorrelation, QueueMutationIntent, StartupReadySnapshot};
    use crate::domain::conversation::{ConversationMessage, ConversationMessageKind};
    use crate::domain::planning::{
        PlanningQueueMutationKind, PlanningQueueMutationReceipt, PlanningQueueMutationReceiptEntry,
        TaskStatus,
    };
    use crate::domain::startup_diagnostics::StartupDiagnostics;
    use crate::domain::terminal_bridge_attachment::TerminalBridgeAttachmentProfile;
    use std::time::Instant;

    fn ready_conversation(app: &NativeTuiApp) -> &ConversationViewModel {
        let ConversationState::Ready(conversation) = &app.conversation_state else {
            panic!("expected ready conversation");
        };
        conversation
    }

    fn ready_conversation_mut(app: &mut NativeTuiApp) -> &mut ConversationViewModel {
        let ConversationState::Ready(conversation) = &mut app.conversation_state else {
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
            availability,
            language,
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
    fn shell_loading_and_failed_tails_keep_prompt_copy_without_ready_conversation() {
        let mut app = test_native_tui_app();

        app.conversation_state = ConversationState::Loading;
        app.startup_state = StartupState::Loading;
        let loading = render_tail(&app, None);
        assert!(loading.contains("thread: loading"));
        assert!(loading.contains("runtime: loading thread history"));
        assert!(loading.contains("prompt: waiting for shell readiness"));

        app.conversation_state =
            ConversationState::Failed("session catalog unavailable".to_string());
        let failed = render_tail(&app, None);
        assert!(failed.contains("thread: unavailable"));
        assert!(failed.contains("status: session catalog unavailable"));
        assert!(failed.contains("prompt: unavailable  |  session catalog unavailable"));
    }

    #[test]
    fn startup_tail_covers_masthead_overlay_state_and_starter_variants() {
        let mut app = test_native_tui_app();

        app.startup_state = StartupState::Idle;
        let idle = render_tail(&app, None);
        assert!(idle.contains("Akra"));
        assert!(idle.contains("preparing startup checks"));
        assert!(idle.contains("workspace: /tmp/root"));

        app.startup_state = StartupState::Loading;
        let loading = render_tail(&app, None);
        assert!(loading.contains("initializing codex shell"));
        assert!(loading.contains("opening codex app-server"));

        app.startup_state = StartupState::Ready(startup_ready_snapshot(true));
        let ready = render_tail(&app, None);
        assert!(ready.contains("workspace: /tmp/root"));
        assert!(ready.contains("first warning should stay visible"));
        assert!(ready.contains("ready: send a task or reopen a session"));
        assert!(!ready.contains("████"));

        app.startup_state = StartupState::Failed("codex missing".to_string());
        let failed = render_tail(&app, None);
        assert!(failed.contains("codex missing"));

        ready_conversation_mut(&mut app).composer.input_buffer =
            "queued startup prompt".to_string();
        let overlay = render_tail(&app, None);
        assert!(overlay.contains("Akra"));
        assert!(!overlay.contains("████"));

        app.startup_state = StartupState::Ready(startup_ready_snapshot(true));
        let context = ConversationScreenModel::from_app(&app);
        assert!(
            rendered(build_inline_startup_screen_lines_with_context(&context))
                .contains("draft: opening prompt buffered below")
        );

        app.conversation_state = ConversationState::Loading;
        let loading_context = ConversationScreenModel::from_app(&app);
        assert!(!startup_prompt_buffered_in_context(&loading_context));
    }

    #[test]
    fn ready_status_helpers_cover_auto_follow_completion_warnings_and_transcript() {
        let startup_state = StartupState::Loading;
        let mut conversation = ConversationViewModel::new_draft("/tmp/root".to_string());
        conversation.thread_id = "thread-1".to_string();
        conversation.status_text =
            "a very long status line that should be compacted inside the inline tail".to_string();

        assert!(!should_show_auto_follow_status(&conversation));
        conversation.auto_follow_state.completed_auto_turns = 2;
        assert!(should_show_auto_follow_status(&conversation));
        conversation.auto_follow_state.completed_auto_turns = 0;
        conversation
            .auto_follow_state
            .pause_post_turn_continuation();
        assert!(should_show_auto_follow_status(&conversation));
        conversation.auto_follow_state.set_max_auto_turns(5);
        conversation.auto_follow_state.runtime_phase = AutoFollowRuntimePhase::Queued {
            started_at: Instant::now(),
            turn_index: 3,
        };

        let ribbon = build_ready_status_ribbon_line(&conversation).to_string();
        assert!(ribbon.contains("auto:"));
        assert!(ribbon.contains("done:"));

        let context = context_for(
            &startup_state,
            ShellActionAvailability::Blocked,
            ShellConversationState::Ready(&conversation),
        );
        let detail = build_ready_status_detail_line(&conversation, &context)
            .expect("non-benign status detail")
            .to_string();
        assert!(detail.contains("startup: startup diagnostics need attention"));
        assert!(detail.contains("gh: polling"));

        let mut running_draft = ConversationViewModel::new_draft("/tmp/root".to_string());
        running_draft.record_turn_started("turn-1".to_string());
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

        assert!(!warning_summary_has_signal("warn: none"));
        assert!(!warning_summary_has_signal("none"));
        assert!(warning_summary_has_signal("warn: disk almost full"));

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

        assert!(
            build_recent_transcript_summary_lines(
                InlineHistoryRenderMode::HostScrollback,
                &conversation
            )
            .is_empty()
        );
        conversation.messages = vec![
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
        let transcript = rendered(build_recent_transcript_summary_lines(
            InlineHistoryRenderMode::ViewportReplay,
            &conversation,
        ));
        assert!(transcript.contains("recent you: first user"));
        assert!(transcript.contains("recent codex: second agent line"));
        assert!(!transcript.contains("tool noise"));

        conversation.messages = vec![
            ConversationMessage::new(ConversationMessageKind::Tool, "tool noise", None, None),
            ConversationMessage::new(ConversationMessageKind::Status, "   ", None, None),
        ];
        let fallback = rendered(build_recent_transcript_summary_lines(
            InlineHistoryRenderMode::ViewportReplay,
            &conversation,
        ));
        assert!(fallback.contains("recent status: (blank)"));
    }

    #[test]
    fn ready_prompt_copy_covers_empty_buffer_commands_and_buffered_states() {
        for (state, availability, expected) in [
            (
                ConversationInputState::DraftReady,
                ShellActionAvailability::Pending,
                "waiting for startup",
            ),
            (
                ConversationInputState::DraftReady,
                ShellActionAvailability::Blocked,
                "blocked by startup diagnostics",
            ),
            (
                ConversationInputState::DraftReady,
                ShellActionAvailability::Ready,
                "new thread ready",
            ),
            (
                ConversationInputState::ReadyToContinue,
                ShellActionAvailability::Ready,
                "session ready",
            ),
            (
                ConversationInputState::SubmittingTurn,
                ShellActionAvailability::Ready,
                "wait for turn start",
            ),
            (
                ConversationInputState::StreamingTurn,
                ShellActionAvailability::Ready,
                "Enter queue",
            ),
        ] {
            let mut conversation = ConversationViewModel::new_draft("/tmp/root".to_string());
            conversation.input_state = state;
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
            .move_inline_shell_command_palette_selection(2);
        let palette_prompt = rendered_ready_prompt(
            &palette,
            ShellActionAvailability::Ready,
            TuiLanguage::English,
        );
        assert!(palette_prompt.contains("palette 3/19"));
        assert!(palette_prompt.contains("Up/Shift+Tab previous"));
        assert!(palette_prompt.contains("Down/Tab next"));
        assert!(palette_prompt.contains(":diag"));

        let korean_palette_prompt = rendered_ready_prompt(
            &palette,
            ShellActionAvailability::Ready,
            TuiLanguage::Korean,
        );
        assert!(
            korean_palette_prompt
                .contains(&TuiLanguage::Korean.inline_command_palette_header(3, 19))
        );
        for key_line in TuiLanguage::Korean.inline_command_palette_key_lines() {
            assert!(korean_palette_prompt.contains(key_line));
        }
        assert!(korean_palette_prompt.contains(&format!(
            ":peek  {}",
            TuiLanguage::Korean.inline_shell_command_detail(InlineShellCommand::Peek)
        )));

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
        assert!(korean_command_prompt.contains(&format!("command: {korean_hint}")));
        assert!(!korean_command_prompt.contains("Press Enter"));

        let mut command = ConversationViewModel::new_draft("/tmp/root".to_string());
        command.composer.input_buffer = ":reset queue".to_string();
        let command_prompt = rendered_ready_prompt(
            &command,
            ShellActionAvailability::Ready,
            TuiLanguage::English,
        );
        assert!(
            command_prompt.contains("command: Press Enter to reset queue-side planning state.")
        );

        let mut busy = ConversationViewModel::new_draft("/tmp/root".to_string());
        busy.composer.input_buffer = "next prompt".to_string();
        busy.auto_follow_state.mark_auto_turn_queued();
        let busy_prompt =
            rendered_ready_prompt(&busy, ShellActionAvailability::Ready, TuiLanguage::English);
        assert!(busy_prompt.contains("auto-follow busy"));

        busy.composer.clear_input_buffer();
        let empty_busy_prompt =
            rendered_ready_prompt(&busy, ShellActionAvailability::Ready, TuiLanguage::English);
        assert!(empty_busy_prompt.contains("prompt: type now  |  Enter when idle"));

        let mut armed = ConversationViewModel::new_draft("/tmp/root".to_string());
        armed.composer.input_buffer = "queued".to_string();
        armed.composer.startup_submit_armed = true;
        let armed_prompt = rendered_ready_prompt(
            &armed,
            ShellActionAvailability::Pending,
            TuiLanguage::English,
        );
        assert!(armed_prompt.contains("editing cancels the queued send"));

        for (state, availability, expected) in [
            (
                ConversationInputState::DraftReady,
                ShellActionAvailability::Ready,
                "Enter send",
            ),
            (
                ConversationInputState::ReadyToContinue,
                ShellActionAvailability::Blocked,
                "Enter when ready",
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
            conversation.input_state = state;
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
        conversation.begin_post_turn_settlement("turn-1");
        conversation.auto_follow_state.set_max_auto_turns(0);
        conversation.auto_follow_state.mark_auto_turn_queued();

        assert!(!conversation.can_accept_manual_prompt());
        assert!(
            build_working_line(&conversation, 40, Instant::now())
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
        assert!(
            buffered_prompt
                .lines()
                .all(|line| line.chars().count() <= 48)
        );
    }

    #[test]
    fn settlement_tail_shows_one_phase_truth_without_auto_or_idle_noise() {
        let mut app = test_native_tui_app();
        app.startup_state = StartupState::Ready(startup_ready_snapshot(true));
        let conversation = ready_conversation_mut(&mut app);
        conversation.thread_id = "thread-settlement".to_string();
        conversation.begin_post_turn_settlement("turn-1");
        conversation.auto_follow_state.set_max_auto_turns(0);
        conversation.auto_follow_state.completed_auto_turns = 1;
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
        app.startup_state = StartupState::Ready(startup_ready_snapshot(true));
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
        app.queue_mutation_ui_state
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

        app.tui_language = TuiLanguage::Korean;
        let korean_tail = render_tail(&app, None);
        assert!(korean_tail.contains("큐: op-1  |  권한 확인 대기 중"));
    }

    #[test]
    fn required_queue_authority_refresh_uses_localized_non_actionable_tail_copy() {
        let mut app = test_native_tui_app();
        app.startup_state = StartupState::Ready(startup_ready_snapshot(true));
        app.tui_language = TuiLanguage::Korean;
        ready_conversation_mut(&mut app).thread_id = "thread-refresh-required".to_string();
        app.queue_mutation_ui_state.require_authority_refresh();

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
        app.startup_state = StartupState::Ready(startup_ready_snapshot(true));
        let conversation = ready_conversation_mut(&mut app);
        conversation.thread_id = "thread-1".to_string();
        conversation.base_warnings.push("warning one".to_string());
        conversation.warnings.push("warning one".to_string());
        conversation.runtime_notices.push("runtime one".to_string());
        conversation.composer.input_buffer = "buffered".to_string();

        let tail = render_tail(&app, Some("review changed"));

        assert!(tail.contains("runtime:"));
        assert!(tail.contains("warning one"));
        assert!(tail.contains("notice:"));
        assert!(tail.contains("review changed"));
        assert!(tail.contains("buffered prompt"));

        assert_eq!(ready_conversation(&app).thread_id, "thread-1");
    }
}
