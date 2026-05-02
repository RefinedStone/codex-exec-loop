// surface_handoff의 delegation module은 wiring entry와 surface implementation 사이를 연결한다.
// 이 층은 handoff라는 이름으로 call chain의 전환점을 드러낸다.
#[path = "surface_handoff/delegation.rs"]
mod delegation;

// 반환 타입은 계속 공통 overlay view다. handoff 계층이 아무리 깊어져도 renderer contract가 유지된다는
// 점이 이 import로 드러난다.
use super::super::super::super::super::super::super::super::PlanningInitOverlayView;
// copy는 아직 raw presentation input이다. 이 handoff 함수는 값을 해석하지 않고 다음 delegation layer로
// 넘기는 연결점이다.
use super::super::super::super::super::super::super::super::copy::PlanningSimpleReviewCopy;

// `build_simple_review_overlay_view_from_copy`는 surface_handoff 단계의 facade다. 같은 signature를 유지해
// 상위 wiring과 하위 delegation을 쉽게 교체하거나 분리할 수 있다.
pub(super) fn build_simple_review_overlay_view_from_copy(
    // `copy`를 소유한 채로 delegation에 넘겨, 불필요한 clone 없이 pipeline을 이어간다.
    copy: PlanningSimpleReviewCopy,
) -> PlanningInitOverlayView {
    // 실제 다음 target은 delegation module이 결정한다. 이 파일은 surface handoff라는 책임 이름을 call chain에
    // 남기는 역할이다.
    delegation::build_simple_review_overlay_view_from_copy(copy)
}
