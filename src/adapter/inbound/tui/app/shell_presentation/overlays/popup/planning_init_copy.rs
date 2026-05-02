// 학습 주석: existing_workspace module은 이미 accepted planning state가 있는 workspace를 설명하는 init overlay를 만듭니다.
#[path = "planning_init_copy/existing_workspace.rs"]
mod existing_workspace;
// 학습 주석: review module은 simple scaffold review와 manual editor 안내 variant를 담당합니다.
#[path = "planning_init_copy/review.rs"]
mod review;
// 학습 주석: selection module은 mode/detail 선택처럼 아직 planning artifacts를 만들기 전의 wizard screens를 담당합니다.
#[path = "planning_init_copy/selection.rs"]
mod selection;

// 학습 주석: selection builders는 UI state enum 값만 받아 현재 highlight/description을 결정합니다.
use super::super::super::super::{PlanningInitDetailSelection, PlanningInitModeSelection};
// 학습 주석: 모든 planning init copy builders는 renderer가 소비하는 공통 overlay DTO를 반환합니다.
use super::super::PlanningInitOverlayView;
// 학습 주석: existing workspace와 simple review는 app/runtime state에서 추출한 copy DTO를 view builder에 넘깁니다.
use super::copy::{PlanningExistingWorkspaceCopy, PlanningSimpleReviewCopy};

// 학습 주석: existing workspace facade entry입니다. router는 copy extraction을 끝낸 뒤 이 함수에 copy를 넘기고,
// 이 함수는 구체 view assembly module path를 숨깁니다.
pub(super) fn build_existing_workspace_overlay_view(
    // 학습 주석: copy에는 workspace path, runtime state label, queue/failure summaries가 이미 들어 있습니다.
    copy: PlanningExistingWorkspaceCopy,
) -> PlanningInitOverlayView {
    // 학습 주석: line construction은 existing_workspace module에 위임해 이 파일은 facade surface만 유지합니다.
    existing_workspace::build_existing_workspace_overlay_view(copy)
}

// 학습 주석: mode selection facade entry입니다. mode selection은 app 전체가 아니라 selected enum 하나만
// 필요하므로 builder dependency가 작습니다.
pub(super) fn build_mode_selection_overlay_view(
    // 학습 주석: 현재 선택된 init mode가 option line 강조와 설명을 결정합니다.
    selected_mode: PlanningInitModeSelection,
) -> PlanningInitOverlayView {
    // 학습 주석: selection module이 common `PlanningInitOverlayView`로 조립합니다.
    selection::build_mode_selection_overlay_view(selected_mode)
}

// 학습 주석: detail selection facade entry입니다. detail-mode authoring을 택한 뒤 어떤 detail path로
// 들어갈지 보여 주는 overlay를 만듭니다.
pub(super) fn build_detail_selection_overlay_view(
    // 학습 주석: selected_detail은 현재 cursor/highlight 역할을 합니다.
    selected_detail: PlanningInitDetailSelection,
) -> PlanningInitOverlayView {
    // 학습 주석: detail-specific line text는 selection module에 남깁니다.
    selection::build_detail_selection_overlay_view(selected_detail)
}

// 학습 주석: simple review facade entry입니다. router가 app state에서 `PlanningSimpleReviewCopy`를 만든 뒤
// 이 함수로 넘기면 review module이 section collection과 final assembly를 진행합니다.
pub(super) fn build_simple_review_overlay_view(
    // 학습 주석: copy ownership을 review path로 넘겨 staged draft metadata가 중복 조회되지 않게 합니다.
    copy: PlanningSimpleReviewCopy,
) -> PlanningInitOverlayView {
    // 학습 주석: review module은 simple review의 header/summary/options/status/key 영역을 모두 조립합니다.
    review::build_simple_review_overlay_view(copy)
}

// 학습 주석: manual editor facade entry입니다. manual editor 안내는 고정 copy로 구성되므로 별도 입력이 없습니다.
pub(super) fn build_manual_editor_overlay_view() -> PlanningInitOverlayView {
    // 학습 주석: review module 안의 manual editor builder를 통해 같은 planning init view shape를 반환합니다.
    review::build_manual_editor_overlay_view()
}
