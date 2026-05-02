// simple_review popup의 실제 view 조립은 `simple_review/view.rs` 아래에 둔다. 이 파일은 review popup
// 상위 module에서 simple review variant로 들어오는 첫 관문이다.
#[path = "simple_review/view.rs"]
mod view;

// 최종 반환 타입은 planning init overlay 공통 view다. simple review도 다른 popup variant와 같은 renderer
// contract로 합류해야 하므로 이 타입을 사용한다.
use super::super::super::super::PlanningInitOverlayView;
// copy는 simple review 화면에 필요한 text와 option 상태를 담은 presentation input이다.
// 이 module은 copy를 받아 view 조립 하위 단계로 넘긴다.
use super::super::super::copy::PlanningSimpleReviewCopy;

// `build_simple_review_overlay_view`는 review popup 밖에서 호출하는 simple review variant의 공개 entry point다.
// 상위 code는 view 하위 module의 세부 assembly 단계를 몰라도 된다.
pub(super) fn build_simple_review_overlay_view(
    // `copy` ownership을 넘겨 하위 builder가 line vectors와 labels를 필요한 section으로 분해하게 한다.
    copy: PlanningSimpleReviewCopy,
) -> PlanningInitOverlayView {
    // 이 wrapper는 상위 review namespace와 하위 view namespace 사이의 이름 안정성을 제공한다.
    // 실제 조립 순서는 `view` module에서 이어진다.
    view::build_simple_review_overlay_view(copy)
}
