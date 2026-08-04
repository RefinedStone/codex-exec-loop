use super::*;
use crate::domain::planning::PlanningValidationSeverity;
use crate::domain::text::compact_whitespace_detail;

/*
shell presentation은 terminal frame capture가 만든 typed input과 owned screen model만
그리는 마지막 어댑터 계층이다. 이 파일은 실제 렌더링 알고리즘을 담기보다 하위 projection
모듈을 한 namespace로 묶는 facade 역할을 한다. 기존 call site가
`shell_presentation::...` 경계를 유지하도록 re-export와 얇은 wrapper를 여기에 남기고,
복잡한 copy/overlay/status 계산은 파일별 하위 모듈로 분리한다.
*/

// footer는 화면 하단의 넓은 status area이고 shell tail은 입력 프롬프트 옆의 매우 좁은
// 영역이다. 같은 domain detail이라도 두 영역에서 읽을 수 있는 길이가 달라 별도 limit을 둔다.
const FOOTER_NOTICE_DETAIL_LIMIT: usize = 56;
const SHELL_TAIL_STATUS_DETAIL_LIMIT: usize = 44;
const SHELL_TAIL_NOTICE_DETAIL_LIMIT: usize = 40;
const SHELL_TAIL_PLANNING_DETAIL_LIMIT: usize = 36;
const SHELL_TAIL_AUTO_FOLLOW_DETAIL_LIMIT: usize = 18;
const INLINE_COMMAND_PALETTE_VISIBLE_LIMIT: usize = 4;
pub(super) const QUEUE_INSPECTION_TASK_LIMIT: usize = 2;
pub(super) const QUEUE_INSPECTION_PROPOSAL_LIMIT: usize = 1;
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
#[path = "shell_presentation/shell_core.rs"]
mod shell_core;
#[path = "shell_presentation/startup_banner.rs"]
mod startup_banner;
#[path = "shell_presentation/status_panels.rs"]
mod status_panels;
#[path = "shell_presentation/terminal_text.rs"]
mod terminal_text;
#[path = "shell_presentation/transcript_copy.rs"]
mod transcript_copy;

#[cfg(test)]
pub(super) use overlays::build_planning_init_overlay_view;
pub(super) use overlays::build_queue_overlay_view_from_screen_model;
pub(super) use overlays::{
    ActivityOverlayDocument, ActivityOverlayView, DirectionsMaintenanceFrameInput,
    DirectionsMaintenanceOverlayView, HelpOverlayView, LanguageSelectionFrameInput,
    LanguageSelectionOverlayView, ModelSelectionFrameInput, ModelSelectionOverlayView,
    OverlayListView, ParallelPeekOverlayView, PlanningDraftEditorOverlayView,
    PlanningInitOverlayFrameInput, PlanningInitOverlayView, QueueOverlayView, ReviewsOverlayView,
    SessionOverlayView, StartupBannerFrameInput, StartupOverlayFrameInput, StartupOverlayView,
    SupersessionOverlayView, ViewSelectionFrameInput, ViewSelectionOverlayView,
    WorkCenterOverlayView, build_activity_overlay_list_view,
    build_directions_maintenance_overlay_view, build_help_overlay_view,
    build_language_selection_overlay_view, build_model_selection_overlay_view,
    build_parallel_peek_overlay_view_from_snapshot,
    build_planning_draft_editor_overlay_view_from_state,
    build_planning_init_overlay_view_from_projection, build_reviews_overlay_view,
    build_session_overlay_view, build_startup_banner_lines, build_startup_overlay_view,
    build_supersession_overlay_view, build_view_selection_overlay_view,
    build_work_center_overlay_view,
};
use runtime_status_copy::{build_working_line, compact_shell_detail};
pub(super) use shell_core::QueueMutationTailState;
use shell_core::ShellConversationState;
pub(super) use shell_core::{
    ConversationComposerScreenModel, ConversationProjectionFrameInput,
    ConversationProjectionSample, ConversationRuntimeStatusScreenModel,
    ConversationScreenFrameInput, ConversationScreenModel, MAX_GITHUB_REVIEW_NOTICE_LEN,
    ParallelPanelProjectionSample, TurnSteerConfirmationScreenModel,
    conversation_startup_screen_is_active, presentation_workspace_directory,
    shell_conversation_state,
};
pub(super) use startup_banner::startup_ascii_art_lines;
pub(super) use status_panels::{ShellTailView, composer_inner_width};
pub(super) use transcript_copy::{
    ConversationTranscriptLineInteraction, ConversationTranscriptLineSurface,
    ConversationTranscriptView, format_fullscreen_conversation_transcript_view,
};

pub(super) fn build_shell_tail_view(
    screen_model: &ConversationScreenModel<'_>,
    content_width: u16,
) -> ShellTailView {
    // renderer는 폭만 알고 status panel의 세부 우선순위는 알지 못한다. content_width를
    // 넘겨 presentation 쪽에서 어떤 상태를 남기고 줄일지 결정한다.
    status_panels::build_shell_tail_view(screen_model, content_width)
}

pub(super) fn build_operator_diagnostic_lines(
    screen_model: &ConversationScreenModel<'_>,
) -> Vec<Line<'static>> {
    status_panels::build_operator_diagnostic_lines(screen_model)
}

#[cfg(test)]
pub(super) fn build_queue_overlay_view(app: &NativeTuiApp) -> QueueOverlayView {
    build_queue_overlay_view_from_screen_model(app.queue_overlay_screen_model())
}

use capability_projection::{
    build_startup_check_lines, build_startup_overlay_summary_lines, build_startup_warning_lines,
};
