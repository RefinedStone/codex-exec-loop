use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};

use super::super::planning::{
    build_planner_panel_lines, build_planning_notice_line, build_planning_summary_line,
};
use super::{
    ConversationInputState, ConversationState, ConversationViewModel,
    INLINE_LIVE_AGENT_DETAIL_LIMIT, INLINE_LIVE_AGENT_MAX_CONTENT_LINES,
    INLINE_TAIL_AUTO_FOLLOW_DETAIL_LIMIT, INLINE_TAIL_NOTICE_DETAIL_LIMIT,
    INLINE_TAIL_PLANNING_DETAIL_LIMIT, INLINE_TAIL_RUNTIME_NOTICE_DETAIL_LIMIT,
    INLINE_TAIL_STATUS_DETAIL_LIMIT, INLINE_TAIL_THREAD_LABEL_LIMIT,
    INLINE_TAIL_WARNING_DETAIL_LIMIT, NativeTuiApp, ShellActionAvailability,
    ShellConversationState, ShellCorePresentationContext, StartupState,
    auto_follow_prompt_status_line, build_prompt_cursor_offset, build_working_line,
    compact_inline_detail, inline_input_state_label, turn_status_label, wrapped_row_count,
};
use crate::adapter::inbound::tui::conversation_text::conversation_message_kind_label;
use crate::application::service::planning::PlanningRuntimeSnapshot;

#[cfg(test)]
use super::{
    FOOTER_AUTO_FOLLOW_DETAIL_LIMIT, FOOTER_NOTICE_DETAIL_LIMIT,
    FOOTER_RUNTIME_NOTICE_DETAIL_LIMIT, FOOTER_STATUS_DETAIL_LIMIT, FOOTER_WARNING_DETAIL_LIMIT,
    INLINE_TAIL_TEMPLATE_LABEL_LIMIT,
};

#[derive(Clone)]
pub(crate) struct InlineTailView {
    pub(crate) lines: Vec<Line<'static>>,
    pub(crate) prompt_cursor_offset: Option<(u16, u16)>,
    pub(crate) render_from_top: bool,
}

#[derive(Clone, Copy)]
pub(super) struct PlanModeIndicatorView {
    primary_label: &'static str,
    detail_label: Option<&'static str>,
    color: Color,
}

#[cfg(test)]
pub(super) fn build_shell_footer_lines_with_context(
    context: &ShellCorePresentationContext<'_>,
    plan_mode_indicator: PlanModeIndicatorView,
    github_review_recent_changes_summary: Option<String>,
    planning_summary_line: Option<String>,
    planning_notice_line: Option<String>,
    planner_panel_lines: Vec<String>,
) -> Vec<Line<'static>> {
    match context.conversation_state {
        ShellConversationState::Loading => vec![
            Line::from(plan_mode_prefixed_spans(
                format!(
                    "startup: {}  |  sessions: {}  |  github: {}",
                    context.shell_action_availability.status_text(),
                    context.recent_session_status_label.as_str(),
                    context.github_review_polling_status_label.as_str(),
                ),
                plan_mode_indicator,
            )),
            Line::from("current state: waiting"),
            Line::from("cause: thread history is still loading from codex app-server"),
            Line::from("next action: wait for the thread history to load"),
        ],
        ShellConversationState::Failed(message) => vec![
            Line::from(plan_mode_prefixed_spans(
                format!(
                    "startup: {}  |  sessions: {}  |  github: {}",
                    context.shell_action_availability.status_text(),
                    context.recent_session_status_label.as_str(),
                    context.github_review_polling_status_label.as_str(),
                ),
                plan_mode_indicator,
            )),
            Line::from("current state: blocked"),
            Line::from("cause: thread history is unavailable because loading failed"),
            Line::from("next action: reload the session or open a new draft"),
            Line::from(format!(
                "conversation error: {}",
                compact_inline_detail(message, FOOTER_STATUS_DETAIL_LIMIT)
            )),
        ],
        ShellConversationState::Ready(conversation) => {
            let warning_summary = conversation.warning_summary(FOOTER_WARNING_DETAIL_LIMIT);
            let runtime_notice_summary =
                conversation.runtime_notice_summary(FOOTER_RUNTIME_NOTICE_DETAIL_LIMIT);
            let mut lines = vec![
                Line::from(plan_mode_prefixed_spans(
                    format!(
                        "thread: {}  |  turn: {}  |  input: {}",
                        inline_thread_label(conversation),
                        turn_status_label(conversation),
                        conversation.input_state.label(),
                    ),
                    plan_mode_indicator,
                )),
                Line::from(format!(
                    "startup: {}  |  gh: {}  |  auto: {}  |  progress: {}  |  mode: {}",
                    context.shell_action_availability.status_text(),
                    context.github_review_polling_status_label.as_str(),
                    auto_follow_status_summary(conversation, FOOTER_AUTO_FOLLOW_DETAIL_LIMIT),
                    conversation
                        .auto_follow_state
                        .compact_completed_progress_label(),
                    inline_mode_label(conversation),
                )),
            ];

            let mut status_segments = vec![format!(
                "operator status: {}",
                compact_inline_detail(&conversation.status_text, FOOTER_STATUS_DETAIL_LIMIT)
            )];
            if warning_summary != "clear" {
                status_segments.push(compact_inline_detail(
                    &warning_summary,
                    FOOTER_WARNING_DETAIL_LIMIT,
                ));
            }
            if let Some(runtime_notice_summary) = runtime_notice_summary.as_deref() {
                status_segments.push(compact_inline_detail(
                    runtime_notice_summary,
                    FOOTER_RUNTIME_NOTICE_DETAIL_LIMIT,
                ));
            } else if warning_summary == "clear" {
                status_segments.push(format!(
                    "sessions: {}",
                    context.recent_session_status_label.as_str()
                ));
            }
            lines.push(Line::from(status_segments.join("  |  ")));
            if let Some(working_line) = build_working_line(conversation, FOOTER_STATUS_DETAIL_LIMIT)
            {
                lines.push(working_line);
            }

            if let Some(planning_line) = planning_summary_line {
                lines.push(Line::from(planning_line));
            }
            if let Some(planning_notice_line) = planning_notice_line {
                lines.push(Line::from(planning_notice_line));
            }
            lines.extend(planner_panel_lines.into_iter().map(Line::from));

            if let Some(notice_line) = build_operator_notice_line(
                github_review_recent_changes_summary.as_deref(),
                conversation,
                FOOTER_NOTICE_DETAIL_LIMIT,
            ) {
                lines.push(Line::from(format!("operator notice: {notice_line}")));
            }

            lines
        }
    }
}

pub(crate) fn build_inline_tail_view(app: &NativeTuiApp, content_width: u16) -> InlineTailView {
    let context = ShellCorePresentationContext::from_app(app);
    let lines = build_inline_tail_lines_with_context(
        app,
        &context,
        app.github_review_recent_changes_summary(INLINE_TAIL_NOTICE_DETAIL_LIMIT),
    );
    let prompt_cursor_offset =
        build_inline_prompt_cursor_offset_for_lines(app, &context, content_width, &lines);

    InlineTailView {
        lines,
        prompt_cursor_offset,
        render_from_top: context.startup_screen_is_active(),
    }
}

#[cfg(test)]
pub(super) fn build_inline_tail_lines(app: &NativeTuiApp) -> Vec<Line<'static>> {
    build_inline_tail_view(app, 0).lines
}

fn build_conversation_loading_operator_lines() -> Vec<Line<'static>> {
    vec![
        Line::from("current state: waiting"),
        Line::from("cause: thread history is still loading from codex app-server"),
        Line::from("next action: wait for the thread history to load"),
    ]
}

fn build_conversation_failed_operator_lines(
    message: &str,
    max_detail_len: usize,
) -> Vec<Line<'static>> {
    vec![
        Line::from("current state: blocked"),
        Line::from("cause: thread history is unavailable because loading failed"),
        Line::from("next action: reload the session or open a new draft"),
        Line::from(format!(
            "conversation error: {}",
            compact_inline_detail(message, max_detail_len)
        )),
    ]
}

fn build_inline_tail_lines_with_context(
    app: &NativeTuiApp,
    context: &ShellCorePresentationContext<'_>,
    github_review_recent_changes_summary: Option<String>,
) -> Vec<Line<'static>> {
    let plan_mode_indicator = current_plan_mode_indicator(app);
    let planning_summary_line = context.ready_conversation().and_then(|conversation| {
        build_planning_summary_line(app, conversation, INLINE_TAIL_PLANNING_DETAIL_LIMIT, false)
    });
    let planning_notice_line = context.ready_conversation().and_then(|conversation| {
        build_planning_notice_line(conversation, INLINE_TAIL_NOTICE_DETAIL_LIMIT)
    });
    let planner_panel_lines = build_planner_panel_lines(app, INLINE_TAIL_NOTICE_DETAIL_LIMIT);

    if context.startup_screen_is_active() {
        let mut lines = build_inline_startup_screen_lines_with_context(context);
        lines.extend(build_inline_tail_prompt_lines_with_context(
            context,
            app.shell_action_availability(),
        ));
        return lines;
    }

    let mut lines = Vec::new();

    match context.conversation_state {
        ShellConversationState::Loading => {
            lines.push(Line::from(plan_mode_prefixed_spans(
                format!(
                    "thread: waiting  |  startup: {}  |  sessions: {}",
                    context.shell_action_availability.status_text(),
                    context.recent_session_status_label.as_str(),
                ),
                plan_mode_indicator,
            )));
            lines.push(Line::from(format!(
                "github: {}  |  flow: terminal main buffer",
                context.github_review_polling_status_label.as_str(),
            )));
            lines.extend(build_conversation_loading_operator_lines());
        }
        ShellConversationState::Failed(message) => {
            lines.push(Line::from(plan_mode_prefixed_spans(
                format!(
                    "thread: blocked  |  startup: {}  |  sessions: {}",
                    context.shell_action_availability.status_text(),
                    context.recent_session_status_label.as_str(),
                ),
                plan_mode_indicator,
            )));
            lines.push(Line::from(format!(
                "github: {}  |  flow: terminal main buffer",
                context.github_review_polling_status_label.as_str(),
            )));
            lines.extend(build_conversation_failed_operator_lines(
                message,
                INLINE_TAIL_STATUS_DETAIL_LIMIT,
            ));
        }
        ShellConversationState::Ready(conversation) => {
            let warning_summary = compact_inline_summary_label(
                &conversation.warning_summary(INLINE_TAIL_WARNING_DETAIL_LIMIT),
            );
            let runtime_notice_summary = conversation
                .runtime_notice_summary(INLINE_TAIL_RUNTIME_NOTICE_DETAIL_LIMIT)
                .map(|summary| compact_inline_summary_label(&summary));

            lines.push(Line::from(plan_mode_prefixed_spans(
                format!(
                    "thread: {}  |  turn: {}  |  auto: {}  |  done: {}  |  in: {}",
                    inline_thread_label(conversation),
                    turn_status_label(conversation),
                    inline_auto_follow_status_summary(
                        conversation,
                        INLINE_TAIL_AUTO_FOLLOW_DETAIL_LIMIT,
                    ),
                    conversation.auto_follow_state.progress_label(),
                    inline_input_state_label(conversation.input_state),
                ),
                plan_mode_indicator,
            )));
            let mut status_segments = vec![format!(
                "operator status: {}",
                compact_inline_detail(&conversation.status_text, INLINE_TAIL_STATUS_DETAIL_LIMIT)
            )];
            if warning_summary != "clear" {
                status_segments.push(warning_summary);
            }
            if let Some(runtime_notice_summary) = runtime_notice_summary.as_deref() {
                status_segments.push(runtime_notice_summary.to_string());
            } else {
                status_segments.push(format!(
                    "startup: {}",
                    context.shell_action_availability.status_text()
                ));
                status_segments.push(format!(
                    "gh: {}",
                    context.github_review_polling_status_label.as_str()
                ));
            }
            lines.push(Line::from(status_segments.join("  |  ")));
            if let Some(working_line) =
                build_working_line(conversation, INLINE_TAIL_STATUS_DETAIL_LIMIT)
            {
                lines.push(working_line);
            }
            if let Some(planning_line) = planning_summary_line {
                lines.push(Line::from(planning_line));
            }
            if let Some(planning_notice_line) = planning_notice_line {
                lines.push(Line::from(planning_notice_line));
            }
            lines.extend(planner_panel_lines.into_iter().map(Line::from));

            if let Some(live_agent_lines) = current_live_agent_lines(conversation) {
                lines.extend(live_agent_lines);
            } else if let Some(notice_line) = build_operator_notice_line(
                github_review_recent_changes_summary.as_deref(),
                conversation,
                INLINE_TAIL_NOTICE_DETAIL_LIMIT,
            ) {
                lines.push(Line::from(format!("operator notice: {notice_line}")));
            }
        }
    }

    lines.extend(build_inline_tail_prompt_lines_with_context(
        context,
        app.shell_action_availability(),
    ));
    lines
}

fn build_inline_startup_screen_lines_with_context(
    context: &ShellCorePresentationContext<'_>,
) -> Vec<Line<'static>> {
    let mut lines = vec![Line::from(format!(
        "startup: {}  |  sessions: {}  |  gh: {}",
        context.shell_action_availability.status_text(),
        context.recent_session_status_label.as_str(),
        context.github_review_polling_status_label.as_str(),
    ))];

    match context.startup_state {
        StartupState::Idle => {
            lines.extend(super::build_startup_operator_lines_from_state(
                context.startup_state,
                INLINE_TAIL_NOTICE_DETAIL_LIMIT,
            ));
            if let Some(conversation) = context.ready_conversation() {
                lines.push(Line::from(format!("workspace: {}", conversation.cwd)));
            }
        }
        StartupState::Loading => {
            lines.extend(super::build_startup_operator_lines_from_state(
                context.startup_state,
                INLINE_TAIL_NOTICE_DETAIL_LIMIT,
            ));
        }
        StartupState::Ready(diagnostics) => {
            lines.extend(super::build_startup_operator_lines_from_state(
                context.startup_state,
                INLINE_TAIL_NOTICE_DETAIL_LIMIT,
            ));
            lines.push(Line::from(format!("workspace: {}", diagnostics.cwd)));
            lines.push(Line::from(super::build_startup_check_summary_line(
                diagnostics,
            )));
            if let Some(first_warning) = diagnostics.warnings.first() {
                lines.push(Line::from(format!(
                    "warning: {}",
                    compact_inline_detail(first_warning, INLINE_TAIL_NOTICE_DETAIL_LIMIT)
                )));
            }
            lines.push(Line::from("conversation"));
            lines.push(Line::from(
                "first reply appears here after you send the opening prompt",
            ));
            lines.push(Line::from(format!(
                "starter: {}",
                inline_starter_copy_in_context(context)
            )));
        }
        StartupState::Failed(message) => {
            let _ = message;
            lines.extend(super::build_startup_operator_lines_from_state(
                context.startup_state,
                INLINE_TAIL_NOTICE_DETAIL_LIMIT,
            ));
            for warning_line in super::build_startup_warning_lines_from_state(context.startup_state)
                .into_iter()
                .filter(|line| !line.to_string().eq_ignore_ascii_case("no warnings"))
            {
                lines.push(Line::from(format!(
                    "warning: {}",
                    compact_inline_detail(
                        &warning_line.to_string(),
                        INLINE_TAIL_NOTICE_DETAIL_LIMIT
                    )
                )));
            }
        }
    }

    lines.push(Line::from(""));
    lines
}

pub(super) fn current_plan_mode_indicator(app: &NativeTuiApp) -> PlanModeIndicatorView {
    match &app.conversation_state {
        ConversationState::Ready(conversation) => {
            plan_mode_indicator_from_snapshot(&conversation.planning_runtime_snapshot)
        }
        ConversationState::Loading | ConversationState::Failed(_) => {
            let workspace_directory = app.current_workspace_directory();
            let snapshot = app.load_planning_runtime_snapshot(&workspace_directory);
            plan_mode_indicator_from_snapshot(&snapshot)
        }
    }
}

fn plan_mode_indicator_from_snapshot(snapshot: &PlanningRuntimeSnapshot) -> PlanModeIndicatorView {
    if !snapshot.plan_enabled() {
        return PlanModeIndicatorView {
            primary_label: "Plan off",
            detail_label: None,
            color: Color::Red,
        };
    }

    PlanModeIndicatorView {
        primary_label: "Plan on",
        detail_label: Some(plan_runtime_substate_label(snapshot)),
        color: Color::Blue,
    }
}

pub(super) fn plan_runtime_substate_label(snapshot: &PlanningRuntimeSnapshot) -> &'static str {
    snapshot.preview_status_label()
}

fn plan_mode_prefixed_spans(
    leading_text: String,
    indicator: PlanModeIndicatorView,
) -> Vec<Span<'static>> {
    let mut spans = vec![Span::raw(leading_text), Span::raw("  |  ")];
    spans.push(Span::styled(
        indicator.primary_label,
        Style::default().fg(indicator.color),
    ));
    if let Some(detail_label) = indicator.detail_label {
        spans.push(Span::raw(format!(" / {detail_label}")));
    }
    spans
}

fn inline_starter_copy_in_context(context: &ShellCorePresentationContext<'_>) -> &'static str {
    let Some(conversation) = context.ready_conversation() else {
        return "start with a task, file path, or bug summary";
    };

    if conversation.input_buffer.trim().is_empty() {
        "start with a task, file path, or bug summary"
    } else {
        "opening prompt buffered below"
    }
}

fn build_inline_prompt_cursor_offset_for_lines(
    app: &NativeTuiApp,
    context: &ShellCorePresentationContext<'_>,
    content_width: u16,
    tail_lines: &[Line<'static>],
) -> Option<(u16, u16)> {
    let ShellConversationState::Ready(conversation) = context.conversation_state else {
        return None;
    };
    let prompt_lines =
        build_inline_tail_prompt_lines_with_context(context, app.shell_action_availability());
    let prompt_start_index = tail_lines.len().saturating_sub(prompt_lines.len());
    let prompt_start_row = tail_lines[..prompt_start_index]
        .iter()
        .map(|line| wrapped_row_count(line.width(), content_width))
        .sum::<usize>() as u16;
    let (cursor_x, cursor_y) = build_prompt_cursor_offset(conversation, content_width)?;

    Some((cursor_x, prompt_start_row.saturating_add(cursor_y)))
}

fn build_inline_tail_prompt_lines_with_context(
    context: &ShellCorePresentationContext<'_>,
    shell_action_availability: ShellActionAvailability,
) -> Vec<Line<'static>> {
    match context.conversation_state {
        ShellConversationState::Loading => {
            vec![Line::from(
                "operator prompt: waiting while thread history loads",
            )]
        }
        ShellConversationState::Failed(_) => vec![Line::from(
            "operator prompt: blocked until you reload the session or open a new draft",
        )],
        ShellConversationState::Ready(conversation) => {
            build_inline_ready_prompt_lines(conversation, shell_action_availability)
        }
    }
}

fn build_inline_ready_prompt_lines(
    conversation: &ConversationViewModel,
    shell_action_availability: ShellActionAvailability,
) -> Vec<Line<'static>> {
    let prompt_buffer = super::build_prompt_buffer_view(conversation);
    let mut lines = prompt_buffer.lines;

    if conversation.input_buffer.is_empty() {
        if let Some(status_line) = auto_follow_prompt_status_line(conversation, true) {
            lines.push(Line::from(status_line));
            return lines;
        }
        let line = match (conversation.input_state, shell_action_availability) {
            (_, ShellActionAvailability::Pending) if conversation.input_state.can_submit_now() => {
                "operator prompt: waiting for startup  |  type now, Enter sends when ready"
                    .to_string()
            }
            (_, ShellActionAvailability::Blocked) if conversation.input_state.can_submit_now() => {
                "operator prompt: blocked while startup checks need attention  |  Ctrl+d inspects"
                    .to_string()
            }
            (ConversationInputState::DraftReady, _) => {
                "operator prompt: new thread ready  |  Enter sends  |  Ctrl+j newline  |  :help"
                    .to_string()
            }
            (ConversationInputState::ReadyToContinue, _) => {
                "operator prompt: session ready  |  Enter sends  |  Ctrl+j newline  |  :help"
                    .to_string()
            }
            (ConversationInputState::SubmittingTurn, _) => {
                "operator prompt: sending  |  wait for turn start".to_string()
            }
            (ConversationInputState::StreamingTurn, _) => {
                "operator prompt: turn running  |  type now, Enter sends when idle".to_string()
            }
        };
        lines.push(Line::from(line));
        return lines;
    }

    if conversation.inline_shell_command_palette_state.is_active() {
        lines.extend(super::build_shell_command_palette_lines(conversation));
        return lines;
    }

    if let Some(command) = super::InlineShellCommandInput::parse(&conversation.input_buffer) {
        lines.push(Line::from(command.buffered_hint()));
        return lines;
    }

    if conversation.auto_follow_state.has_live_activity()
        && conversation.input_state.can_submit_now()
    {
        lines.push(Line::from(
            "operator prompt: buffered  |  automation busy  |  Enter sends when idle",
        ));
        return lines;
    }

    let hint = match (conversation.input_state, shell_action_availability) {
        (
            ConversationInputState::DraftReady | ConversationInputState::ReadyToContinue,
            ShellActionAvailability::Pending,
        ) if conversation.startup_submit_armed => {
            "queued until startup is ready  |  editing cancels the queued send"
        }
        (
            ConversationInputState::DraftReady | ConversationInputState::ReadyToContinue,
            ShellActionAvailability::Ready,
        ) => "operator prompt: buffered  |  Enter sends  |  Ctrl+j newline",
        (ConversationInputState::DraftReady | ConversationInputState::ReadyToContinue, _) => {
            "operator prompt: buffered  |  Enter sends when ready  |  Ctrl+j newline"
        }
        (ConversationInputState::SubmittingTurn | ConversationInputState::StreamingTurn, _) => {
            "operator prompt: buffered  |  Enter sends when idle  |  Ctrl+j newline"
        }
    };
    lines.push(Line::from(hint));
    lines
}

pub(super) fn current_live_agent_lines(
    conversation: &ConversationViewModel,
) -> Option<Vec<Line<'static>>> {
    let message = conversation.live_agent_message.as_ref()?;
    let label = conversation_message_kind_label(message.kind, message.phase.as_deref());
    let content_lines = message.text.split('\n').collect::<Vec<_>>();
    let start_index = content_lines
        .len()
        .saturating_sub(INLINE_LIVE_AGENT_MAX_CONTENT_LINES);
    let mut lines = vec![Line::from(format!("live: {label}"))];

    for line in content_lines.into_iter().skip(start_index) {
        lines.push(Line::from(format!(
            "  {}",
            compact_live_agent_line(line, INLINE_LIVE_AGENT_DETAIL_LIMIT)
        )));
    }

    Some(lines)
}

fn inline_thread_label(conversation: &ConversationViewModel) -> String {
    if !conversation.has_active_thread() {
        return "new draft".to_string();
    }

    compact_inline_detail(&conversation.title, INLINE_TAIL_THREAD_LABEL_LIMIT)
}

#[cfg(test)]
fn inline_mode_label(conversation: &ConversationViewModel) -> String {
    compact_inline_detail(
        conversation.auto_follow_state.mode_label(),
        INLINE_TAIL_TEMPLATE_LABEL_LIMIT,
    )
}

fn compact_inline_summary_label(summary: &str) -> String {
    compact_inline_detail(
        &summary
            .replace("runtime warning:", "rt warn:")
            .replace("runtime warnings", "rt warns")
            .replace("warning:", "warn:")
            .replace("warnings:", "warn:")
            .replace("runtime notices", "notices")
            .replace("runtime:", "notice:"),
        INLINE_TAIL_WARNING_DETAIL_LIMIT,
    )
}

fn compact_live_agent_line(text: &str, max_len: usize) -> String {
    let rendered = text.replace('\t', "    ");
    if rendered.chars().count() <= max_len {
        return rendered;
    }

    let keep = max_len.saturating_sub(3);
    let truncated = rendered.chars().take(keep).collect::<String>();
    format!("{truncated}...")
}

fn build_operator_notice_line(
    github_review_recent_changes_summary: Option<&str>,
    conversation: &ConversationViewModel,
    max_detail_len: usize,
) -> Option<String> {
    if let Some(github_review_summary) = github_review_recent_changes_summary {
        return Some(format!(
            "gh update: {}",
            compact_inline_detail(github_review_summary, max_detail_len)
        ));
    }

    let turn_running = conversation.has_running_turn();
    let activity_scope = conversation
        .turn_activity
        .activity_scope_label(turn_running);
    let activity_summary = conversation.turn_activity.activity_summary(turn_running);
    let activity_command_count = conversation
        .turn_activity
        .activity_command_count(turn_running);
    let activity_file_change_count = conversation
        .turn_activity
        .activity_file_change_count(turn_running);
    let has_tool_activity = (activity_summary != "idle" && activity_summary != "none")
        || activity_command_count > 0
        || activity_file_change_count > 0;
    if turn_running && has_tool_activity {
        let mut notice_line = format!(
            "tool activity: {}  |  {activity_scope} commands: {}  |  {activity_scope} file changes: {}",
            compact_inline_detail(activity_summary, max_detail_len),
            activity_command_count,
            activity_file_change_count,
        );
        if let Some(approval_summary) = conversation.approval_summary().as_deref() {
            notice_line.push_str(&format!(
                "  |  approval: {}",
                compact_inline_detail(approval_summary, max_detail_len)
            ));
        }
        return Some(notice_line);
    }

    if let Some(activity) = conversation.last_auto_followup_activity.as_ref() {
        return Some(format!(
            "automation update: {}  |  operator action: {}",
            activity.summary,
            compact_inline_detail(&activity.detail, max_detail_len)
        ));
    }

    if has_tool_activity {
        let mut notice_line = format!(
            "tool activity: {}  |  {activity_scope} commands: {}  |  {activity_scope} file changes: {}",
            compact_inline_detail(activity_summary, max_detail_len),
            activity_command_count,
            activity_file_change_count,
        );
        if let Some(approval_summary) = conversation.approval_summary().as_deref() {
            notice_line.push_str(&format!(
                "  |  approval: {}",
                compact_inline_detail(approval_summary, max_detail_len)
            ));
        }
        return Some(notice_line);
    }

    conversation.approval_summary().map(|approval_summary| {
        format!(
            "approval: {}",
            compact_inline_detail(&approval_summary, max_detail_len)
        )
    })
}

#[cfg(test)]
fn auto_follow_status_summary(
    conversation: &ConversationViewModel,
    max_detail_len: usize,
) -> String {
    let summary = if conversation.auto_follow_state.enabled {
        format!(
            "{} / {}",
            conversation.auto_follow_state.status_label(),
            conversation.auto_follow_state.activity_label()
        )
    } else {
        conversation.auto_follow_state.status_label().to_string()
    };
    compact_inline_detail(&summary, max_detail_len)
}

fn inline_auto_follow_status_summary(
    conversation: &ConversationViewModel,
    max_detail_len: usize,
) -> String {
    let summary = if conversation.auto_follow_state.enabled {
        format!(
            "{}/{}",
            conversation.auto_follow_state.status_label(),
            conversation.auto_follow_state.activity_label()
        )
    } else {
        conversation.auto_follow_state.status_label().to_string()
    };
    compact_inline_detail(&summary, max_detail_len)
}
