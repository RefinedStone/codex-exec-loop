// link layer도 반환 타입을 공통 overlay view로 유지한다. 이름은 작지만, handoff branch가 surface
// implementation으로 이어지는 마지막 링크를 표현한다.
use super::super::super::super::super::super::super::super::super::super::super::super::PlanningInitOverlayView;
// copy는 link layer에서 여전히 raw presentation input이다. 이 파일은 값의 의미를 바꾸지 않고 target
// surface로 넘기는 데 집중한다.
use super::super::super::super::super::super::super::super::super::super::super::super::copy::PlanningSimpleReviewCopy;
// 이 surface import는 link가 도달해야 할 실제 surface builder다. delegation chain이 길어도 마지막에는
// 같은 copy-to-view implementation으로 합류한다.
use super::super::super::super::super::surface;

// `build_simple_review_overlay_view_from_copy`는 link layer의 facade다. 파일 경계가 작은 이유는 review popup
// assembly trace에서 각 handoff 이름을 명확히 남기기 위해서다.
pub(super) fn build_simple_review_overlay_view_from_copy(
    // `copy`는 target surface로 move된다. link layer는 validation, formatting, contract 생성 어느 것도
    // 수행하지 않는다.
    copy: PlanningSimpleReviewCopy,
) -> PlanningInitOverlayView {
    // 실제 조립 구현에 합류하는 마지막 위임이다.
    surface::build_simple_review_overlay_view_from_copy(copy)
}
