// 학습 주석: header module은 simple review overlay 상단에 들어가는 제목/context line builder를 담습니다.
#[path = "review/header.rs"]
mod header;
// 학습 주석: manual_editor module은 자동 simple review 대신 수동 편집 안내 overlay를 만드는 경로입니다.
#[path = "review/manual_editor.rs"]
mod manual_editor;
// 학습 주석: options module은 simple review에서 사용자가 선택할 수 있는 accept/edit/cancel류 action line을
// 구성합니다.
#[path = "review/options.rs"]
mod options;
// 학습 주석: simple_review module은 copy DTO를 받아 최종 planning init overlay DTO로 조립하는 main path입니다.
#[path = "review/simple_review.rs"]
mod simple_review;
// 학습 주석: status module은 review 대상의 현재 상태와 단축키 안내를 묶은 하단 영역을 담당합니다.
#[path = "review/status.rs"]
mod status;

// 학습 주석: 모든 review builder는 popup renderer가 소비하는 공통 planning init overlay DTO를 반환합니다.
use super::super::super::PlanningInitOverlayView;
// 학습 주석: copy DTO는 application state에서 뽑은 presentation용 데이터입니다. 이 facade는 copy를 하위
// simple review 조립 경로로 넘깁니다.
use super::super::copy::PlanningSimpleReviewCopy;

// 학습 주석: simple review overlay의 public entry입니다. caller는 review 하위 module 구조를 몰라도
// 이 함수만 호출해 final `PlanningInitOverlayView`를 얻습니다.
pub(super) fn build_simple_review_overlay_view(
    // 학습 주석: copy는 이미 화면 문구로 변환 가능한 값만 담고 있습니다. build 단계는 application state를
    // 다시 조회하지 않고 이 DTO만 소비합니다.
    copy: PlanningSimpleReviewCopy,
) -> PlanningInitOverlayView {
    // 학습 주석: 실제 section composition과 assembly는 simple_review module에 위임합니다. 이 파일은 review
    // variant들을 한 surface로 묶는 facade 역할에 집중합니다.
    simple_review::build_simple_review_overlay_view(copy)
}

// 학습 주석: manual editor overlay는 copy가 필요 없는 고정 안내 화면입니다. 같은 review facade에서
// 제공해 planning init router가 manual/simple variant를 같은 namespace에서 선택하게 합니다.
pub(super) fn build_manual_editor_overlay_view() -> PlanningInitOverlayView {
    // 학습 주석: 구체 line 구성은 manual_editor module에 남겨 이 facade가 layout detail을 갖지 않게 합니다.
    manual_editor::build_manual_editor_overlay_view()
}
