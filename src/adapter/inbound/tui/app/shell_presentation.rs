use std::time::{Duration, Instant};

pub(super) use super::planning::{
    build_followup_template_preview_lines, build_followup_template_status_lines,
};
use super::planning::{
    build_planner_panel_lines, build_planning_notice_line, build_planning_summary_line,
};
use super::*;
use crate::adapter::inbound::tui::conversation_text::{
    conversation_message_kind_label, conversation_message_label,
};
use crate::application::service::planning::{
    PlanningRuntimeSnapshot, PlanningRuntimeWorkspaceStatus,
};
use crate::application::service::session_service::{
    SessionBrowserView, SessionProjectFilter, build_session_browser_view,
};
use crate::domain::followup_template::FollowupTemplateDefinition;
use crate::domain::planning::{
    PlanningValidationSeverity, PriorityQueueSkippedTask, PriorityQueueTask,
};
use crate::domain::text::compact_whitespace_detail;

#[cfg(test)]
const FOOTER_WARNING_DETAIL_LIMIT: usize = 48;
#[cfg(test)]
const FOOTER_RUNTIME_NOTICE_DETAIL_LIMIT: usize = 48;
#[cfg(test)]
const FOOTER_STATUS_DETAIL_LIMIT: usize = 72;
const FOOTER_NOTICE_DETAIL_LIMIT: usize = 56;
#[cfg(test)]
const FOOTER_PLANNING_DETAIL_LIMIT: usize = 56;
#[cfg(test)]
const FOOTER_AUTO_FOLLOW_DETAIL_LIMIT: usize = 28;
const INLINE_LIVE_AGENT_DETAIL_LIMIT: usize = 72;
const INLINE_LIVE_AGENT_MAX_CONTENT_LINES: usize = 2;
const INLINE_TAIL_THREAD_LABEL_LIMIT: usize = 20;
#[cfg(test)]
const INLINE_TAIL_TEMPLATE_LABEL_LIMIT: usize = 16;
const INLINE_TAIL_STATUS_DETAIL_LIMIT: usize = 44;
const INLINE_TAIL_NOTICE_DETAIL_LIMIT: usize = 40;
const INLINE_TAIL_WARNING_DETAIL_LIMIT: usize = 24;
const INLINE_TAIL_RUNTIME_NOTICE_DETAIL_LIMIT: usize = 24;
const INLINE_TAIL_PLANNING_DETAIL_LIMIT: usize = 36;
const INLINE_TAIL_AUTO_FOLLOW_DETAIL_LIMIT: usize = 18;
const INLINE_COMMAND_PALETTE_VISIBLE_LIMIT: usize = 4;
const QUEUE_INSPECTION_TASK_LIMIT: usize = 2;
const QUEUE_INSPECTION_PROPOSAL_LIMIT: usize = 1;
const QUEUE_INSPECTION_TITLE_DETAIL_LIMIT: usize = 56;
const QUEUE_INSPECTION_NOTE_DETAIL_LIMIT: usize = 56;
const PROMPT_PRIMARY_PREFIX: &str = "> ";
const PROMPT_CONTINUATION_PREFIX: &str = "  ";
const STARTUP_ASCII_ART_DEFAULT: &str = r#"
.::::::.::::::.::::::.::::::.::::::.::::::.::::::.::::::

.::::::.::::::.::::::.::::::.::::::.::::::.::::::.::::::



      .:       .::
     .: ::     .::
    .:  .::    .::  .::.: .:::   .::
   .::   .::   .:: .::  .::    .::  .::
  .:::::: .::  .:.::    .::   .::   .::
 .::       .:: .:: .::  .::   .::   .::
.::         .::.::  .::.:::     .:: .:::

    .::::
  .::    .::
.::       .::.::  .::   .::    .::  .::   .::
.::       .::.::  .:: .:   .:: .::  .:: .:   .::
.::       .::.::  .::.::::: .::.::  .::.::::: .::
  .:: .: .:: .::  .::.:        .::  .::.:
    .:: ::     .::.::  .::::     .::.::  .::::
         .:


.::::::.::::::.::::::.::::::.::::::.::::::.::::::.::::::

.::::::.::::::.::::::.::::::.::::::.::::::.::::::.::::::
"#;

#[cfg(test)]
pub(super) struct ConversationShellView {
    pub(super) shell_title: Line<'static>,
    pub(super) header_lines: Vec<Line<'static>>,
    pub(super) conversation_lines: Vec<Line<'static>>,
    pub(super) status_title: Line<'static>,
    pub(super) footer_lines: Vec<Line<'static>>,
    pub(super) input_title: Line<'static>,
    pub(super) input_lines: Vec<Line<'static>>,
}

#[cfg(test)]
#[allow(dead_code)]
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

#[cfg(test)]
pub(super) struct TranscriptPanelView {
    pub(super) title: Line<'static>,
    pub(super) lines: Vec<Line<'static>>,
    pub(super) scroll_offset: u16,
}

struct PromptBufferView {
    lines: Vec<Line<'static>>,
    cursor_line_index: usize,
    cursor_column: usize,
}

#[derive(Clone, Copy)]
struct PlanModeIndicatorView {
    primary_label: &'static str,
    detail_label: Option<&'static str>,
    color: Color,
}

#[derive(Clone, Copy)]
enum ShellConversationState<'a> {
    Loading,
    Failed(&'a str),
    Ready(&'a ConversationViewModel),
}

struct ShellCorePresentationContext<'a> {
    show_startup_ascii_art: bool,
    startup_state: &'a StartupState,
    shell_action_availability: ShellActionAvailability,
    recent_session_status_label: String,
    github_review_polling_status_label: String,
    #[cfg(test)]
    planner_shows_debug_details: bool,
    conversation_state: ShellConversationState<'a>,
}

impl<'a> ShellCorePresentationContext<'a> {
    fn from_app(app: &'a NativeTuiApp) -> Self {
        Self {
            show_startup_ascii_art: app.show_startup_ascii_art,
            startup_state: &app.startup_state,
            shell_action_availability: app.shell_action_availability(),
            recent_session_status_label: recent_session_status_label(app),
            github_review_polling_status_label: app.github_review_polling_status_label(),
            #[cfg(test)]
            planner_shows_debug_details: app.planner_shows_debug_details(),
            conversation_state: match &app.conversation_state {
                ConversationState::Loading => ShellConversationState::Loading,
                ConversationState::Failed(message) => ShellConversationState::Failed(message),
                ConversationState::Ready(conversation) => {
                    ShellConversationState::Ready(conversation)
                }
            },
        }
    }

    fn ready_conversation(&self) -> Option<&'a ConversationViewModel> {
        match self.conversation_state {
            ShellConversationState::Ready(conversation) => Some(conversation),
            ShellConversationState::Loading | ShellConversationState::Failed(_) => None,
        }
    }
}

include!("shell_presentation/overlays.rs");

fn current_live_agent_lines(conversation: &ConversationViewModel) -> Option<Vec<Line<'static>>> {
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

#[cfg(test)]
fn build_conversation_lines_with_context(
    context: &ShellCorePresentationContext<'_>,
) -> Vec<Line<'static>> {
    if let Some(startup_banner_lines) = build_startup_banner_lines_from_context(context, None) {
        return startup_banner_lines;
    }

    match context.conversation_state {
        ShellConversationState::Loading => vec![Line::from("Loading thread history...")],
        ShellConversationState::Failed(message) => vec![Line::from(message.to_string())],
        ShellConversationState::Ready(conversation) => {
            if context.planner_shows_debug_details {
                format_conversation_lines_with_debug(&conversation.messages, true)
            } else {
                conversation.cached_conversation_lines.clone()
            }
        }
    }
}

#[cfg(test)]
fn build_startup_banner_lines_from_context(
    context: &ShellCorePresentationContext<'_>,
    max_height: Option<u16>,
) -> Option<Vec<Line<'static>>> {
    if !startup_banner_is_active_in_context(context) {
        return None;
    }

    let max_height = match max_height {
        Some(0) => return None,
        value => value,
    };

    Some(startup_ascii_art_lines(max_height))
}

fn startup_ascii_art_lines(max_height: Option<u16>) -> Vec<Line<'static>> {
    let mut art_lines = STARTUP_ASCII_ART_DEFAULT.lines().collect::<Vec<_>>();
    let start = art_lines
        .iter()
        .position(|line| !line.trim().is_empty())
        .unwrap_or(0);
    let end = art_lines
        .iter()
        .rposition(|line| !line.trim().is_empty())
        .map(|index| index + 1)
        .unwrap_or(art_lines.len());
    art_lines = art_lines[start..end].to_vec();

    if let Some(max_height) = max_height {
        let max_height = max_height as usize;
        if max_height > 0 && art_lines.len() > max_height {
            let start = art_lines.len().saturating_sub(max_height) / 2;
            art_lines = art_lines[start..start + max_height].to_vec();
        }
    }

    art_lines
        .into_iter()
        .map(|line| Line::from(line.to_string()))
        .collect()
}

pub(super) fn format_conversation_lines(messages: &[ConversationMessage]) -> Vec<Line<'static>> {
    format_conversation_lines_with_debug(messages, false)
}

pub(super) fn format_conversation_lines_with_debug(
    messages: &[ConversationMessage],
    show_debug_details: bool,
) -> Vec<Line<'static>> {
    let mut lines = Vec::new();

    for message in messages {
        let label = conversation_message_label(message);
        lines.push(Line::from(Span::styled(
            format!("{label}:"),
            label_style(message.kind),
        )));
        for text_line in message.text.lines() {
            lines.push(Line::from(format!("  {text_line}")));
        }
        if show_debug_details && let Some(debug_detail) = message.debug_detail.as_deref() {
            for detail_line in debug_detail.lines() {
                lines.push(Line::from(Span::styled(
                    format!("  {detail_line}"),
                    Style::default().fg(Color::Gray),
                )));
            }
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

#[cfg(test)]
pub(super) fn build_shell_footer_lines(app: &NativeTuiApp) -> Vec<Line<'static>> {
    let context = ShellCorePresentationContext::from_app(app);
    let plan_mode_indicator = current_plan_mode_indicator(app);
    let planning_summary_line = context.ready_conversation().and_then(|conversation| {
        build_planning_summary_line(app, conversation, FOOTER_PLANNING_DETAIL_LIMIT, false)
    });
    let planning_notice_line = context.ready_conversation().and_then(|conversation| {
        build_planning_notice_line(conversation, FOOTER_NOTICE_DETAIL_LIMIT)
    });
    let planner_panel_lines = build_planner_panel_lines(app, FOOTER_NOTICE_DETAIL_LIMIT);

    build_shell_footer_lines_with_context(
        &context,
        plan_mode_indicator,
        app.github_review_recent_changes_summary(FOOTER_NOTICE_DETAIL_LIMIT),
        planning_summary_line,
        planning_notice_line,
        planner_panel_lines,
    )
}

#[cfg(test)]
fn build_shell_footer_lines_with_context(
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
            Line::from("conversation state: loading thread metadata"),
            Line::from("status: waiting for thread history from codex app-server"),
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
            Line::from("conversation state: failed"),
            Line::from(format!("status: {message}")),
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
                    "startup: {}  |  gh: {}  |  auto: {}  |  progress: {}  |  tmpl: {}",
                    context.shell_action_availability.status_text(),
                    context.github_review_polling_status_label.as_str(),
                    auto_follow_status_summary(conversation, FOOTER_AUTO_FOLLOW_DETAIL_LIMIT),
                    conversation
                        .auto_follow_state
                        .compact_completed_progress_label(),
                    inline_template_label(conversation),
                )),
            ];

            let mut status_segments = vec![format!(
                "status: {}",
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
                lines.push(Line::from(format!("notice: {notice_line}")));
            }

            lines
        }
    }
}

pub(super) fn build_inline_tail_lines(app: &NativeTuiApp) -> Vec<Line<'static>> {
    let context = ShellCorePresentationContext::from_app(app);
    let plan_mode_indicator = current_plan_mode_indicator(app);
    let planning_summary_line = context.ready_conversation().and_then(|conversation| {
        build_planning_summary_line(app, conversation, INLINE_TAIL_PLANNING_DETAIL_LIMIT, false)
    });
    let planning_notice_line = context.ready_conversation().and_then(|conversation| {
        build_planning_notice_line(conversation, INLINE_TAIL_NOTICE_DETAIL_LIMIT)
    });
    let planner_panel_lines = build_planner_panel_lines(app, INLINE_TAIL_NOTICE_DETAIL_LIMIT);

    build_inline_tail_lines_with_context(
        &context,
        plan_mode_indicator,
        app.github_review_recent_changes_summary(INLINE_TAIL_NOTICE_DETAIL_LIMIT),
        planning_summary_line,
        planning_notice_line,
        planner_panel_lines,
    )
}

fn build_inline_tail_lines_with_context(
    context: &ShellCorePresentationContext<'_>,
    plan_mode_indicator: PlanModeIndicatorView,
    github_review_recent_changes_summary: Option<String>,
    planning_summary_line: Option<String>,
    planning_notice_line: Option<String>,
    planner_panel_lines: Vec<String>,
) -> Vec<Line<'static>> {
    if startup_screen_is_active_in_context(context) {
        let mut lines = build_inline_startup_screen_lines_with_context(context);
        lines.extend(build_inline_tail_prompt_lines_with_context(context));
        return lines;
    }

    let mut lines = Vec::new();

    match context.conversation_state {
        ShellConversationState::Loading => {
            lines.push(Line::from(plan_mode_prefixed_spans(
                format!(
                    "thread: loading  |  startup: {}  |  sessions: {}",
                    context.shell_action_availability.status_text(),
                    context.recent_session_status_label.as_str(),
                ),
                plan_mode_indicator,
            )));
            lines.push(Line::from(format!(
                "github: {}  |  flow: terminal main buffer",
                context.github_review_polling_status_label.as_str(),
            )));
            lines.push(Line::from(
                "status: waiting for thread history from codex app-server",
            ));
        }
        ShellConversationState::Failed(message) => {
            lines.push(Line::from(plan_mode_prefixed_spans(
                format!(
                    "thread: unavailable  |  startup: {}  |  sessions: {}",
                    context.shell_action_availability.status_text(),
                    context.recent_session_status_label.as_str(),
                ),
                plan_mode_indicator,
            )));
            lines.push(Line::from(format!(
                "github: {}  |  flow: terminal main buffer",
                context.github_review_polling_status_label.as_str(),
            )));
            lines.push(Line::from(format!("status: {message}")));
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
                "status: {}",
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
                lines.push(Line::from(format!("notice: {notice_line}")));
            }
        }
    }

    lines.extend(build_inline_tail_prompt_lines_with_context(context));
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
            lines.push(Line::from("status: preparing startup checks"));
            if let Some(conversation) = context.ready_conversation() {
                lines.push(Line::from(format!("workspace: {}", conversation.cwd)));
            }
        }
        StartupState::Loading => {
            lines.push(Line::from("status: initializing codex shell"));
            lines.extend(build_startup_check_lines_from_state(context.startup_state));
        }
        StartupState::Ready(diagnostics) => {
            lines.push(Line::from(format!("workspace: {}", diagnostics.cwd)));
            lines.push(Line::from(format!(
                "diagnostics: codex {}  |  app-server {}  |  account {}",
                inline_diagnostic_status(diagnostics.codex_binary_ok, "ok", "check"),
                inline_diagnostic_status(diagnostics.initialize_ok, "ok", "check"),
                inline_diagnostic_status(diagnostics.account_ok, "ok", "attention"),
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
            lines.push(Line::from(format!("status: {message}")));
            for warning_line in build_startup_warning_lines_from_state(context.startup_state)
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

fn current_plan_mode_indicator(app: &NativeTuiApp) -> PlanModeIndicatorView {
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

fn plan_runtime_substate_label(snapshot: &PlanningRuntimeSnapshot) -> &'static str {
    if snapshot.workspace_status() == PlanningRuntimeWorkspaceStatus::Invalid {
        "invalid"
    } else if snapshot.auto_followup_pause_reason().is_some() {
        "paused"
    } else if snapshot.has_actionable_queue_head() {
        "ready"
    } else {
        "idle"
    }
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

fn inline_diagnostic_status(
    ok: bool,
    ready_label: &'static str,
    blocked_label: &'static str,
) -> &'static str {
    if ok { ready_label } else { blocked_label }
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

fn build_inline_tail_prompt_lines(app: &NativeTuiApp) -> Vec<Line<'static>> {
    build_inline_tail_prompt_lines_with_context(&ShellCorePresentationContext::from_app(app))
}

fn build_inline_tail_prompt_lines_with_context(
    context: &ShellCorePresentationContext<'_>,
) -> Vec<Line<'static>> {
    match context.conversation_state {
        ShellConversationState::Loading => vec![Line::from("prompt: waiting for shell readiness")],
        ShellConversationState::Failed(message) => {
            vec![Line::from(format!("prompt: unavailable  |  {message}"))]
        }
        ShellConversationState::Ready(conversation) => {
            build_inline_ready_prompt_lines(conversation, context.shell_action_availability)
        }
    }
}

fn build_inline_ready_prompt_lines(
    conversation: &ConversationViewModel,
    shell_action_availability: ShellActionAvailability,
) -> Vec<Line<'static>> {
    let prompt_buffer = build_prompt_buffer_view(conversation);
    let mut lines = prompt_buffer.lines;

    if conversation.input_buffer.is_empty() {
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
        lines.extend(build_shell_command_palette_lines(conversation));
        return lines;
    }

    if let Some(command) = InlineShellCommandInput::parse(&conversation.input_buffer) {
        lines.push(Line::from(command.buffered_hint()));
        return lines;
    }

    if conversation.auto_follow_state.has_live_activity()
        && conversation.input_state.can_submit_now()
    {
        lines.push(Line::from(
            "buffered prompt  |  auto follow-up busy  |  Enter when idle",
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

fn inline_thread_label(conversation: &ConversationViewModel) -> String {
    if !conversation.has_active_thread() {
        return "new draft".to_string();
    }

    compact_inline_detail(&conversation.title, INLINE_TAIL_THREAD_LABEL_LIMIT)
}

#[cfg(test)]
fn inline_template_label(conversation: &ConversationViewModel) -> String {
    let label = conversation.auto_follow_state.template_label();
    let compact_label = label
        .strip_prefix("builtin ")
        .or_else(|| label.strip_prefix("workspace "))
        .unwrap_or(label);
    compact_inline_detail(compact_label, INLINE_TAIL_TEMPLATE_LABEL_LIMIT)
}

fn compact_inline_summary_label(summary: &str) -> String {
    compact_inline_detail(
        &summary
            .replace("runtime warning:", "rt warn:")
            .replace("runtime warnings", "rt warns")
            .replace("template warning:", "tmpl warn:")
            .replace("template warnings", "tmpl warns")
            .replace("warning:", "warn:")
            .replace("warnings:", "warn:")
            .replace("runtime notices", "notices")
            .replace("runtime:", "notice:"),
        INLINE_TAIL_WARNING_DETAIL_LIMIT,
    )
}

fn compact_inline_detail(text: &str, max_len: usize) -> String {
    compact_whitespace_detail(text, max_len)
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
            "auto: {}  |  detail: {}",
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
fn build_input_lines_with_context(
    context: &ShellCorePresentationContext<'_>,
) -> Vec<Line<'static>> {
    match context.conversation_state {
        ShellConversationState::Loading => vec![
            Line::from("Thread is still loading."),
            Line::from("Input becomes available when the shell reaches ready state."),
        ],
        ShellConversationState::Failed(message) => vec![Line::from(message.to_string())],
        ShellConversationState::Ready(conversation) => {
            build_ready_input_lines(conversation, context.shell_action_availability)
        }
    }
}

#[cfg(test)]
pub(super) fn build_ready_input_lines(
    conversation: &ConversationViewModel,
    shell_action_availability: ShellActionAvailability,
) -> Vec<Line<'static>> {
    let prompt_buffer = build_prompt_buffer_view(conversation);
    let mut lines = prompt_buffer.lines;

    if conversation.input_buffer.is_empty() {
        if let Some(status_lines) = auto_follow_prompt_lines(conversation) {
            lines.extend(status_lines);
            lines.push(Line::from(InlineShellCommand::command_list_line()));
            return lines;
        }
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

    if conversation.inline_shell_command_palette_state.is_active() {
        lines.extend(build_shell_command_palette_lines(conversation));
        return lines;
    }

    if let Some(command) = InlineShellCommandInput::parse(&conversation.input_buffer) {
        lines.push(Line::from(command.buffered_hint()));
        return lines;
    }

    if conversation.auto_follow_state.has_live_activity()
        && conversation.input_state.can_submit_now()
    {
        lines.push(Line::from(
            "Prompt buffered. Ctrl+j inserts a new line. Press Enter when auto follow-up finishes.",
        ));
        return lines;
    }

    match (conversation.input_state, shell_action_availability) {
        (
            ConversationInputState::DraftReady | ConversationInputState::ReadyToContinue,
            ShellActionAvailability::Pending,
        ) if conversation.startup_submit_armed => {
            lines.push(Line::from("Prompt queued until startup checks finish."));
            lines.push(Line::from(
                "Ctrl+j inserts a new line. Editing cancels the queued send.",
            ));
        }
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

fn build_shell_command_palette_lines(conversation: &ConversationViewModel) -> Vec<Line<'static>> {
    let palette_state = &conversation.inline_shell_command_palette_state;
    if !palette_state.is_active() {
        return Vec::new();
    }

    let Some(prefix) = InlineShellCommand::suggestion_prefix(&conversation.input_buffer) else {
        return Vec::new();
    };
    if palette_state.suggestions().is_empty() {
        return vec![Line::from(format!("  no shell commands match `{prefix}`"))];
    }

    let selected_index = palette_state.selected_index().unwrap_or(0);
    let suggestions = palette_state.suggestions();
    let (window_start, window_end) =
        build_shell_command_palette_window(suggestions.len(), selected_index);

    suggestions[window_start..window_end]
        .iter()
        .enumerate()
        .map(|(offset, command)| {
            let is_selected = selected_index == window_start + offset;
            let selector = if is_selected { "> " } else { "  " };
            let detail = if command.requires_argument() {
                format!("{} / add value", command.suggestion_detail())
            } else {
                command.suggestion_detail().to_string()
            };
            let label_style = if is_selected {
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            };
            let detail_style = if is_selected {
                Style::default().add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::DarkGray)
            };

            Line::from(vec![
                Span::raw(selector),
                Span::styled(command.command_name().to_string(), label_style),
                Span::raw("  "),
                Span::styled(detail, detail_style),
            ])
        })
        .collect()
}

fn build_shell_command_palette_window(
    suggestion_count: usize,
    selected_index: usize,
) -> (usize, usize) {
    if suggestion_count <= INLINE_COMMAND_PALETTE_VISIBLE_LIMIT {
        return (0, suggestion_count);
    }

    let max_window_start = suggestion_count - INLINE_COMMAND_PALETTE_VISIBLE_LIMIT;
    let window_start = selected_index
        .saturating_sub(INLINE_COMMAND_PALETTE_VISIBLE_LIMIT / 2)
        .min(max_window_start);
    (
        window_start,
        window_start + INLINE_COMMAND_PALETTE_VISIBLE_LIMIT,
    )
}

#[cfg(test)]
pub(super) fn build_input_prompt_cursor_offset(
    app: &NativeTuiApp,
    content_width: u16,
) -> Option<(u16, u16)> {
    let ConversationState::Ready(conversation) = &app.conversation_state else {
        return None;
    };

    build_prompt_cursor_offset(conversation, content_width)
}

pub(super) fn build_inline_prompt_cursor_offset(
    app: &NativeTuiApp,
    content_width: u16,
) -> Option<(u16, u16)> {
    let ConversationState::Ready(conversation) = &app.conversation_state else {
        return None;
    };

    let prompt_lines = build_inline_tail_prompt_lines(app);
    let tail_lines = build_inline_tail_lines(app);
    let prompt_start_index = tail_lines.len().saturating_sub(prompt_lines.len());
    let prompt_start_row = tail_lines[..prompt_start_index]
        .iter()
        .map(|line| wrapped_row_count(line.width(), content_width))
        .sum::<usize>() as u16;
    let (cursor_x, cursor_y) = build_prompt_cursor_offset(conversation, content_width)?;

    Some((cursor_x, prompt_start_row.saturating_add(cursor_y)))
}

fn build_prompt_cursor_offset(
    conversation: &ConversationViewModel,
    content_width: u16,
) -> Option<(u16, u16)> {
    if content_width == 0 {
        return None;
    }

    let prompt_buffer = build_prompt_buffer_view(conversation);
    let wrapped_rows_before_cursor = prompt_buffer.lines[..prompt_buffer.cursor_line_index]
        .iter()
        .map(|line| wrapped_row_count(line.width(), content_width))
        .sum::<usize>();
    let cursor_row_in_line = prompt_buffer.cursor_column / content_width as usize;
    let cursor_column = (prompt_buffer.cursor_column % content_width as usize) as u16;
    let cursor_row = wrapped_rows_before_cursor
        .saturating_add(cursor_row_in_line)
        .min(u16::MAX as usize) as u16;

    Some((cursor_column, cursor_row))
}

fn build_prompt_buffer_view(conversation: &ConversationViewModel) -> PromptBufferView {
    let buffer_lines = conversation.input_buffer.split('\n').collect::<Vec<_>>();
    let mut lines = Vec::with_capacity(buffer_lines.len().max(1));
    let mut cursor_line_index = 0;
    let mut cursor_column = 0;

    for (index, buffer_line) in buffer_lines.iter().enumerate() {
        let prefix = if index == 0 {
            PROMPT_PRIMARY_PREFIX
        } else {
            PROMPT_CONTINUATION_PREFIX
        };
        let rendered_line = format!("{prefix}{buffer_line}");
        if index + 1 == buffer_lines.len() {
            cursor_line_index = index;
            cursor_column = Line::from(rendered_line.clone()).width();
        }
        lines.push(Line::from(rendered_line));
    }

    PromptBufferView {
        lines,
        cursor_line_index,
        cursor_column,
    }
}

fn wrapped_row_count(line_width: usize, content_width: u16) -> usize {
    if content_width == 0 {
        return 0;
    }
    if line_width == 0 {
        return 1;
    }

    line_width.div_ceil(content_width as usize)
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
        Line::from(
            "Ctrl+a: automation on/off    Ctrl+l: edit max turns    Ctrl+g: edit stop keyword",
        ),
        Line::from("Ctrl+k: stop rule on/off    Ctrl+n: no-file stop    Enter/Esc/Ctrl+C: close"),
    ]
}

#[cfg(test)]
fn build_shell_header_lines_with_context(
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

#[cfg(test)]
fn build_shell_title() -> Line<'static> {
    Line::from("Shell / Ctrl+t new draft / Ctrl+C back / Ctrl+q quit")
}

#[cfg(test)]
pub(super) fn build_transcript_title(app: &NativeTuiApp, mode: ShellFrontendMode) -> Line<'static> {
    let _ = mode;
    build_transcript_title_with_context(&ShellCorePresentationContext::from_app(app))
}

#[cfg(test)]
fn build_transcript_title_with_context(
    _context: &ShellCorePresentationContext<'_>,
) -> Line<'static> {
    Line::from("Transcript / live scrollback")
}

#[cfg(test)]
pub(super) fn build_status_title() -> Line<'static> {
    Line::from("Controls / shell shortcuts and live status")
}

#[cfg(test)]
pub(super) fn build_input_title(app: &NativeTuiApp, mode: ShellFrontendMode) -> Line<'static> {
    let _ = mode;
    build_input_title_with_context(&ShellCorePresentationContext::from_app(app))
}

#[cfg(test)]
fn build_input_title_with_context(context: &ShellCorePresentationContext<'_>) -> Line<'static> {
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
                    conversation.input_state.label().to_string(),
                    input_state_style(conversation.input_state),
                ),
                Span::raw(" / "),
                Span::raw(submit_hint),
                Span::raw(" / Ctrl+j newline"),
            ])
        }
    }
}

#[cfg(test)]
fn build_frontend_summary_line() -> Line<'static> {
    Line::from(
        "frontend: inline main buffer  |  history: host terminal scrollback  |  tail: prompt anchored",
    )
}

#[cfg(test)]
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

#[cfg(test)]
fn startup_state_style_for_availability(
    shell_action_availability: ShellActionAvailability,
) -> Style {
    match shell_action_availability {
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

fn build_startup_check_lines(app: &NativeTuiApp) -> Vec<Line<'static>> {
    build_startup_check_lines_from_state(&app.startup_state)
}

fn build_startup_check_lines_from_state(startup_state: &StartupState) -> Vec<Line<'static>> {
    match startup_state {
        StartupState::Idle => vec![Line::from("startup check has not started")],
        StartupState::Loading => vec![
            Line::from("checking codex binary"),
            Line::from("opening codex app-server"),
            Line::from("reading account state"),
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
            Line::from(format!("schema snapshot: {}", diagnostics.schema_snapshot)),
        ],
        StartupState::Failed(message) => vec![Line::from(format!("startup error: {message}"))],
    }
}

fn build_startup_warning_lines(app: &NativeTuiApp) -> Vec<Line<'static>> {
    build_startup_warning_lines_from_state(&app.startup_state)
}

fn build_startup_warning_lines_from_state(startup_state: &StartupState) -> Vec<Line<'static>> {
    match startup_state {
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
    let current_workspace_directory = app.current_workspace_directory();

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
        SessionState::Ready(recent_sessions) => {
            let browser_view = build_session_browser_view(
                recent_sessions,
                app.session_overlay_ui_state.browser_state(),
                Some(current_workspace_directory.as_str()),
                app.session_overlay_ui_state.selected_session_id(),
                app.selected_session_index,
            );
            if recent_sessions.items.is_empty() {
                let mut lines = build_session_browser_summary_lines(app, &browser_view);
                lines.push(Line::from(""));
                lines.push(Line::from(
                    "codex app-server has not returned any recent sessions yet",
                ));
                lines.push(Line::from(
                    "Start a new draft with n, then reload the browser with r.",
                ));
                return (
                    OverlayListView {
                        message_lines: Some(vec![Line::from(
                            "no recent sessions have been recorded yet",
                        )]),
                        items: Vec::new(),
                        selected_index: None,
                    },
                    lines,
                );
            }

            if browser_view.visible_sessions.is_empty() {
                let search_query = app
                    .session_overlay_ui_state
                    .browser_state()
                    .search_query
                    .as_str();
                let mut lines = build_session_browser_summary_lines(app, &browser_view);
                lines.push(Line::from(""));
                lines.push(Line::from(build_session_empty_detail_line(
                    &browser_view,
                    search_query,
                )));
                lines.push(Line::from(build_session_empty_hint_line(&browser_view)));
                return (
                    OverlayListView {
                        message_lines: Some(vec![Line::from(build_session_empty_message(
                            &browser_view,
                            search_query,
                        ))]),
                        items: Vec::new(),
                        selected_index: None,
                    },
                    lines,
                );
            }

            let Some(selected_session) = browser_view.selected_session() else {
                let search_query = app
                    .session_overlay_ui_state
                    .browser_state()
                    .search_query
                    .as_str();
                let mut lines = build_session_browser_summary_lines(app, &browser_view);
                lines.push(Line::from(""));
                lines.push(Line::from(build_session_empty_detail_line(
                    &browser_view,
                    search_query,
                )));
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
                    lines,
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

            lines.extend(build_session_browser_summary_lines(app, &browser_view));

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

fn build_session_browser_summary_lines(
    app: &NativeTuiApp,
    browser_view: &SessionBrowserView<'_>,
) -> Vec<Line<'static>> {
    let active_filter_option = browser_view.projection.active_project_filter_option();
    let filter_label = active_filter_option
        .map(|option| option.label.clone())
        .unwrap_or_else(|| "all projects".to_string());
    let filter_session_count = active_filter_option
        .map(|option| option.session_count)
        .unwrap_or(browser_view.projection.filtered_session_count);
    let browser_query = if app.session_overlay_ui_state.is_search_query_editing() {
        app.session_overlay_ui_state.search_query_editor_buffer()
    } else {
        &app.session_overlay_ui_state.browser_state().search_query
    };
    let mut lines = vec![
        Line::from(format!(
            "{}: {}",
            if app.session_overlay_ui_state.is_search_query_editing() {
                "query edit"
            } else {
                "query"
            },
            format_session_query_label(browser_query)
        )),
        Line::from(format_session_filter_line(
            &browser_view.projection,
            &filter_label,
            filter_session_count,
        )),
        Line::from(build_session_project_context_line(
            &browser_view.projection,
            &app.current_workspace_directory(),
        )),
        Line::from(format_session_browser_line(
            &browser_view.projection,
            &filter_label,
        )),
    ];

    if app.session_overlay_ui_state.is_search_query_editing() {
        lines.push(Line::from(
            "Enter applies the query. Esc keeps the saved browser state.",
        ));
    }

    lines
}

fn build_session_key_lines(app: &NativeTuiApp) -> Vec<Line<'static>> {
    if app.session_overlay_ui_state.is_search_query_editing() {
        return vec![
            Line::from("Type the session query directly. Spaces match multiple tokens."),
            Line::from("Enter: apply query    Esc/Ctrl+C: cancel    Backspace: delete"),
        ];
    }

    vec![
        Line::from("/: query    c: clear    Tab/BackTab: filter    [ ] or PgUp/PgDn: page"),
        Line::from("Up/Down or Home/End or g/G: move    Enter: open    Esc/Ctrl+C: close"),
        Line::from("n: draft    r: reload    Ctrl+d: diagnostics"),
    ]
}

fn format_session_query_label(search_query: &str) -> String {
    if search_query.is_empty() {
        "(all text)".to_string()
    } else {
        search_query.to_string()
    }
}

fn build_session_project_context_line(
    projection: &crate::application::service::session_service::SessionBrowserProjection,
    current_workspace_directory: &str,
) -> String {
    let current_workspace_label = format!("current workspace ({current_workspace_directory})");
    let Some(active_filter_option) = projection.active_project_filter_option() else {
        return format!("context: {current_workspace_label}");
    };

    if active_filter_option.is_current_workspace {
        return format!("context: showing only {current_workspace_label}");
    }

    match projection.current_workspace_session_count {
        0 => format!("context: {current_workspace_label} has no recent sessions"),
        1 => format!("context: {current_workspace_label} has 1 recent session"),
        count => format!("context: {current_workspace_label} has {count} recent sessions"),
    }
}

fn build_session_empty_message(
    browser_view: &SessionBrowserView<'_>,
    search_query: &str,
) -> String {
    format_session_empty_message(
        &browser_view.projection.active_project_filter,
        search_query,
        browser_view
            .projection
            .active_project_filter_option()
            .map(|option| option.label.as_str()),
        browser_view
            .projection
            .active_project_filter_option()
            .is_some_and(|option| option.is_current_workspace),
        browser_view.projection.filtered_session_count,
    )
}

fn build_session_empty_detail_line(
    browser_view: &SessionBrowserView<'_>,
    search_query: &str,
) -> String {
    format_session_empty_detail_line(
        &browser_view.projection.active_project_filter,
        search_query,
        browser_view
            .projection
            .active_project_filter_option()
            .map(|option| option.label.as_str()),
        browser_view
            .projection
            .active_project_filter_option()
            .is_some_and(|option| option.is_current_workspace),
        browser_view.projection.filtered_session_count,
    )
}

fn build_session_empty_hint_line(browser_view: &SessionBrowserView<'_>) -> String {
    if browser_view.projection.filtered_session_count == 0 {
        "Press c to clear the browser, Tab/BackTab to cycle filters, or r to reload.".to_string()
    } else {
        "Use Up/Down or Home/End to pick another session, or reload with r.".to_string()
    }
}

fn format_session_empty_message(
    active_project_filter: &SessionProjectFilter,
    search_query: &str,
    active_filter_label: Option<&str>,
    is_current_workspace_filter: bool,
    filtered_session_count: usize,
) -> String {
    if filtered_session_count > 0 {
        return "the current page has no visible session selection".to_string();
    }

    match active_project_filter {
        SessionProjectFilter::AllProjects if search_query.is_empty() => {
            "no sessions match the current browser state".to_string()
        }
        SessionProjectFilter::AllProjects => {
            format!(
                "no sessions match query {}",
                quoted_session_query(search_query)
            )
        }
        SessionProjectFilter::RecentProject { .. }
            if is_current_workspace_filter && search_query.is_empty() =>
        {
            "no current-workspace sessions match the current browser state".to_string()
        }
        SessionProjectFilter::RecentProject { .. } if is_current_workspace_filter => {
            format!(
                "no current-workspace sessions match query {}",
                quoted_session_query(search_query)
            )
        }
        SessionProjectFilter::RecentProject { .. } if search_query.is_empty() => format!(
            "no sessions in {} match the current browser state",
            active_filter_label.unwrap_or("the selected project")
        ),
        SessionProjectFilter::RecentProject { .. } => format!(
            "no sessions in {} match query {}",
            active_filter_label.unwrap_or("the selected project"),
            quoted_session_query(search_query),
        ),
    }
}

fn format_session_empty_detail_line(
    active_project_filter: &SessionProjectFilter,
    search_query: &str,
    active_filter_label: Option<&str>,
    is_current_workspace_filter: bool,
    filtered_session_count: usize,
) -> String {
    if filtered_session_count > 0 {
        return "no session detail is available for the current browser page".to_string();
    }

    match active_project_filter {
        SessionProjectFilter::AllProjects if search_query.is_empty() => {
            "no session detail is available for the current browser state".to_string()
        }
        SessionProjectFilter::AllProjects => {
            format!(
                "no session detail is available for query {}",
                quoted_session_query(search_query)
            )
        }
        SessionProjectFilter::RecentProject { .. }
            if is_current_workspace_filter && search_query.is_empty() =>
        {
            "no session detail is available for the current workspace filter".to_string()
        }
        SessionProjectFilter::RecentProject { .. } if is_current_workspace_filter => {
            format!(
                "no current-workspace session detail is available for query {}",
                quoted_session_query(search_query)
            )
        }
        SessionProjectFilter::RecentProject { .. } if search_query.is_empty() => format!(
            "no session detail is available for {}",
            active_filter_label.unwrap_or("the selected project filter")
        ),
        SessionProjectFilter::RecentProject { .. } => format!(
            "no session detail is available for {} and query {}",
            active_filter_label.unwrap_or("the selected project filter"),
            quoted_session_query(search_query),
        ),
    }
}

fn quoted_session_query(search_query: &str) -> String {
    format!("\"{search_query}\"")
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

fn diagnostic_item(title: &str, ok: bool, detail: &str) -> Line<'static> {
    let marker = if ok { "[ok]" } else { "[warn]" };
    Line::from(format!("{marker} {title}: {detail}"))
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

fn format_session_filter_line(
    projection: &crate::application::service::session_service::SessionBrowserProjection,
    filter_label: &str,
    filter_session_count: usize,
) -> String {
    let session_suffix = plural_suffix(filter_session_count);
    match &projection.active_project_filter {
        crate::application::service::session_service::SessionProjectFilter::AllProjects => {
            let workspace_count = projection.project_filter_options.len().saturating_sub(1);
            let workspace_suffix = plural_suffix(workspace_count);
            if workspace_count > 1 {
                format!(
                    "filter: {filter_label} ({filter_session_count} recent session{session_suffix} across {workspace_count} workspace{workspace_suffix})"
                )
            } else {
                format!(
                    "filter: {filter_label} ({filter_session_count} recent session{session_suffix})"
                )
            }
        }
        crate::application::service::session_service::SessionProjectFilter::RecentProject {
            ..
        } => {
            format!(
                "filter: {filter_label} ({filter_session_count} recent session{session_suffix})"
            )
        }
    }
}

fn format_session_browser_line(
    projection: &crate::application::service::session_service::SessionBrowserProjection,
    filter_label: &str,
) -> String {
    if projection.total_session_count == 0 {
        return "browser: no recent sessions loaded".to_string();
    }

    if projection.filtered_session_count == 0 {
        return match &projection.active_project_filter {
            crate::application::service::session_service::SessionProjectFilter::AllProjects => {
                format!(
                    "browser: no matches in {} recent session{}",
                    projection.project_filtered_session_count,
                    plural_suffix(projection.project_filtered_session_count)
                )
            }
            crate::application::service::session_service::SessionProjectFilter::RecentProject {
                ..
            } => format!(
                "browser: no matches in {filter_label} across {} recent session{}",
                projection.project_filtered_session_count,
                plural_suffix(projection.project_filtered_session_count)
            ),
        };
    }

    let (visible_start, visible_end) = projection
        .visible_session_range
        .expect("visible range should exist when filtered sessions are visible");
    format!(
        "browser: page {} of {} | showing {}-{} of {} matches",
        projection.page_index + 1,
        projection.total_pages.max(1),
        visible_start,
        visible_end,
        projection.filtered_session_count,
    )
}

fn plural_suffix(count: usize) -> &'static str {
    if count == 1 { "" } else { "s" }
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
    if conversation.has_running_turn() || conversation.auto_follow_state.has_live_activity() {
        "working"
    } else {
        "idle"
    }
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

fn build_working_line(
    conversation: &ConversationViewModel,
    max_detail_len: usize,
) -> Option<Line<'static>> {
    let (started_at, detail) = if conversation.auto_follow_state.has_live_activity() {
        (
            conversation.auto_follow_state.active_started_at()?,
            auto_follow_working_detail(conversation),
        )
    } else {
        (
            conversation.active_turn_started_at?,
            manual_turn_working_detail(conversation)?,
        )
    };
    let detail = compact_inline_detail(&detail, max_detail_len);
    let elapsed = format_elapsed(Instant::now().saturating_duration_since(started_at));

    Some(Line::from(vec![
        Span::styled(
            "◦ Working".to_string(),
            Style::default()
                .fg(Color::Gray)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!(" ({elapsed} • {detail})"),
            Style::default().fg(Color::DarkGray),
        ),
    ]))
}

fn manual_turn_working_detail(conversation: &ConversationViewModel) -> Option<String> {
    if !conversation.has_running_turn() {
        return None;
    }

    match conversation.input_state {
        ConversationInputState::SubmittingTurn => Some("starting turn".to_string()),
        ConversationInputState::StreamingTurn => {
            if conversation.live_agent_message.is_some() {
                Some("turn running".to_string())
            } else {
                Some("waiting for response".to_string())
            }
        }
        ConversationInputState::DraftReady | ConversationInputState::ReadyToContinue => None,
    }
}

fn auto_follow_working_detail(conversation: &ConversationViewModel) -> String {
    match &conversation.auto_follow_state.runtime_phase {
        AutoFollowRuntimePhase::Idle => "idle".to_string(),
        AutoFollowRuntimePhase::Evaluating { .. } => "evaluating next auto follow-up".to_string(),
        AutoFollowRuntimePhase::Queued { turn_index, .. } => format!(
            "auto turn {turn_index}/{} queued for submission",
            conversation.auto_follow_state.max_auto_turns_value()
        ),
        AutoFollowRuntimePhase::Submitting { turn_index, .. } => format!(
            "auto turn {turn_index}/{} starting",
            conversation.auto_follow_state.max_auto_turns_value()
        ),
        AutoFollowRuntimePhase::Running { turn_index, .. } => format!(
            "auto turn {turn_index}/{} running",
            conversation.auto_follow_state.max_auto_turns_value()
        ),
    }
}

fn auto_follow_prompt_status_line(
    conversation: &ConversationViewModel,
    inline: bool,
) -> Option<String> {
    let detail = match &conversation.auto_follow_state.runtime_phase {
        AutoFollowRuntimePhase::Idle => return None,
        AutoFollowRuntimePhase::Evaluating { .. } => "auto follow-up evaluating".to_string(),
        AutoFollowRuntimePhase::Queued { turn_index, .. } => format!(
            "auto turn {turn_index}/{} queued",
            conversation.auto_follow_state.max_auto_turns_value()
        ),
        AutoFollowRuntimePhase::Submitting { turn_index, .. } => format!(
            "auto turn {turn_index}/{} starting",
            conversation.auto_follow_state.max_auto_turns_value()
        ),
        AutoFollowRuntimePhase::Running { turn_index, .. } => format!(
            "auto turn {turn_index}/{} running",
            conversation.auto_follow_state.max_auto_turns_value()
        ),
    };

    Some(if inline {
        format!("prompt: {detail}  |  type now, Enter when idle")
    } else {
        detail
    })
}

#[cfg(test)]
fn auto_follow_prompt_lines(conversation: &ConversationViewModel) -> Option<Vec<Line<'static>>> {
    let detail = auto_follow_prompt_status_line(conversation, false)?;
    Some(vec![
        Line::from(format!("Auto follow-up is {detail}.")),
        Line::from("Type now; press Enter after the shell returns idle."),
    ])
}

pub(super) fn format_elapsed(duration: Duration) -> String {
    let total_seconds = duration.as_secs();
    let hours = total_seconds / 3600;
    let minutes = (total_seconds % 3600) / 60;
    let seconds = total_seconds % 60;

    if hours > 0 {
        format!("{hours}h {minutes}m")
    } else if minutes > 0 {
        format!("{minutes}m {seconds}s")
    } else {
        format!("{seconds}s")
    }
}

fn inline_input_state_label(input_state: ConversationInputState) -> &'static str {
    match input_state {
        ConversationInputState::DraftReady => "draft",
        ConversationInputState::ReadyToContinue => "ready",
        ConversationInputState::SubmittingTurn => "sending",
        ConversationInputState::StreamingTurn => "streaming",
    }
}

#[cfg(test)]
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
        ConversationMessageKind::Status => Style::default().fg(Color::Gray),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::service::session_service::{
        SessionBrowserProjection, SessionProjectFilter, SessionProjectFilterOption,
    };

    #[test]
    fn project_context_line_surfaces_current_workspace_session_count() {
        let projection = sample_projection(
            SessionProjectFilter::RecentProject {
                workspace_directory: "/tmp/docs".to_string(),
            },
            vec![
                SessionProjectFilterOption {
                    filter: SessionProjectFilter::AllProjects,
                    label: "all projects".to_string(),
                    session_count: 5,
                    is_current_workspace: false,
                },
                SessionProjectFilterOption {
                    filter: SessionProjectFilter::RecentProject {
                        workspace_directory: "/tmp/docs".to_string(),
                    },
                    label: "/tmp/docs".to_string(),
                    session_count: 3,
                    is_current_workspace: false,
                },
                SessionProjectFilterOption {
                    filter: SessionProjectFilter::RecentProject {
                        workspace_directory: "/tmp/root".to_string(),
                    },
                    label: "current workspace (/tmp/root)".to_string(),
                    session_count: 2,
                    is_current_workspace: true,
                },
            ],
            2,
            3,
        );

        let line = build_session_project_context_line(&projection, "/tmp/root");

        assert_eq!(
            line,
            "context: current workspace (/tmp/root) has 2 recent sessions"
        );
    }

    #[test]
    fn empty_state_messages_include_query_for_current_workspace_filter() {
        let message = format_session_empty_message(
            &SessionProjectFilter::RecentProject {
                workspace_directory: "/tmp/root".to_string(),
            },
            "release",
            Some("current workspace (/tmp/root)"),
            true,
            0,
        );
        let detail = format_session_empty_detail_line(
            &SessionProjectFilter::RecentProject {
                workspace_directory: "/tmp/root".to_string(),
            },
            "release",
            Some("current workspace (/tmp/root)"),
            true,
            0,
        );

        assert_eq!(
            message,
            "no current-workspace sessions match query \"release\""
        );
        assert_eq!(
            detail,
            "no current-workspace session detail is available for query \"release\""
        );
    }

    fn sample_projection(
        active_project_filter: SessionProjectFilter,
        project_filter_options: Vec<SessionProjectFilterOption>,
        current_workspace_session_count: usize,
        filtered_session_count: usize,
    ) -> SessionBrowserProjection {
        let total_session_count = project_filter_options
            .first()
            .map(|option| option.session_count)
            .unwrap_or(filtered_session_count);
        let project_filtered_session_count = project_filter_options
            .iter()
            .find(|option| option.filter == active_project_filter)
            .map(|option| option.session_count)
            .unwrap_or(filtered_session_count);
        SessionBrowserProjection {
            active_project_filter,
            project_filter_options,
            current_workspace_session_count,
            total_session_count,
            project_filtered_session_count,
            filtered_session_count,
            page_index: 0,
            total_pages: 1,
            visible_session_range: Some((1, 1)),
            page_session_indexes: vec![0],
        }
    }
}
