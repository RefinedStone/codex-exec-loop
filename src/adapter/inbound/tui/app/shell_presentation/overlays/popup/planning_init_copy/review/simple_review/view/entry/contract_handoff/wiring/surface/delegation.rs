// 이 file은 surface delegation에서 실제 변환을 시작하는 지점이다. 최종 반환은 renderer가 받을 공통
// `PlanningInitOverlayView`다.
use super::super::super::super::super::super::super::super::super::PlanningInitOverlayView;
// 입력 copy는 simple review popup의 raw presentation source다. 아래에서 이 값을 assembly contract 생성
// 단계에 borrow로 넘긴다.
use super::super::super::super::super::super::super::super::super::copy::PlanningSimpleReviewCopy;
// contract module은 copy를 assembly contract로 바꾸고, assembly module은 그 contract를 final overlay view로
// 바꾼다. 이 두 module import가 pipeline의 실제 두 단계를 보여 준다.
use super::super::super::{assembly, contract};

// `build_simple_review_overlay_view_from_copy`는 wrapper 계층 끝에서 실제 조립을 수행한다.
// 여기서 처음으로 copy가 contract builder에 들어가고, 그 결과가 final assembly로 이어진다.
pub(super) fn build_simple_review_overlay_view_from_copy(
    // `copy`는 이 함수가 소유하지만 contract builder에는 `&copy`로 빌려 준다. contract가 만들어진 뒤 copy는
    // 더 쓰이지 않고 drop된다.
    copy: PlanningSimpleReviewCopy,
) -> PlanningInitOverlayView {
    /*
    이 nested call은 simple review view pipeline의 핵심 압축이다.
    1. contract::build_simple_review_assembly_contract_from_copy(&copy)가 copy를 section/contract로 정규화한다.
    2. assembly::build_simple_review_overlay_view_from_contract(...)가 contract를 공통 overlay view로 펼친다.
    */
    assembly::build_simple_review_overlay_view_from_contract(
        // contract 생성은 copy를 읽기만 하므로 borrow를 사용한다. final assembly에는 owned contract가 넘어간다.
        contract::build_simple_review_assembly_contract_from_copy(&copy),
    )
}
