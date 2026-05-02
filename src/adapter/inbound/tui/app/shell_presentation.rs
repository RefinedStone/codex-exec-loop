use super::*;
use crate::domain::planning::{
    PlanningValidationSeverity, PriorityQueueSkippedTask, PriorityQueueTask,
};
use crate::domain::text::compact_whitespace_detail;

/*
shell presentation은 TUI app state를 직접 그리는 마지막 어댑터 계층이다. 이 파일은
실제 렌더링 알고리즘을 담기보다 하위 projection 모듈을 한 namespace로 묶는 facade 역할을
한다. 기존 call site가 `shell_presentation::...` 경계를 유지하도록 re-export와 얇은 wrapper를
여기에 남기고, 복잡한 copy/overlay/status 계산은 파일별 하위 모듈로 분리한다.
*/

// footer는 화면 하단의 넓은 status area이고 inline tail은 입력 프롬프트 옆의 매우 좁은
// 영역이다. 같은 domain detail이라도 두 영역에서 읽을 수 있는 길이가 달라 별도 limit을 둔다.
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

// 각 하위 모듈은 presentation의 한 관심사를 맡는다. `#[path]`를 명시해 파일 시스템은
// `shell_presentation/` 디렉터리로 나누되, Rust module API는 이 facade 아래로 모은다.
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

// test-only import는 과거 snapshot/contract test가 이 facade의 private helper를 직접 호출한
// 구조를 유지하기 위한 것이다. production surface에는 필요한 projection만 re-export한다.
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
use runtime_status_copy::{auto_follow_prompt_lines, input_state_style};
use runtime_status_copy::{
    auto_follow_prompt_status_line, build_working_line, compact_inline_detail,
    inline_input_state_label, turn_status_label,
};
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
    // 테스트는 footer 입력 조합을 직접 만든다. production builder는 app state에서 값을
    // 모으지만, 이 wrapper는 projection 순수 함수만 검증할 수 있게 같은 경로로 위임한다.
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
    // live transcript projection은 shell footer와 inline transcript가 공유하므로 facade에서
    // test 접근점을 제공해 두 경로의 copy가 갈라지지 않게 한다.
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
    // renderer는 폭만 알고 status panel의 세부 우선순위는 알지 못한다. content_width를
    // 넘겨 presentation 쪽에서 어떤 상태를 남기고 줄일지 결정한다.
    status_panels::build_inline_tail_view(app, content_width)
}

pub(super) fn build_inline_live_transcript_lines(app: &NativeTuiApp) -> Vec<Line<'static>> {
    // loading/failed 상태에서는 live agent message가 존재할 수 없으므로 빈 view를 반환한다.
    // Ready에서만 cached conversation과 live streaming tail을 같은 panel 규칙으로 합친다.
    let ConversationState::Ready(conversation) = &app.conversation_state else {
        return Vec::new();
    };
    status_panels::current_live_agent_lines(conversation).unwrap_or_default()
}

#[cfg(test)]
fn build_conversation_lines_with_context(
    context: &ShellCorePresentationContext<'_>,
) -> Vec<Line<'static>> {
    // startup banner는 conversation transcript보다 우선한다. 아직 history가 준비되지 않은
    // 초기 화면에서는 capability/startup 상태가 사용자의 첫 판단 기준이기 때문이다.
    if let Some(startup_banner_lines) = build_startup_banner_lines_from_context(context, None) {
        return startup_banner_lines;
    }

    // shell_core의 context enum을 transcript copy로 낮추는 마지막 단계다. debug mode는
    // 원본 message를 다시 format하고, 일반 모드는 controller가 미리 캐시한 line을 사용해
    // 매 frame마다 transcript를 재계산하지 않는다.
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
    // startup/capability projection은 overlay와 footer가 함께 쓰는 copy라 facade에서
    // 이름을 보존하고 하위 모듈로 위임한다.
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
