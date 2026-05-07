// assembly module은 이미 만들어진 assembly contract를 최종 overlay view로 넘기는 절반을 담당한다.
// pipeline index는 copy->contract와 contract->view를 한 namespace에 묶는다.
#[path = "contract_pipeline/assembly.rs"]
mod assembly;
// contract module은 copy를 assembly contract로 만드는 절반을 담당한다. 이 단계에서 copy의 presentation
// data가 section/contract 구조로 정리된다.
#[path = "contract_pipeline/contract.rs"]
mod contract;
// wiring module은 contract 생성과 final assembly를 실제 순서로 호출한다. pipeline index는 하위 단계들을
// 숨기고 copy 입력 하나를 받는 facade를 제공한다.
#[path = "contract_pipeline/wiring.rs"]
mod wiring;

// pipeline의 최종 산출물도 공통 overlay view다. 이 타입이 상위 popup renderer와 simple review 내부 조립
// pipeline을 연결한다.
use super::super::super::super::super::super::PlanningInitOverlayView;
// input copy는 아직 contract로 정규화되지 않은 simple review presentation source다.
// pipeline은 이 값을 받아 view 생성 전체를 실행한다.
use super::super::super::super::super::super::copy::PlanningSimpleReviewCopy;

// `build_simple_review_overlay_view_from_copy`는 pipeline 계층의 공개 함수다. caller는 copy만 넘기고,
// 이 함수 아래에서 contract 생성과 overlay assembly가 이어진다.
pub(super) fn build_simple_review_overlay_view_from_copy(
    // `copy`를 값으로 받아 pipeline 전체가 입력을 소유한다. contract 생성 단계에서는 borrow로 읽고,
    // 최종 view는 만들어진 contract를 소유한다.
    copy: PlanningSimpleReviewCopy,
) -> PlanningInitOverlayView {
    // wiring module에 실제 순서 결정을 맡겨, 이 파일은 pipeline namespace와 public entry 이름을 제공하는
    // 얇은 facade로 남는다.
    wiring::build_simple_review_overlay_view_from_copy(copy)
}
