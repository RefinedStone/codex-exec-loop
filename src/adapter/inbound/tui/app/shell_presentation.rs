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
#[path = "shell_presentation/shell_core.rs"]
mod shell_core;
#[path = "shell_presentation/startup_banner.rs"]
mod startup_banner;
#[path = "shell_presentation/status_panels.rs"]
mod status_panels;
#[path = "shell_presentation/transcript_copy.rs"]
mod transcript_copy;

#[cfg(test)]
use runtime_status_copy::{auto_follow_prompt_lines, input_state_style};
use runtime_status_copy::{
    auto_follow_prompt_status_line, build_working_line, compact_inline_detail,
    inline_input_state_label, turn_status_label,
};

#[cfg(test)]
pub(super) use overlays::build_conversation_shell_frame_view;
pub(super) use overlays::{
    DirectionsMaintenanceOverlayView, HelpOverlayView, OverlayListView,
    PlanningDraftEditorOverlayView, PlanningInitOverlayView, QueueOverlayView, SessionOverlayView,
    StartupOverlayView, SupersessionOverlayView, TaskIntakeOverlayView,
    build_directions_maintenance_overlay_view, build_help_overlay_view,
    build_planning_draft_editor_overlay_view, build_planning_init_overlay_view,
    build_queue_overlay_view, build_session_overlay_view, build_startup_banner_lines,
    build_startup_overlay_view, build_supersession_overlay_view, build_task_intake_overlay_view,
};
#[cfg(test)]
pub(super) use prompt_composer::build_input_prompt_cursor_offset;
#[cfg(test)]
pub(super) use shell_core::{
    ConversationShellFrameView, ConversationShellView, TranscriptPanelView,
};
use shell_core::{ShellConversationState, ShellCorePresentationContext};
#[cfg(test)]
use startup_banner::build_startup_banner_lines_from_context;
pub(super) use startup_banner::startup_ascii_art_lines;
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
