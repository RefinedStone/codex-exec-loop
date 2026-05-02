// 학습 주석: 이 file은 surface delegation에서 실제 변환을 시작하는 지점입니다. 최종 반환은
// renderer가 받을 공통 `PlanningInitOverlayView`입니다.
use super::super::super::super::super::super::super::super::super::PlanningInitOverlayView;
// 학습 주석: 입력 copy는 simple review popup의 raw presentation source입니다. 아래에서 이 값을
// assembly contract 생성 단계에 borrow로 넘깁니다.
use super::super::super::super::super::super::super::super::super::copy::PlanningSimpleReviewCopy;
// 학습 주석: contract module은 copy를 assembly contract로 바꾸고, assembly module은 그 contract를
// final overlay view로 바꿉니다. 이 두 module import가 pipeline의 실제 두 단계를 보여 줍니다.
use super::super::super::{assembly, contract};

// 학습 주석: `build_simple_review_overlay_view_from_copy`는 wrapper 계층 끝에서 실제 조립을 수행합니다.
// 여기서 처음으로 copy가 contract builder에 들어가고, 그 결과가 final assembly로 이어집니다.
pub(super) fn build_simple_review_overlay_view_from_copy(
    // 학습 주석: `copy`는 이 함수가 소유하지만 contract builder에는 `&copy`로 빌려 줍니다. contract가
    // 만들어진 뒤 copy는 더 쓰이지 않고 drop됩니다.
    copy: PlanningSimpleReviewCopy,
) -> PlanningInitOverlayView {
    /*
    학습 주석: 이 nested call은 simple review view pipeline의 핵심 압축입니다.
    1. contract::build_simple_review_assembly_contract_from_copy(&copy)가 copy를 section/contract로 정규화합니다.
    2. assembly::build_simple_review_overlay_view_from_contract(...)가 contract를 공통 overlay view로 펼칩니다.
    */
    assembly::build_simple_review_overlay_view_from_contract(
        // 학습 주석: contract 생성은 copy를 읽기만 하므로 borrow를 사용합니다. final assembly에는
        // owned contract가 넘어갑니다.
        contract::build_simple_review_assembly_contract_from_copy(&copy),
    )
}
