use std::time::{Duration, Instant};

pub(super) use super::planning::{build_automation_preview_lines, build_automation_status_lines};
use super::*;
use crate::adapter::inbound::tui::conversation_text::conversation_message_label;
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

    fn startup_screen_is_active(&self) -> bool {
        let Some(conversation) = self.ready_conversation() else {
            return false;
        };

        !conversation.has_active_thread()
            && conversation.messages.is_empty()
            && conversation.active_turn_id.is_none()
            && conversation.live_agent_message.is_none()
    }

    fn startup_banner_is_active(&self) -> bool {
        self.show_startup_ascii_art && self.startup_screen_is_active()
    }
}

#[path = "shell_presentation/overlays.rs"]
mod overlays;
#[path = "shell_presentation/session_browser.rs"]
mod session_browser;
#[path = "shell_presentation/status_panels.rs"]
mod status_panels;

use session_browser::recent_session_status_label;

pub(super) use overlays::{
    AutomationOverlayView, DirectionsMaintenanceOverlayView, OverlayListView,
    PlanningDraftEditorOverlayView, PlanningInitOverlayView, QueueOverlayView, SessionOverlayView,
    StartupOverlayView, SupersessionOverlayView, build_automation_overlay_view,
    build_directions_maintenance_overlay_view, build_planning_draft_editor_overlay_view,
    build_planning_init_overlay_view, build_queue_overlay_view, build_session_overlay_view,
    build_startup_banner_lines, build_startup_overlay_view, build_supersession_overlay_view,
};
#[cfg(test)]
pub(super) use overlays::{
    build_conversation_shell_frame_view, build_conversation_shell_view, build_transcript_panel_view,
};
pub(super) use status_panels::InlineTailView;

#[cfg(test)]
fn build_shell_footer_lines_with_context(
    context: &ShellCorePresentationContext<'_>,
    plan_mode_indicator: status_panels::PlanModeIndicatorView,
    parallel_mode_summary_line: String,
    parallel_mode_alert_line: Option<String>,
    github_review_recent_changes_summary: Option<String>,
    planning_summary_line: Option<String>,
    planning_notice_line: Option<String>,
    planner_panel_lines: Vec<String>,
) -> Vec<Line<'static>> {
    status_panels::build_shell_footer_lines_with_context(
        context,
        plan_mode_indicator,
        parallel_mode_summary_line,
        parallel_mode_alert_line,
        github_review_recent_changes_summary,
        planning_summary_line,
        planning_notice_line,
        planner_panel_lines,
    )
}

#[cfg(test)]
fn current_live_agent_lines(conversation: &ConversationViewModel) -> Option<Vec<Line<'static>>> {
    status_panels::current_live_agent_lines(conversation)
}

#[cfg(test)]
fn current_plan_mode_indicator(app: &NativeTuiApp) -> status_panels::PlanModeIndicatorView {
    status_panels::current_plan_mode_indicator(app)
}

#[cfg(test)]
pub(super) fn build_inline_tail_lines(app: &NativeTuiApp) -> Vec<Line<'static>> {
    status_panels::build_inline_tail_lines(app)
}

pub(super) fn build_inline_tail_view(app: &NativeTuiApp, content_width: u16) -> InlineTailView {
    status_panels::build_inline_tail_view(app, content_width)
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
    if !context.startup_banner_is_active() {
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

fn compact_inline_detail(text: &str, max_len: usize) -> String {
    compact_whitespace_detail(text, max_len)
}

pub(super) fn build_automation_key_lines(app: &NativeTuiApp) -> Vec<Line<'static>> {
    if app.is_max_auto_turns_editing() {
        return vec![
            Line::from("Type the new max-turn value directly. Backspace deletes."),
            Line::from("Enter: save max turns    Esc/Ctrl+C: cancel edit"),
            Line::from("Use a whole number greater than 0, or type infinite."),
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
        Line::from("PageUp/PageDown or Ctrl+u/Ctrl+d: scroll preview"),
        Line::from(
            "Ctrl+a: automation on/off    Ctrl+l: edit max turns    Ctrl+g: edit stop keyword",
        ),
        Line::from("Ctrl+k: stop rule on/off    Ctrl+n: no-file stop    Ctrl+b: planner detail"),
        Line::from("Enter/Esc/Ctrl+C: close"),
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

fn build_automation_list_view(app: &NativeTuiApp) -> OverlayListView {
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
        ConversationState::Ready(_) => OverlayListView {
            message_lines: Some(vec![
                Line::from("automation follows the planning queue only"),
                Line::from("no legacy automation catalogs or workspace prompt files are used"),
            ]),
            items: Vec::new(),
            selected_index: None,
        },
    }
}

fn diagnostic_item(title: &str, ok: bool, detail: &str) -> Line<'static> {
    let marker = if ok { "[ok]" } else { "[warn]" };
    Line::from(format!("{marker} {title}: {detail}"))
}

fn turn_status_label(conversation: &ConversationViewModel) -> &'static str {
    if conversation.has_running_turn() || conversation.auto_follow_state.has_live_activity() {
        "working"
    } else {
        "idle"
    }
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
    let max_auto_turns = conversation.auto_follow_state.max_auto_turns_label();
    match &conversation.auto_follow_state.runtime_phase {
        AutoFollowRuntimePhase::Idle => "idle".to_string(),
        AutoFollowRuntimePhase::Evaluating { .. } => "evaluating next auto follow-up".to_string(),
        AutoFollowRuntimePhase::Queued { turn_index, .. } => {
            format!("auto turn {turn_index}/{max_auto_turns} queued for submission")
        }
        AutoFollowRuntimePhase::Submitting { turn_index, .. } => {
            format!("auto turn {turn_index}/{max_auto_turns} starting")
        }
        AutoFollowRuntimePhase::Running { turn_index, .. } => {
            format!("auto turn {turn_index}/{max_auto_turns} running")
        }
    }
}

fn auto_follow_prompt_status_line(
    conversation: &ConversationViewModel,
    inline: bool,
) -> Option<String> {
    let max_auto_turns = conversation.auto_follow_state.max_auto_turns_label();
    let detail = match &conversation.auto_follow_state.runtime_phase {
        AutoFollowRuntimePhase::Idle => return None,
        AutoFollowRuntimePhase::Evaluating { .. } => "auto follow-up evaluating".to_string(),
        AutoFollowRuntimePhase::Queued { turn_index, .. } => {
            format!("auto turn {turn_index}/{max_auto_turns} queued")
        }
        AutoFollowRuntimePhase::Submitting { turn_index, .. } => {
            format!("auto turn {turn_index}/{max_auto_turns} starting")
        }
        AutoFollowRuntimePhase::Running { turn_index, .. } => {
            format!("auto turn {turn_index}/{max_auto_turns} running")
        }
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
    use crate::adapter::inbound::tui::app::{
        AutoFollowRuntimePhase, AutoFollowState, ConversationViewModel,
        INFINITE_AUTO_FOLLOW_MAX_TURNS,
    };

    #[test]
    fn auto_follow_status_lines_use_infinite_label() {
        let mut conversation = ConversationViewModel::new_draft("/tmp/workspace".to_string());
        conversation.auto_follow_state = AutoFollowState::new();
        conversation
            .auto_follow_state
            .set_max_auto_turns(INFINITE_AUTO_FOLLOW_MAX_TURNS);
        conversation.auto_follow_state.runtime_phase = AutoFollowRuntimePhase::Running {
            started_at: Instant::now(),
            turn_index: 2,
        };

        assert_eq!(
            auto_follow_working_detail(&conversation),
            "auto turn 2/infinite running"
        );
        assert_eq!(
            auto_follow_prompt_status_line(&conversation, true).as_deref(),
            Some("prompt: auto turn 2/infinite running  |  type now, Enter when idle")
        );
    }
}
