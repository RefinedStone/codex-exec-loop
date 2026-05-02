// 학습 주석: delegation 단계는 boundary에서 받은 copy를 다시 상위 surface implementation으로 넘기고,
// 반환되는 값은 계속 공통 overlay view입니다.
use super::super::super::super::super::super::super::super::super::super::super::PlanningInitOverlayView;
// 학습 주석: copy는 이 layer에서도 해석되지 않습니다. wrapper 계층이 많아도 ownership은 한 방향으로
// 이동하며, 실제 contract 생성은 target surface에 도달한 뒤 일어납니다.
use super::super::super::super::super::super::super::super::super::super::super::copy::PlanningSimpleReviewCopy;
// 학습 주석: 이 relative path의 surface는 contract handoff wiring의 surface implementation입니다.
// surface_handoff branch가 결국 같은 surface builder로 합류하는 연결점입니다.
use super::super::super::super::surface;

// 학습 주석: 이 함수는 boundary 아래 delegation의 entry입니다. wrapper 이름을 보존해 call chain에서
// "surface_handoff -> boundary -> delegation -> surface" 흐름을 추적할 수 있습니다.
pub(super) fn build_simple_review_overlay_view_from_copy(
    // 학습 주석: `copy`를 surface builder로 이동시켜 이 delegation layer가 상태를 남기지 않게 합니다.
    copy: PlanningSimpleReviewCopy,
) -> PlanningInitOverlayView {
    // 학습 주석: surface implementation에 합류하면 contract 생성과 final assembly로 이어집니다.
    surface::build_simple_review_overlay_view_from_copy(copy)
}
