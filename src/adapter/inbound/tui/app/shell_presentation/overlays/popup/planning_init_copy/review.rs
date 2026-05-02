// review overlay는 header/status/options/manual path를 작은 조립 단위로 나눈다.
// 이 facade는 planning init router가 세부 layout module을 직접 알지 않게 막는다.
#[path = "review/header.rs"]
mod header;
#[path = "review/manual_editor.rs"]
mod manual_editor;
#[path = "review/options.rs"]
mod options;
#[path = "review/simple_review.rs"]
mod simple_review;
#[path = "review/status.rs"]
mod status;

use super::super::super::PlanningInitOverlayView;
use super::super::copy::PlanningSimpleReviewCopy;

// simple review는 이미 presentation copy로 변환된 데이터만 소비한다. 여기서 state를
// 다시 조회하지 않아야 overlay rendering이 application state shape에 묶이지 않는다.
pub(super) fn build_simple_review_overlay_view(
    copy: PlanningSimpleReviewCopy,
) -> PlanningInitOverlayView {
    simple_review::build_simple_review_overlay_view(copy)
}

// manual editor path는 고정 안내 화면이라 copy가 없다. 그래도 같은 facade에 두어
// router가 review variant 선택만 하고 구체 layout 파일에는 의존하지 않게 한다.
pub(super) fn build_manual_editor_overlay_view() -> PlanningInitOverlayView {
    manual_editor::build_manual_editor_overlay_view()
}
