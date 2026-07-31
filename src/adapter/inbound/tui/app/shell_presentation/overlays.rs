// overlays surface는 shell presentation 안에서 modal, help, directions, list projection을
// 한 boundary로 묶는다. shell frontend는 하위 파일 배치 대신 이 re-export surface만 본다.
#[path = "overlays/base.rs"]
mod base;

#[path = "overlays/activity.rs"]
mod activity;

#[path = "overlays/activity_diff.rs"]
mod activity_diff;

#[path = "overlays/directions.rs"]
mod directions;

#[path = "overlays/help.rs"]
mod help;

#[path = "overlays/list_projection.rs"]
mod list_projection;

#[path = "overlays/option_lines.rs"]
mod option_lines;

#[path = "overlays/popup.rs"]
mod popup;

// startup banner는 modal이 아니라 shell boot copy다. 그래도 shell presentation
// ownership에 속하므로 overlay surface에서 함께 공개한다.
pub(crate) use base::{StartupBannerFrameInput, build_startup_banner_lines};

pub(crate) use activity::{
    ActivityOverlayDocument, ActivityOverlayView, build_activity_overlay_list_view,
};

pub(crate) struct InlineDiffPreview {
    pub(crate) lines: Vec<ratatui::text::Line<'static>>,
    pub(crate) has_more: bool,
}

pub(crate) fn build_inline_diff_preview(text: &str, width: u16, height: u16) -> InlineDiffPreview {
    let page = activity_diff::build_bounded_diff_page(
        text,
        super::ProgressiveActivityPageCursor::at(0),
        width,
        height,
    );
    InlineDiffPreview {
        lines: page.lines,
        has_more: page.next_cursor.is_some(),
    }
}

// directions maintenance는 planning/task popup과 별도 흐름이다. active directions 상태를
// 점검하고 복구하는 overlay라 DTO와 builder를 독립 surface로 공개한다.
pub(crate) use directions::{
    DirectionsMaintenanceFrameInput, DirectionsMaintenanceOverlayView,
    build_directions_maintenance_overlay_view,
};

// help overlay는 read-only command catalog다. action popup과 분리해도 frontend는
// 같은 overlays namespace에서 view와 builder를 가져갈 수 있다.
pub(crate) use help::{HelpOverlayView, build_help_overlay_view};

// list projection은 queue, session, selection popup이 공유하는 renderer contract다.
// 개별 popup builder가 달라도 list rows는 같은 DTO shape로 downstream renderer에 들어간다.
pub(crate) use list_projection::{OverlayListEntryView, OverlayListView};

// modal popup variant는 popup module 안에 숨기고, shell frontend에는 builder와 view DTO만
// 공개한다. 이 경계를 유지해야 planning/session/queue popup layout 변경이 frontend import
// churn으로 번지지 않는다.
pub(crate) use popup::{
    LanguageSelectionFrameInput, LanguageSelectionOverlayView, ModelSelectionFrameInput,
    ModelSelectionOverlayView, ParallelPeekOverlayView, PlanningDraftEditorOverlayView,
    PlanningInitOverlayFrameInput, PlanningInitOverlayView, QueueOverlayView, ReviewsOverlayView,
    SessionOverlayView, StartupOverlayFrameInput, StartupOverlayView, SupersessionOverlayView,
    ViewSelectionFrameInput, ViewSelectionOverlayView, WorkCenterOverlayView,
    build_language_selection_overlay_view, build_model_selection_overlay_view,
    build_parallel_peek_overlay_view_from_snapshot,
    build_planning_draft_editor_overlay_view_from_state,
    build_planning_init_overlay_view_from_projection, build_queue_overlay_view_from_screen_model,
    build_reviews_overlay_view, build_session_overlay_view, build_startup_overlay_view,
    build_supersession_overlay_view, build_view_selection_overlay_view,
    build_work_center_overlay_view,
};
#[cfg(test)]
pub(crate) use popup::{build_parallel_peek_overlay_view, build_planning_init_overlay_view};
