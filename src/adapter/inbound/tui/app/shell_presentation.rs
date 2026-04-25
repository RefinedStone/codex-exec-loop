pub(super) use super::planning::{build_automation_preview_lines, build_automation_status_lines};
use super::*;
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
    _    _  __ ____    _
   / \  | |/ /|  _ \  / \
  / _ \ | ' / | |_) |/ _ \
 / ___ \| . \ |  _ </ ___ \
/_/   \_\_|\_\|_| \_\_/   \_\
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
#[path = "shell_presentation/runtime_status_copy.rs"]
mod runtime_status_copy;
#[path = "shell_presentation/session_browser.rs"]
mod session_browser;
#[cfg(test)]
#[path = "shell_presentation/shell_copy.rs"]
mod shell_copy;
#[path = "shell_presentation/status_panels.rs"]
mod status_panels;
#[path = "shell_presentation/transcript_copy.rs"]
mod transcript_copy;

use capability_projection::recent_session_status_label;
#[cfg(test)]
use runtime_status_copy::{auto_follow_prompt_lines, input_state_style};
use runtime_status_copy::{
    auto_follow_prompt_status_line, build_working_line, compact_inline_detail,
    inline_input_state_label, turn_status_label,
};

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
pub(super) use runtime_status_copy::format_elapsed;
pub(super) use status_panels::InlineTailView;
pub(super) use transcript_copy::{format_conversation_lines, format_conversation_lines_with_debug};

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

pub(super) fn build_inline_live_transcript_lines(app: &NativeTuiApp) -> Vec<Line<'static>> {
    let ConversationState::Ready(conversation) = &app.conversation_state else {
        return Vec::new();
    };
    status_panels::current_live_agent_lines(conversation).unwrap_or_default()
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

pub(super) fn build_automation_key_lines(app: &NativeTuiApp) -> Vec<Line<'static>> {
    if app.is_max_auto_turns_editing() {
        return vec![
            AkraTheme::key_line("Type the new max-turn value directly. Backspace deletes."),
            AkraTheme::key_line("Enter: save max turns    Esc/Ctrl+C: cancel edit"),
            AkraTheme::key_line("Use a whole number greater than 0, or type infinite."),
        ];
    }

    if app.is_stop_keyword_editing() {
        return vec![
            AkraTheme::key_line("Type the new stop keyword directly. Backspace deletes."),
            AkraTheme::key_line("Enter: save stop keyword    Esc/Ctrl+C: cancel edit"),
            AkraTheme::key_line("Use letters, numbers, or underscores only."),
        ];
    }

    vec![
        AkraTheme::key_line("PageUp/PageDown or Ctrl+u/Ctrl+d: scroll preview"),
        AkraTheme::key_line(
            "Ctrl+a: automation on/off    Ctrl+l: edit max turns    Ctrl+g: edit stop keyword",
        ),
        AkraTheme::key_line(
            "Ctrl+k: stop rule on/off    Ctrl+n: no-file stop    Ctrl+b: planner detail",
        ),
        AkraTheme::key_line("Enter/Esc/Ctrl+C: close"),
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
