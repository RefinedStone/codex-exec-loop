// 학습 주석: surface module은 wiring 단계에서 실제 copy-to-view 위임을 수행하는 하위 surface입니다.
// 이 index는 surface와 handoff를 나누어 각 단계의 이름을 파일 구조에 남깁니다.
#[path = "wiring/surface.rs"]
mod surface;
// 학습 주석: surface_handoff는 wiring entry에서 surface delegation으로 넘어가는 한 단계 더 좁은
// adapter입니다. 깊은 파일 구조는 단순 wrapper라도 handoff 책임을 명명하기 위해 유지됩니다.
#[path = "wiring/surface_handoff.rs"]
mod surface_handoff;

// 학습 주석: wiring의 반환 타입은 최종 overlay view입니다. 즉 이 계층은 단순 data 변환이 아니라
// renderer가 받을 완성된 popup view까지 이어지는 pipeline을 대표합니다.
use super::super::super::super::super::super::super::PlanningInitOverlayView;
// 학습 주석: input copy는 아직 section이나 contract로 나뉘지 않은 simple review presentation source입니다.
use super::super::super::super::super::super::super::copy::PlanningSimpleReviewCopy;

// 학습 주석: `build_simple_review_overlay_view_from_copy`는 contract handoff wiring의 public wrapper입니다.
// 이름은 상위 함수와 같지만 위치가 달라, call stack에서 어느 handoff layer를 지나고 있는지 보여 줍니다.
pub(super) fn build_simple_review_overlay_view_from_copy(
    // 학습 주석: `copy` ownership은 이 wiring entry에서 하위 surface_handoff로 그대로 이동합니다.
    // 이후 단계에서 contract 생성과 final assembly가 이어집니다.
    copy: PlanningSimpleReviewCopy,
) -> PlanningInitOverlayView {
    // 학습 주석: surface_handoff에 위임해 이 파일은 wiring namespace의 입구만 담당합니다.
    surface_handoff::build_simple_review_overlay_view_from_copy(copy)
}
