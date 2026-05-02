// surface delegation module은 이 layer의 실제 위임 함수를 담는다. surface index는 delegation 파일을 감싸
// public 함수명을 안정적으로 유지한다.
#[path = "surface/delegation.rs"]
mod delegation;

// surface 단계도 최종 overlay view를 반환한다. 이 타입이 유지되기 때문에 caller는 깊은 delegation 계층을
// 지나도 반환 계약이 바뀌지 않는다고 믿을 수 있다.
use super::super::super::super::super::super::super::super::PlanningInitOverlayView;
// copy는 이 surface에서 해석하지 않고 delegation으로 넘긴다. 의미 있는 변환은 더 아래 contract/assembly
// 함수들이 담당한다.
use super::super::super::super::super::super::super::super::copy::PlanningSimpleReviewCopy;

// `build_simple_review_overlay_view_from_copy`는 wiring/surface namespace의 entry다.
// 얇은 wrapper지만 call chain을 따라가면 copy가 어떤 layer를 통과하는지 명확히 드러난다.
pub(super) fn build_simple_review_overlay_view_from_copy(
    // copy는 값으로 받아 delegation에 그대로 넘긴다. 이 layer는 ownership을 변경하거나 복제하지 않는다.
    copy: PlanningSimpleReviewCopy,
) -> PlanningInitOverlayView {
    // delegation module이 다음 handoff target을 결정하므로, surface index는 위임 경계만 명명한다.
    delegation::build_simple_review_overlay_view_from_copy(copy)
}
