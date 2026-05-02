// delegation 단계는 surface_handoff에서 실제 surface boundary로 넘어가는 중간 이름이다.
// path attribute로 다음 surface module을 이 delegation namespace 아래에 연결한다.
#[path = "delegation/surface.rs"]
mod surface;

// 반환 타입은 여전히 최종 planning init overlay view다. 이 wrapper는 변환하지 않고 call chain의 delegation
// 단계만 명명한다.
use super::super::super::super::super::super::super::super::super::PlanningInitOverlayView;
// copy는 simple review presentation source이며, 이 단계에서는 아직 contract로 바뀌지 않았다.
// 다음 surface layer까지 ownership을 그대로 넘긴다.
use super::super::super::super::super::super::super::super::super::copy::PlanningSimpleReviewCopy;

// 이 함수는 surface_handoff delegation의 public entry다. 같은 signature를 반복해 상위 layer가 하위 구조를
// 몰라도 copy-to-view pipeline을 계속 호출할 수 있게 한다.
pub(super) fn build_simple_review_overlay_view_from_copy(
    // `copy`는 여기서 소비되어 다음 surface 단계로 이동한다. 이 layer는 clone이나 field access를 하지
    // 않으므로 ownership 흐름이 단순하다.
    copy: PlanningSimpleReviewCopy,
) -> PlanningInitOverlayView {
    // surface module에 위임해 실제 boundary/delegation 분리는 다음 파일에서 이어진다.
    surface::build_simple_review_overlay_view_from_copy(copy)
}
