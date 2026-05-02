// 학습 주석: base module은 shell frame과 startup banner처럼 모든 overlay 주변을 감싸는 공통
// presentation 조각을 제공합니다. overlays index는 이 공통 기반과 개별 overlay를 한 surface로 묶습니다.
#[path = "overlays/base.rs"]
mod base;

// 학습 주석: directions module은 방향성 유지보수 overlay를 담당합니다. planning/task popup과 별도
// overlay인 이유는 active directions 상태를 독립적으로 점검하고 복구하기 때문입니다.
#[path = "overlays/directions.rs"]
mod directions;

// 학습 주석: help module은 shell help overlay의 view DTO와 builder를 담습니다. 다른 overlay와 달리
// action을 수행하지 않고 사용 가능한 조작을 설명하는 read-only presentation입니다.
#[path = "overlays/help.rs"]
mod help;

// 학습 주석: list_projection module은 여러 overlay가 공유하는 list view DTO를 제공합니다. queue,
// session, task selection처럼 항목 목록을 그릴 때 같은 projection shape를 씁니다.
#[path = "overlays/list_projection.rs"]
mod list_projection;

// 학습 주석: option_lines module은 선택지/명령 안내를 line 단위로 만드는 공통 helper입니다.
#[path = "overlays/option_lines.rs"]
mod option_lines;

// 학습 주석: popup module은 session, queue, planning init 등 modal 성격의 overlay를 묶습니다.
// overlays index는 popup 내부 세부 variant를 한 번 더 re-export합니다.
#[path = "overlays/popup.rs"]
mod popup;

// 학습 주석: shell frame view builder는 현재 test에서만 직접 검증합니다. production renderer는 더 높은
// 수준의 frontend path를 통해 호출하므로 test cfg 안에만 re-export합니다.
#[cfg(test)]
pub(crate) use base::build_conversation_shell_frame_view;
// 학습 주석: startup banner는 shell presentation의 기본 안내 line입니다. shell entry가 banner만
// 따로 필요로 하므로 overlays surface에서 바로 가져가게 합니다.
pub(crate) use base::build_startup_banner_lines;
// 학습 주석: directions overlay type과 builder를 함께 공개해 caller가 view DTO와 생성 함수를 같은
// namespace에서 import하게 합니다.
pub(crate) use directions::{
    DirectionsMaintenanceOverlayView, build_directions_maintenance_overlay_view,
};
// 학습 주석: help overlay도 DTO와 builder를 함께 re-export해 shell frontend가 하위 파일 구조를 몰라도
// 도움말 overlay를 생성할 수 있게 합니다.
pub(crate) use help::{HelpOverlayView, build_help_overlay_view};
// 학습 주석: list projection DTO는 여러 popup/list overlay가 공유하는 작은 contract이므로 최상위
// overlay surface에서 공개합니다.
pub(crate) use list_projection::{OverlayListEntryView, OverlayListView};
// 학습 주석: popup re-export는 modal overlay 전체를 shell frontend가 한 곳에서 가져오게 하는
// public surface입니다. 구체 variant의 module path는 popup 내부에 숨깁니다.
pub(crate) use popup::{
    PlanningDraftEditorOverlayView, PlanningInitOverlayView, QueueOverlayView, SessionOverlayView,
    StartupOverlayView, SupersessionOverlayView, TaskIntakeOverlayView,
    build_planning_draft_editor_overlay_view, build_planning_init_overlay_view,
    build_queue_overlay_view, build_session_overlay_view, build_startup_overlay_view,
    build_supersession_overlay_view, build_task_intake_overlay_view,
};
