// 학습 주석: surface delegation module은 이 layer의 실제 위임 함수를 담습니다. surface index는
// delegation 파일을 감싸 public 함수명을 안정적으로 유지합니다.
#[path = "surface/delegation.rs"]
mod delegation;

// 학습 주석: surface 단계도 최종 overlay view를 반환합니다. 이 타입이 유지되기 때문에 caller는
// 깊은 delegation 계층을 지나도 반환 계약이 바뀌지 않는다고 믿을 수 있습니다.
use super::super::super::super::super::super::super::super::PlanningInitOverlayView;
// 학습 주석: copy는 이 surface에서 해석하지 않고 delegation으로 넘깁니다. 의미 있는 변환은
// 더 아래 contract/assembly 함수들이 담당합니다.
use super::super::super::super::super::super::super::super::copy::PlanningSimpleReviewCopy;

// 학습 주석: `build_simple_review_overlay_view_from_copy`는 wiring/surface namespace의 entry입니다.
// 얇은 wrapper지만 call chain을 따라가면 copy가 어떤 layer를 통과하는지 명확히 드러납니다.
pub(super) fn build_simple_review_overlay_view_from_copy(
    // 학습 주석: copy는 값으로 받아 delegation에 그대로 넘깁니다. 이 layer는 ownership을 변경하거나
    // 복제하지 않습니다.
    copy: PlanningSimpleReviewCopy,
) -> PlanningInitOverlayView {
    // 학습 주석: delegation module이 다음 handoff target을 결정하므로, surface index는 위임 경계만
    // 명명합니다.
    delegation::build_simple_review_overlay_view_from_copy(copy)
}
