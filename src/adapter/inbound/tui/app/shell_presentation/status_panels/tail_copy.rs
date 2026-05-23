use ratatui::text::{Line, Span};
use std::time::{SystemTime, UNIX_EPOCH};

use super::super::capability_copy::{
    startup_attachment_summary_line, startup_diagnostics_summary_line,
    startup_initializing_status_line, startup_preparing_status_line,
    thread_history_loading_status_line,
};
use super::super::planning::build_planning_worker_panel_lines;
use super::super::planning::status_projection::build_planning_status_surface_projection;
use super::super::prompt_composer::{build_prompt_buffer_view, build_shell_command_palette_lines};
use super::super::{
    AkraTheme, ConversationInputState, ConversationViewModel, INLINE_TAIL_AUTO_FOLLOW_DETAIL_LIMIT,
    INLINE_TAIL_NOTICE_DETAIL_LIMIT, INLINE_TAIL_PLANNING_DETAIL_LIMIT,
    INLINE_TAIL_RUNTIME_NOTICE_DETAIL_LIMIT, INLINE_TAIL_STATUS_DETAIL_LIMIT,
    INLINE_TAIL_WARNING_DETAIL_LIMIT, InlineHistoryRenderMode, InlineShellCommandInput, Modifier,
    NativeTuiApp, ShellActionAvailability, ShellConversationState, ShellCorePresentationContext,
    ShellOverlay, StartupState, auto_follow_prompt_status_line, build_working_line,
    compact_inline_detail, inline_input_state_label, turn_status_label,
};
use super::parallel_working_copy::build_parallel_slot_working_line;
use super::tail_shared::{
    build_operator_notice_line, compact_auto_follow_status_summary, compact_inline_summary_label,
    inline_thread_label, parallel_mode_alert_line, parallel_mode_summary_line,
};

use crate::adapter::inbound::tui::conversation_text::conversation_message_kind_label;
use crate::domain::conversation::{ConversationMessage, ConversationMessageKind};

/* The inline tail is the compact operational dashboard below the transcript. It
 * keeps high-priority state visible in this order: startup readiness, conversation
 * turn state, parallel/planning health, recent transcript context, then prompt
 * affordances. The order matters because this view is scanned repeatedly while a
 * turn is streaming or while startup checks are blocking submission.
 */
pub(super) fn build_inline_tail_lines_with_context(
    app: &NativeTuiApp,
    context: &ShellCorePresentationContext<'_>,
    github_review_recent_changes_summary: Option<String>,
) -> Vec<Line<'static>> {
    /*
    Planning projection is computed before state branching because both the ready
    tail and prompt affordance lines need the same compact limits. Keeping this
    projection renderer-adjacent prevents lower application services from knowing
    about terminal row budgets.
    */
    let planning_status_projection = context.ready_conversation().map(|conversation| {
        build_planning_status_surface_projection(
            app,
            conversation,
            INLINE_TAIL_PLANNING_DETAIL_LIMIT,
            INLINE_TAIL_NOTICE_DETAIL_LIMIT,
            false,
        )
    });
    let planning_worker_panel_lines =
        build_planning_worker_panel_lines(app, INLINE_TAIL_NOTICE_DETAIL_LIMIT);

    if context.startup_screen_is_active() {
        let has_buffered_input = context
            .ready_conversation()
            .is_some_and(|conversation| !conversation.input_buffer.is_empty());
        // The full startup masthead is useful only before the operator starts typing.
        // Once an overlay or buffered prompt exists, keep the tail compact so the
        // prompt remains close to its status line.
        let mut lines = if app.shell_overlay == ShellOverlay::Hidden && !has_buffered_input {
            build_inline_startup_screen_lines_with_context(context)
        } else {
            build_inline_startup_overlay_tail_lines_with_context(context)
        };
        lines.extend(build_inline_tail_prompt_lines_with_context(
            app,
            context,
            app.shell_action_availability(),
        ));
        return lines;
    }
    let mut lines = Vec::new();
    match context.conversation_state {
        ShellConversationState::Loading => {
            /*
            Loading and failed states still render a full tail because the inline
            terminal layout needs stable prompt/status rows even before a thread
            snapshot exists. These branches avoid conversation-only helpers.
            */
            lines.push(Line::from(format!(
                "Akra  |  thread: loading  |  startup: {}  |  sessions: {}",
                context.shell_action_availability.status_text(),
                context.recent_session_status_label.as_str(),
            )));
            lines.push(Line::from(format!(
                "runtime: loading thread history  |  gh: {}  |  flow: terminal main buffer",
                context.github_review_polling_status_label.as_str(),
            )));
            lines.push(Line::from(format!(
                "status: {}",
                thread_history_loading_status_line()
            )));
        }
        ShellConversationState::Failed(message) => {
            lines.push(Line::from(format!(
                "Akra  |  thread: unavailable  |  startup: {}  |  sessions: {}",
                context.shell_action_availability.status_text(),
                context.recent_session_status_label.as_str(),
            )));
            lines.push(Line::from(format!(
                "runtime: unavailable  |  gh: {}  |  flow: terminal main buffer",
                context.github_review_polling_status_label.as_str(),
            )));
            lines.push(Line::from(format!("status: {message}")));
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

            /*
            The first three lines form the always-visible health header. They avoid
            expensive detail expansion and reserve later rows for planning, worker,
            transcript, and notice detail that may appear only in specific states.
            */
            lines.push(build_ready_status_ribbon_line(conversation));
            lines.push(build_ready_status_detail_line(conversation, context));
            if let Some(completion_line) = build_completion_alert_line(conversation) {
                lines.push(completion_line);
            }
            if let Some(runtime_notice_summary) = runtime_notice_summary {
                lines.push(Line::from(format!(
                    "runtime: {runtime_notice_summary}  |  {warning_summary}",
                )));
            } else if warning_summary_has_signal(&warning_summary) {
                lines.push(Line::from(warning_summary));
            }
            if !app.turn_options.is_default() {
                lines.push(Line::from(format!(
                    "turn options: {}",
                    app.turn_options.summary_label()
                )));
            }
            if let Some(parallel_summary_line) = parallel_mode_summary_line(app) {
                lines.push(Line::from(parallel_summary_line));
            }

            if let Some(parallel_mode_alert_line) = parallel_mode_alert_line(app) {
                /*
                Parallel alerts sit immediately after the summary line because they
                can block dispatch even when the rest of the conversation is ready.
                Placing them before planning detail keeps slot/recovery issues from
                being buried below queue copy.
                */
                lines.push(Line::from(parallel_mode_alert_line));
            }
            if let Some(working_line) =
                build_working_line(conversation, INLINE_TAIL_STATUS_DETAIL_LIMIT)
            {
                lines.push(working_line);
            }
            if let Some(planning_projection) = planning_status_projection.as_ref() {
                /*
                The planning projection is already budgeted for inline detail limits.
                Tail copy preserves its order: summary first, queue framing next,
                notices last. That mirrors the popup/status surfaces without making
                this compact renderer know planning service enum internals.
                */
                if let Some(planning_line) = planning_projection.summary_line.as_deref() {
                    lines.push(Line::from(planning_line.to_string()));
                }
                lines.extend(planning_projection.queue_framing_lines.iter().cloned());
                if let Some(planning_notice_line) = planning_projection.notice_line.as_deref() {
                    lines.push(Line::from(planning_notice_line.to_string()));
                }
            } else {
                lines.push(Line::from(format!(
                    "planning: unavailable  |  startup: {}",
                    context.shell_action_availability.status_text()
                )));
            }
            if let Some(parallel_working_line) = build_parallel_slot_working_line(app) {
                lines.push(parallel_working_line);
            }

            lines.extend(planning_worker_panel_lines.into_iter().map(Line::from));
            lines.extend(build_recent_transcript_summary_lines(
                app.inline_history_render_mode,
                conversation,
            ));
            if let Some(notice_line) = build_operator_notice_line(
                github_review_recent_changes_summary.as_deref(),
                conversation,
                INLINE_TAIL_NOTICE_DETAIL_LIMIT,
            ) {
                lines.push(Line::from(format!("notice: {notice_line}")));
            }
        }
    }

    lines.extend(build_inline_tail_prompt_lines_with_context(
        app,
        context,
        app.shell_action_availability(),
    ));
    lines
}
fn build_ready_status_ribbon_line(conversation: &ConversationViewModel) -> Line<'static> {
    /*
    The ribbon is the single-line state index for the ready shell. It carries
    thread identity, turn lifecycle, and input readiness. Auto-follow details are
    only added while an automatic chain has useful state to report.
    */
    let mut parts = vec![
        "Akra".to_string(),
        format!("thread: {}", inline_thread_label(conversation)),
        format!("turn: {}", turn_status_label(conversation)),
        format!(
            "input: {}",
            inline_input_state_label(conversation.input_state)
        ),
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

fn should_show_auto_follow_status(conversation: &ConversationViewModel) -> bool {
    conversation.auto_follow_state.has_live_activity()
        || conversation
            .auto_follow_state
            .post_turn_continuation_paused()
        || conversation.auto_follow_state.completed_auto_turns > 0
}

fn build_ready_status_detail_line(
    conversation: &ConversationViewModel,
    context: &ShellCorePresentationContext<'_>,
) -> Line<'static> {
    let mut parts = vec![format!(
        "status: {}",
        compact_inline_detail(&conversation.status_text, INLINE_TAIL_STATUS_DETAIL_LIMIT)
    )];
    if context.shell_action_availability != ShellActionAvailability::Ready {
        parts.push(format!(
            "startup: {}",
            context.shell_action_availability.status_text()
        ));
    }
    let github_status = context.github_review_polling_status_label.as_str();
    if github_status != "off" {
        parts.push(format!("gh: {github_status}"));
    }

    Line::from(parts.join("  |  "))
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
    context: &ShellCorePresentationContext<'_>,
) -> Vec<Line<'static>> {
    /*
    The startup masthead is allowed to be taller than the steady-state tail
    because no transcript exists yet. Once the operator starts typing, callers
    switch to the compact startup overlay tail to keep the prompt close to hand.
    */
    let mut lines = startup_masthead_lines();
    lines.push(Line::from(vec![
        ratatui::text::Span::styled("Akra", AkraTheme::brand()),
        ratatui::text::Span::raw(
            context.tui_language.startup_axis_row(
                context
                    .tui_language
                    .startup_axis_status(context.shell_action_availability),
                context.recent_session_status_label.as_str(),
                &context
                    .tui_language
                    .github_review_polling_status(&context.github_review_polling_status_label),
            ),
        ),
    ]));
    match context.startup_state {
        StartupState::Idle => {
            lines.push(Line::from(startup_preparing_status_line()));
            if let Some(conversation) = context.ready_conversation() {
                lines.push(Line::from(
                    context
                        .tui_language
                        .startup_workspace_line(&conversation.cwd),
                ));
            }
        }
        StartupState::Loading => {
            lines.push(Line::from(startup_initializing_status_line()));
            lines.extend(super::super::build_startup_check_lines_from_state(
                context.startup_state,
            ));
        }
        StartupState::Ready(ready) => {
            lines.push(Line::from(
                context.tui_language.startup_workspace_line(&ready.cwd),
            ));
            lines.push(Line::from(startup_diagnostics_summary_line(
                ready,
                context.tui_language,
            )));
            lines.push(Line::from(startup_attachment_summary_line(
                ready,
                context.tui_language,
            )));
            if let Some(first_warning) = ready.warnings.first() {
                lines.push(Line::from(context.tui_language.startup_warning_line(
                    &compact_inline_detail(first_warning, INLINE_TAIL_NOTICE_DETAIL_LIMIT),
                )));
            }
            lines.push(Line::from(
                context.tui_language.startup_conversation_label(),
            ));
            lines.push(Line::from(context.tui_language.startup_first_reply_hint()));
            lines.push(Line::from(
                context
                    .tui_language
                    .startup_starter_line(inline_starter_copy_in_context(context)),
            ));
        }
        StartupState::Failed(message) => {
            lines.push(Line::from(
                context.tui_language.startup_status_line(message),
            ));
            for warning_line in
                super::super::build_startup_warning_lines_from_state(context.startup_state)
                    .into_iter()
                    .filter(|line| !line.to_string().eq_ignore_ascii_case("no warnings"))
            {
                lines.push(Line::from(context.tui_language.startup_warning_line(
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
    context: &ShellCorePresentationContext<'_>,
) -> Vec<Line<'static>> {
    /*
    Compact startup tail is deliberately a single operational axis row. It is used
    while overlays or buffered input need vertical space, so detailed diagnostics
    stay available through inspection instead of crowding the prompt.
    */
    vec![Line::from(vec![
        ratatui::text::Span::styled("Akra", AkraTheme::brand()),
        ratatui::text::Span::raw(
            context.tui_language.startup_axis_row(
                context
                    .tui_language
                    .startup_axis_status(context.shell_action_availability),
                context.recent_session_status_label.as_str(),
                &context
                    .tui_language
                    .github_review_polling_status(&context.github_review_polling_status_label),
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

fn inline_starter_copy_in_context(context: &ShellCorePresentationContext<'_>) -> &'static str {
    let Some(conversation) = context.ready_conversation() else {
        /*
        Without a ready conversation there is no input buffer to inspect, so the
        starter copy must be generic and safe for loading/failed startup states.
        */
        return context.tui_language.startup_empty_starter_copy();
    };
    if conversation.input_buffer.trim().is_empty() {
        context.tui_language.startup_empty_starter_copy()
    } else {
        context.tui_language.startup_buffered_starter_copy()
    }
}

pub(super) fn build_inline_tail_prompt_lines_with_context(
    app: &NativeTuiApp,
    context: &ShellCorePresentationContext<'_>,
    shell_action_availability: ShellActionAvailability,
) -> Vec<Line<'static>> {
    /*
    Prompt copy is separated from the status body because layout code also uses
    this function to compute cursor offsets. Loading/failed states get static
    affordance rows; ready state delegates to the input-aware branch below.
    */
    let mut lines = match context.conversation_state {
        ShellConversationState::Loading => vec![Line::from("prompt: waiting for shell readiness")],
        ShellConversationState::Failed(message) => {
            vec![Line::from(format!("prompt: unavailable  |  {message}"))]
        }
        ShellConversationState::Ready(conversation) => {
            build_inline_ready_prompt_lines(conversation, shell_action_availability)
        }
    };
    if app.parallel_mode_loading_prompt_indicator_visible()
        && let Some(first_line) = lines.first_mut()
    {
        first_line.spans.insert(
            0,
            Span::styled(
                format!("{} ", parallel_loading_prompt_indicator_frame()),
                AkraTheme::brand(),
            ),
        );
    }
    lines
}

fn parallel_loading_prompt_indicator_frame() -> &'static str {
    const FRAMES: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
    let tick = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| (duration.as_millis() / 120) as usize)
        .unwrap_or(0);
    FRAMES[tick % FRAMES.len()]
}

fn build_inline_ready_prompt_lines(
    conversation: &ConversationViewModel,
    shell_action_availability: ShellActionAvailability,
) -> Vec<Line<'static>> {
    let prompt_buffer = build_prompt_buffer_view(conversation);
    let mut lines = prompt_buffer.lines;

    // Empty prompt copy prioritizes what blocks or enables the next Enter press.
    // Buffered prompt copy instead explains what will happen to the typed text.
    if conversation.input_buffer.is_empty() {
        /*
        Empty-buffer copy is command guidance rather than content preview. It
        must explain whether Enter can send immediately, is gated by startup, or
        is blocked by a running/paused automation state.
        */
        if let Some(status_line) = auto_follow_prompt_status_line(conversation, true) {
            lines.push(Line::from(status_line));
            return lines;
        }
        let line = match (conversation.input_state, shell_action_availability) {
            (_, ShellActionAvailability::Pending) if conversation.input_state.can_submit_now() => {
                "prompt: waiting for startup  |  type now, Enter sends when ready".to_string()
            }
            (_, ShellActionAvailability::Blocked) if conversation.input_state.can_submit_now() => {
                "prompt: blocked by startup diagnostics  |  Ctrl+d inspect".to_string()
            }
            (ConversationInputState::DraftReady, _) => {
                "prompt: new thread ready  |  Enter send  |  Ctrl+j nl  |  :help".to_string()
            }
            (ConversationInputState::ReadyToContinue, _) => {
                "prompt: session ready  |  Enter send  |  Ctrl+j nl  |  :help".to_string()
            }
            (ConversationInputState::SubmittingTurn, _) => {
                "prompt: sending  |  wait for turn start".to_string()
            }
            (ConversationInputState::StreamingTurn, _) => {
                "prompt: turn running  |  type now, Enter when idle".to_string()
            }
        };
        lines.push(Line::from(line));
        return lines;
    }

    if conversation.inline_shell_command_palette_state.is_active() {
        /*
        Palette copy takes precedence over raw command parsing because the
        operator is navigating an already-open menu; showing parse hints here
        would fight with Up/Down/Enter semantics.
        */
        lines.push(Line::from(
            "command: palette  |  Up/Down move  |  Enter choose  |  Esc close",
        ));
        lines.extend(build_shell_command_palette_lines(conversation));
        return lines;
    }

    if let Some(command) = InlineShellCommandInput::parse(&conversation.input_buffer) {
        /*
        Parsed shell commands get a dedicated hint line before generic prompt
        guidance. That keeps destructive or overlay-opening commands legible
        while the text is still just buffered input.
        */
        lines.push(Line::from(format!("command: {}", command.buffered_hint())));
        return lines;
    }

    if conversation.auto_follow_state.has_live_activity()
        && conversation.input_state.can_submit_now()
    {
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

    let hint = match (conversation.input_state, shell_action_availability) {
        (
            ConversationInputState::DraftReady | ConversationInputState::ReadyToContinue,
            ShellActionAvailability::Pending,
        ) if conversation.startup_submit_armed => {
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
        (ConversationInputState::SubmittingTurn | ConversationInputState::StreamingTurn, _) => {
            "buffered prompt  |  Enter when idle  |  Ctrl+j nl"
        }
    };
    lines.push(Line::from(hint));
    lines
}

#[cfg(test)]
mod coverage_tests {
    use super::*;
    use crate::adapter::inbound::tui::app::conversation_model::RecordedAutoFollowActivity;
    use crate::adapter::inbound::tui::app::language::TuiLanguage;
    use crate::adapter::inbound::tui::app::test_helpers::test_native_tui_app;
    use crate::adapter::inbound::tui::app::{AutoFollowRuntimePhase, ConversationState};
    use crate::core::app::StartupReadySnapshot;
    use crate::domain::conversation::{ConversationMessage, ConversationMessageKind};
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

    fn render_tail(app: &NativeTuiApp, recent_changes: Option<&str>) -> String {
        let context = ShellCorePresentationContext::from_app(app);
        rendered(build_inline_tail_lines_with_context(
            app,
            &context,
            recent_changes.map(str::to_string),
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
    ) -> ShellCorePresentationContext<'a> {
        ShellCorePresentationContext {
            show_startup_ascii_art: false,
            startup_state,
            shell_action_availability,
            recent_session_status_label: "loaded".to_string(),
            github_review_polling_status_label: "polling".to_string(),
            tui_language: TuiLanguage::English,
            parallel_mode_enabled: false,
            conversation_state,
        }
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
        assert!(ready.contains("first reply appears here after you send the opening prompt"));

        app.startup_state = StartupState::Failed("codex missing".to_string());
        let failed = render_tail(&app, None);
        assert!(failed.contains("codex missing"));

        ready_conversation_mut(&mut app).input_buffer = "queued startup prompt".to_string();
        let overlay = render_tail(&app, None);
        assert!(overlay.contains("Akra"));
        assert!(!overlay.contains("████"));

        app.startup_state = StartupState::Ready(startup_ready_snapshot(true));
        let context = ShellCorePresentationContext::from_app(&app);
        assert!(
            rendered(build_inline_startup_screen_lines_with_context(&context))
                .contains("opening prompt buffered below")
        );

        app.conversation_state = ConversationState::Loading;
        let loading_context = ShellCorePresentationContext::from_app(&app);
        assert_eq!(
            inline_starter_copy_in_context(&loading_context),
            loading_context.tui_language.startup_empty_starter_copy()
        );
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
        conversation
            .auto_follow_state
            .clear_post_turn_continuation_pause();
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
        let detail = build_ready_status_detail_line(&conversation, &context).to_string();
        assert!(detail.contains("startup: startup diagnostics need attention"));
        assert!(detail.contains("gh: polling"));

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
                "sending",
            ),
            (
                ConversationInputState::StreamingTurn,
                ShellActionAvailability::Ready,
                "turn running",
            ),
        ] {
            let mut conversation = ConversationViewModel::new_draft("/tmp/root".to_string());
            conversation.input_state = state;
            let prompt = rendered(build_inline_ready_prompt_lines(&conversation, availability));
            assert!(
                prompt.contains(expected),
                "expected `{expected}` in `{prompt}`"
            );
        }

        let mut palette = ConversationViewModel::new_draft("/tmp/root".to_string());
        palette.input_buffer = ":".to_string();
        palette.sync_inline_shell_command_palette();
        let palette_prompt = rendered(build_inline_ready_prompt_lines(
            &palette,
            ShellActionAvailability::Ready,
        ));
        assert!(palette_prompt.contains("command: palette"));
        assert!(palette_prompt.contains(":diag"));

        let mut command = ConversationViewModel::new_draft("/tmp/root".to_string());
        command.input_buffer = ":reset queue".to_string();
        let command_prompt = rendered(build_inline_ready_prompt_lines(
            &command,
            ShellActionAvailability::Ready,
        ));
        assert!(
            command_prompt.contains("command: Press Enter to reset queue-side planning state.")
        );

        let mut busy = ConversationViewModel::new_draft("/tmp/root".to_string());
        busy.input_buffer = "next prompt".to_string();
        busy.auto_follow_state.begin_post_turn_evaluation();
        let busy_prompt = rendered(build_inline_ready_prompt_lines(
            &busy,
            ShellActionAvailability::Ready,
        ));
        assert!(busy_prompt.contains("auto-follow busy"));

        let mut armed = ConversationViewModel::new_draft("/tmp/root".to_string());
        armed.input_buffer = "queued".to_string();
        armed.startup_submit_armed = true;
        let armed_prompt = rendered(build_inline_ready_prompt_lines(
            &armed,
            ShellActionAvailability::Pending,
        ));
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
                ConversationInputState::StreamingTurn,
                ShellActionAvailability::Ready,
                "Enter when idle",
            ),
        ] {
            let mut conversation = ConversationViewModel::new_draft("/tmp/root".to_string());
            conversation.input_buffer = "buffered".to_string();
            conversation.input_state = state;
            let prompt = rendered(build_inline_ready_prompt_lines(&conversation, availability));
            assert!(
                prompt.contains(expected),
                "expected `{expected}` in `{prompt}`"
            );
        }

        assert!(!parallel_loading_prompt_indicator_frame().is_empty());
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
        conversation.input_buffer = "buffered".to_string();

        let tail = render_tail(&app, Some("review changed"));

        assert!(tail.contains("runtime:"));
        assert!(tail.contains("warning one"));
        assert!(tail.contains("notice:"));
        assert!(tail.contains("review changed"));
        assert!(tail.contains("buffered prompt"));

        assert_eq!(ready_conversation(&app).thread_id, "thread-1");
    }
}
