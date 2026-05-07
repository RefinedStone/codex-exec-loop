// surface module은 wiring 단계에서 실제 copy-to-view 위임을 수행하는 하위 surface다.
#[path = "wiring/surface.rs"]
mod surface;

// wiring의 반환 타입은 최종 overlay view다. 즉 이 계층은 단순 data 변환이 아니라 renderer가 받을 완성된
// popup view까지 이어지는 pipeline을 대표한다.
use super::super::super::super::super::super::super::PlanningInitOverlayView;
// input copy는 아직 section이나 contract로 나뉘지 않은 simple review presentation source다.
use super::super::super::super::super::super::super::copy::PlanningSimpleReviewCopy;

// `build_simple_review_overlay_view_from_copy`는 contract wiring의 public wrapper다.
pub(super) fn build_simple_review_overlay_view_from_copy(
    // `copy` ownership은 이 wiring entry에서 하위 surface로 그대로 이동한다.
    // 이후 단계에서 contract 생성과 final assembly가 이어진다.
    copy: PlanningSimpleReviewCopy,
) -> PlanningInitOverlayView {
    // surface에 위임해 이 파일은 wiring namespace의 입구만 담당한다.
    surface::build_simple_review_overlay_view_from_copy(copy)
}
