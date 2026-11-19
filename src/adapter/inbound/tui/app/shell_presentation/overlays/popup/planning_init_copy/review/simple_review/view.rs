// assembly module은 renderer가 받을 최종 `PlanningInitOverlayView`를 만드는 단계다.
// view index는 이 하위 파일들을 순서 있는 조립 파이프라인으로 묶는다.
#[path = "view/assembly.rs"]
mod assembly;
// assembly_contract는 section 결과를 renderer에 넘길 중간 contract로 정리한다.
// sections와 final assembly 사이의 데이터 모양을 고정하는 경계다.
#[path = "view/assembly_contract.rs"]
mod assembly_contract;
// chaining은 copy에서 section collection, contract creation을 이어 주는 glue layer다.
// 각 단계의 helper를 직접 호출하지 않고 조립 순서를 이름 있는 함수로 남긴다.
#[path = "view/chaining.rs"]
mod chaining;
// sections는 header, option status, entry/handoff 등 화면 구역별 line 묶음을 수집한다.
// copy에서 바로 final view를 만들지 않고 section 단위로 나누는 이유가 여기에 있다.
#[path = "view/sections.rs"]
mod sections;

// 반환 타입은 planning init popup renderer가 공통으로 받는 view다. simple review 내부 조립이 끝나면 이
// 타입으로 상위 overlay 흐름에 합류한다.
use super::super::super::super::super::PlanningInitOverlayView;
// copy는 text와 옵션 표시 값을 담은 input surface다. view layer는 이 값을 section line들로 바꾸는
// presentation mapping을 수행한다.
use super::super::super::super::copy::PlanningSimpleReviewCopy;
// assembly는 contract-to-view, chaining은 copy-to-contract 변환이다. facade는 이 두 단계를 직접 잇는다.
use assembly::assemble_simple_review_overlay_view;
use chaining::build_simple_review_assembly_contract_for_copy;

// 이 함수는 simple review view 내부의 public facade다. 호출자는 `copy`만 넘기고, module 내부가 section
// 수집, contract 조립, final view assembly 순서를 책임진다.
pub(super) fn build_simple_review_overlay_view(
    // `copy`는 값으로 받아 조립 파이프라인이 필요한 데이터를 소유하게 한다. facade에서 필요한 곳에
    // borrow하거나 move할 수 있다.
    copy: PlanningSimpleReviewCopy,
) -> PlanningInitOverlayView {
    // contract 생성은 copy를 읽기만 하므로 borrow를 사용한다. final assembly에는 owned contract가 넘어간다.
    assemble_simple_review_overlay_view(build_simple_review_assembly_contract_for_copy(&copy))
}
