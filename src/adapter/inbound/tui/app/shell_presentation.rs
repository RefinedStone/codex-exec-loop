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
const INLINE_TAIL_THREAD_LABEL_LIMIT: usize = 20;
#[cfg(test)]
const FOOTER_MODE_LABEL_LIMIT: usize = 16;
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

#[path = "shell_presentation/capability_copy.rs"]
mod capability_copy;
#[path = "shell_presentation/capability_projection.rs"]
mod capability_projection;
#[path = "shell_presentation/overlays.rs"]
mod overlays;
#[path = "shell_presentation/prompt_composer.rs"]
mod prompt_composer;
#[path = "shell_presentation/session_browser.rs"]
mod session_browser;
#[cfg(test)]
#[path = "shell_presentation/shell_copy.rs"]
mod shell_copy;
#[path = "shell_presentation/status_panels.rs"]
mod status_panels;

use capability_projection::recent_session_status_label;

#[cfg(test)]
pub(super) use overlays::build_conversation_shell_frame_view;
pub(super) use overlays::{
    AutomationOverlayView, DirectionsMaintenanceOverlayView, HelpOverlayView, OverlayListView,
    PlanningDraftEditorOverlayView, PlanningInitOverlayView, QueueOverlayView, SessionOverlayView,
    StartupOverlayView, SupersessionOverlayView, TaskIntakeOverlayView,
    build_automation_overlay_view, build_directions_maintenance_overlay_view,
    build_help_overlay_view, build_planning_draft_editor_overlay_view,
    build_planning_init_overlay_view, build_queue_overlay_view, build_session_overlay_view,
    build_startup_banner_lines, build_startup_overlay_view, build_supersession_overlay_view,
    build_task_intake_overlay_view,
};
#[cfg(test)]
pub(super) use prompt_composer::{build_input_prompt_cursor_offset, build_ready_input_lines};
pub(super) use status_panels::InlineTailView;

#[cfg(test)]
#[allow(clippy::too_many_arguments)]
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
            lines.push(Line::from(format!("  {}", expand_tui_tabs(text_line))));
        }
        if show_debug_details && let Some(debug_detail) = message.debug_detail.as_deref() {
            for detail_line in debug_detail.lines() {
                lines.push(Line::from(Span::styled(
                    format!("  {}", expand_tui_tabs(detail_line)),
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

fn expand_tui_tabs(text: &str) -> String {
    text.replace('\t', "    ")
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

fn build_startup_check_lines(app: &NativeTuiApp) -> Vec<Line<'static>> {
    capability_projection::build_startup_check_lines(app)
}

fn build_startup_overlay_summary_lines(app: &NativeTuiApp) -> Vec<Line<'static>> {
    capability_projection::build_startup_overlay_summary_lines(app)
}

fn build_startup_check_lines_from_state(startup_state: &StartupState) -> Vec<Line<'static>> {
    capability_projection::build_startup_check_lines_from_state(startup_state)
}

fn build_startup_warning_lines(app: &NativeTuiApp) -> Vec<Line<'static>> {
    capability_projection::build_startup_warning_lines(app)
}

fn build_startup_warning_lines_from_state(startup_state: &StartupState) -> Vec<Line<'static>> {
    capability_projection::build_startup_warning_lines_from_state(startup_state)
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

    let interrupt_label = conversation.interrupt_support_label();
    match conversation.input_state {
        ConversationInputState::SubmittingTurn => {
            Some(format!("starting turn / interrupt {interrupt_label}"))
        }
        ConversationInputState::StreamingTurn => {
            if conversation.live_agent_message.is_some() {
                Some(format!("turn running / interrupt {interrupt_label}"))
            } else {
                Some(format!(
                    "waiting for response / interrupt {interrupt_label}"
                ))
            }
        }
        ConversationInputState::DraftReady | ConversationInputState::ReadyToContinue => None,
    }
}

fn auto_follow_working_detail(conversation: &ConversationViewModel) -> String {
    let max_auto_turns = conversation.auto_follow_state.max_auto_turns_label();
    let interrupt_label = conversation.interrupt_support_label();
    match &conversation.auto_follow_state.runtime_phase {
        AutoFollowRuntimePhase::Idle => "idle".to_string(),
        AutoFollowRuntimePhase::Evaluating { .. } => "evaluating next auto follow-up".to_string(),
        AutoFollowRuntimePhase::Queued { turn_index, .. } => {
            format!("auto turn {turn_index}/{max_auto_turns} queued for submission")
        }
        AutoFollowRuntimePhase::Submitting { turn_index, .. } => {
            format!(
                "auto turn {turn_index}/{max_auto_turns} starting / interrupt {interrupt_label}"
            )
        }
        AutoFollowRuntimePhase::Running { turn_index, .. } => {
            format!("auto turn {turn_index}/{max_auto_turns} running / interrupt {interrupt_label}")
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
            "auto turn 2/infinite running / interrupt unsupported"
        );
        assert_eq!(
            auto_follow_prompt_status_line(&conversation, true).as_deref(),
            Some("prompt: auto turn 2/infinite running  |  type now, Enter when idle")
        );
    }
}
