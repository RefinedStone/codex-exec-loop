use std::time::{Duration, Instant};

pub(super) use super::planning::{build_automation_preview_lines, build_automation_status_lines};
use super::*;
use crate::adapter::inbound::tui::conversation_text::conversation_message_label;
use crate::application::service::session_service::{
    SessionBrowserView, SessionProjectFilter, build_session_browser_view,
};
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
#[path = "shell_presentation/status_panels.rs"]
mod status_panels;

pub(super) use overlays::{
    AutomationOverlayView, DirectionsMaintenanceOverlayView, OverlayListEntryView, OverlayListView,
    PlanningDraftEditorOverlayView, PlanningInitOverlayView, QueueOverlayView, SessionOverlayView,
    StartupOverlayView, build_automation_overlay_view, build_directions_maintenance_overlay_view,
    build_planning_draft_editor_overlay_view, build_planning_init_overlay_view,
    build_queue_overlay_view, build_session_overlay_view, build_startup_banner_lines,
    build_startup_overlay_view,
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
    github_review_recent_changes_summary: Option<String>,
    planning_summary_line: Option<String>,
    planning_notice_line: Option<String>,
    planner_panel_lines: Vec<String>,
) -> Vec<Line<'static>> {
    status_panels::build_shell_footer_lines_with_context(
        context,
        plan_mode_indicator,
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
        ShellConversationState::Loading => vec![
            Line::from("current state: waiting"),
            Line::from("cause: thread history is still loading from codex app-server"),
            Line::from("next action: wait for the thread history to load"),
        ],
        ShellConversationState::Failed(message) => vec![
            Line::from("current state: blocked"),
            Line::from("cause: thread history is unavailable because loading failed"),
            Line::from("next action: reload the session or open a new draft"),
            Line::from(format!("conversation error: {message}")),
        ],
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
            Line::from("current state: waiting"),
            Line::from("cause: thread history is still loading from codex app-server"),
            Line::from("next action: wait for the thread history to load"),
        ],
        ShellConversationState::Failed(message) => vec![
            Line::from("current state: blocked"),
            Line::from("cause: thread history is unavailable because loading failed"),
            Line::from("next action: reload the session or open a new draft"),
            Line::from(format!("conversation error: {message}")),
        ],
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
                    "Type now if you want, then send once startup checks finish.",
                ));
            }
            (_, ShellActionAvailability::Blocked) if conversation.input_state.can_submit_now() => {
                lines.push(Line::from("Startup checks need attention."));
                lines.push(Line::from(
                    "Open Ctrl+d, resolve the blocking check, then send the prompt.",
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
            "Prompt buffered. Ctrl+j inserts a new line. Press Enter when automation finishes.",
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
                "Prompt buffered. Ctrl+j inserts a new line. Press Enter after startup checks finish.",
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
            Line::from("Type the new turn-budget value directly. Backspace deletes."),
            Line::from("Enter: save turn budget    Esc/Ctrl+C: cancel edit"),
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
        Line::from("PageUp/PageDown or Ctrl+u/Ctrl+d: scroll preview"),
        Line::from(
            "Ctrl+a: automation on/off    Ctrl+l: edit turn budget    Ctrl+g: edit stop keyword",
        ),
        Line::from(
            "Ctrl+k: stop keyword on/off    Ctrl+n: no-file-change rule    Ctrl+b: planner visibility",
        ),
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
                Span::raw(" / waiting"),
            ]),
            Line::from("current state: waiting"),
            Line::from("cause: thread history is still loading from codex app-server"),
            Line::from("next action: wait for the thread history to load"),
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
                Span::raw(" / blocked"),
            ]),
            Line::from("current state: blocked"),
            Line::from("cause: thread history is unavailable because loading failed"),
            Line::from("next action: reload the session or open a new draft"),
            Line::from(format!("conversation error: {message}")),
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
            Line::from(vec![Span::raw("Prompt"), Span::raw(" / waiting")])
        }
        ShellConversationState::Failed(_) => {
            Line::from(vec![Span::raw("Prompt"), Span::raw(" / blocked")])
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
                "blocked while startup checks need attention".to_string()
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
        StartupState::Idle => vec![Line::from("startup checks have not started yet")],
        StartupState::Loading => vec![
            Line::from("checking codex CLI"),
            Line::from("checking workspace access"),
            Line::from("checking app-server readiness"),
            Line::from("checking account access"),
        ],
        StartupState::Ready(diagnostics) => vec![
            diagnostic_item(
                "codex CLI",
                diagnostics.codex_binary_ok,
                &diagnostics.codex_binary_detail,
            ),
            diagnostic_item(
                "workspace access",
                diagnostics.workspace_ok,
                &diagnostics.workspace_detail,
            ),
            diagnostic_item(
                "app-server readiness",
                diagnostics.initialize_ok,
                &diagnostics.initialize_detail,
            ),
            diagnostic_item(
                "account access",
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
                    "waiting for recent sessions"
                } else {
                    session_idle_message_line(&app.startup_state)
                })]),
                items: Vec::new(),
                selected_index: None,
            },
            build_session_unavailable_operator_lines(&app.startup_state),
        ),
        SessionState::Loading => (
            OverlayListView {
                message_lines: Some(vec![Line::from("loading recent sessions")]),
                items: Vec::new(),
                selected_index: None,
            },
            vec![
                Line::from("current state: waiting"),
                Line::from("cause: recent sessions are loading from codex app-server"),
                Line::from("next action: wait for the session list to load"),
            ],
        ),
        SessionState::Failed(message) => (
            OverlayListView {
                message_lines: Some(vec![Line::from("recent sessions blocked")]),
                items: Vec::new(),
                selected_index: None,
            },
            vec![
                Line::from("current state: blocked"),
                Line::from("cause: recent sessions are unavailable because loading failed"),
                Line::from("next action: press r to retry, or start a new draft with n"),
                Line::from(format!("recent sessions error: {message}")),
            ],
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
                Line::from(format!("thread id: {}", selected_session.id)),
                Line::from(format!(
                    "last updated: {}",
                    selected_session.updated_at_label()
                )),
                Line::from(format!("workspace: {}", selected_session.cwd)),
                Line::from(format!("thread source: {}", selected_session.source)),
                Line::from(format!(
                    "model provider: {}",
                    selected_session.model_provider
                )),
                Line::from(format!("current state: {}", selected_session.status_type)),
            ];

            if let Some(branch) = &selected_session.git_branch {
                lines.push(Line::from(format!("git branch: {branch}")));
            }

            lines.extend(build_session_browser_summary_lines(app, &browser_view));

            if recent_sessions.next_cursor.is_some() {
                lines.push(Line::from("more threads are available in the next cursor"));
            }

            lines.push(Line::from(""));
            lines.push(Line::from("latest preview"));
            lines.push(Line::from(selected_session.preview_block()));
            lines.push(Line::from(""));
            lines.push(Line::from(format!(
                "session file: {}",
                selected_session.path
            )));
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
        Line::from("n: draft    r: reload    Ctrl+d: startup checks"),
    ]
}

fn session_idle_message_line(startup_state: &StartupState) -> &'static str {
    match startup_state {
        StartupState::Ready(diagnostics) if diagnostics.can_continue() => {
            "waiting for recent sessions"
        }
        StartupState::Ready(_) | StartupState::Failed(_) => "recent sessions blocked",
        StartupState::Idle | StartupState::Loading => "waiting for startup checks",
    }
}

fn build_session_unavailable_operator_lines(startup_state: &StartupState) -> Vec<Line<'static>> {
    match startup_state {
        StartupState::Ready(diagnostics) if diagnostics.can_continue() => {
            return vec![
                Line::from("current state: waiting"),
                Line::from("cause: recent sessions have not loaded yet"),
                Line::from("next action: press r to load recent sessions"),
            ];
        }
        StartupState::Idle | StartupState::Loading => {
            return vec![
                Line::from("current state: waiting"),
                Line::from("cause: startup checks have not finished yet"),
                Line::from(
                    "next action: wait for startup checks to finish, then load recent sessions",
                ),
            ];
        }
        StartupState::Ready(_) | StartupState::Failed(_) => {}
    }

    vec![
        Line::from("current state: blocked"),
        Line::from("cause: startup checks must succeed before recent sessions are available"),
        Line::from(
            "next action: open startup checks with Ctrl+d, fix them, then reload recent sessions",
        ),
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
            "recent sessions remain unavailable until startup checks succeed",
        )],
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
                Line::from(
                    "automation continues only when the planning queue exposes actionable work",
                ),
                Line::from(
                    "legacy automation catalogs and ad hoc workspace prompt files stay ignored",
                ),
            ]),
            items: Vec::new(),
            selected_index: None,
        },
    }
}

fn diagnostic_item(title: &str, ok: bool, detail: &str) -> Line<'static> {
    let marker = if ok { "[ready]" } else { "[attention]" };
    Line::from(format!("{marker} {title}: {detail}"))
}

fn startup_check_component_label(ok: bool) -> &'static str {
    if ok { "ready" } else { "needs attention" }
}

fn startup_blocked_check_labels(
    diagnostics: &crate::domain::startup_diagnostics::StartupDiagnostics,
) -> Vec<&'static str> {
    let mut labels = Vec::new();
    if !diagnostics.codex_binary_ok {
        labels.push("codex CLI");
    }
    if !diagnostics.workspace_ok {
        labels.push("workspace access");
    }
    if !diagnostics.initialize_ok {
        labels.push("app-server readiness");
    }
    if !diagnostics.account_ok {
        labels.push("account access");
    }
    labels
}

pub(super) fn build_startup_check_summary_line(
    diagnostics: &crate::domain::startup_diagnostics::StartupDiagnostics,
) -> String {
    format!(
        "startup checks: codex {}  |  workspace {}  |  app-server {}  |  account {}",
        startup_check_component_label(diagnostics.codex_binary_ok),
        startup_check_component_label(diagnostics.workspace_ok),
        startup_check_component_label(diagnostics.initialize_ok),
        startup_check_component_label(diagnostics.account_ok),
    )
}

pub(super) fn build_startup_operator_lines_from_state(
    startup_state: &StartupState,
    max_detail_len: usize,
) -> Vec<Line<'static>> {
    match startup_state {
        StartupState::Idle => vec![
            Line::from("current state: waiting"),
            Line::from("cause: startup checks have not started yet"),
            Line::from("next action: wait for startup checks to start, or rerun them with r"),
        ],
        StartupState::Loading => vec![
            Line::from("current state: checking"),
            Line::from(
                "cause: the shell is verifying codex, workspace, app-server, and account access",
            ),
            Line::from("next action: wait for startup checks to finish"),
        ],
        StartupState::Ready(diagnostics) if diagnostics.can_continue() => {
            let cause_line = if diagnostics.warnings.is_empty() {
                "cause: codex, workspace, app-server, and account access are ready".to_string()
            } else {
                "cause: startup checks passed, but warnings still need review".to_string()
            };
            let next_action_line = if diagnostics.warnings.is_empty() {
                "next action: continue in the shell or open another inspection surface".to_string()
            } else {
                "next action: review the warnings, or continue if they do not block your work"
                    .to_string()
            };
            vec![
                Line::from("current state: ready"),
                Line::from(cause_line),
                Line::from(next_action_line),
            ]
        }
        StartupState::Ready(diagnostics) => {
            let blocked_checks = startup_blocked_check_labels(diagnostics).join(", ");
            vec![
                Line::from("current state: blocked"),
                Line::from(format!(
                    "cause: startup checks need attention for {blocked_checks}"
                )),
                Line::from("next action: inspect the failed checks, fix them, then rerun with r"),
            ]
        }
        StartupState::Failed(message) => vec![
            Line::from("current state: failed"),
            Line::from(format!(
                "cause: {}",
                compact_inline_detail(message, max_detail_len)
            )),
            Line::from("next action: rerun startup checks with r"),
        ],
    }
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
    match &conversation.auto_follow_state.runtime_phase {
        AutoFollowRuntimePhase::Idle => "idle".to_string(),
        AutoFollowRuntimePhase::Evaluating { .. } => {
            "automation is evaluating the next step".to_string()
        }
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
        AutoFollowRuntimePhase::Evaluating { .. } => {
            "automation evaluating the next step".to_string()
        }
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
