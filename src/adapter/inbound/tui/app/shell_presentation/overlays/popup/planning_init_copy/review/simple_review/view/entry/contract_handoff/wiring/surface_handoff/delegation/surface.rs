// boundary module은 surface_handoff delegation의 마지막 경계 이름이다. 이 파일은 boundary와 실제 delegation
// implementation을 분리해 call chain의 의미를 더 잘게 드러낸다.
#[path = "surface/boundary.rs"]
mod boundary;
// delegation module은 boundary 아래에서 다음 function call을 수행하는 구현 위치다.
// surface index는 둘을 묶어 surface_handoff의 public wrapper로 제공한다.
#[path = "surface/delegation.rs"]
mod delegation;
// 반환 타입은 계속 공통 overlay view다. surface_handoff 아래의 boundary 세분화가 외부 contract를 바꾸지
// 않는다는 점을 보여 준다.
use super::super::super::super::super::super::super::super::super::super::PlanningInitOverlayView;
// copy는 이 surface 단계에서도 그대로 통과한다. 실제 section/contract 생성은 boundary 하위 delegation에서
// 시작된다.
use super::super::super::super::super::super::super::super::super::super::copy::PlanningSimpleReviewCopy;

// `build_simple_review_overlay_view_from_copy`는 surface_handoff delegation surface의 public facade다.
// boundary module로 넘겨 실제 변환 지점에 도달하게 한다.
pub(super) fn build_simple_review_overlay_view_from_copy(
    // `copy` ownership을 boundary로 이동시켜 pipeline을 한 방향으로 흐르게 한다.
    copy: PlanningSimpleReviewCopy,
) -> PlanningInitOverlayView {
    // boundary wrapper가 다음 책임 이름을 유지하므로, 이 surface index는 하위 구조를 숨긴다.
    boundary::build_simple_review_overlay_view_from_copy(copy)
}
