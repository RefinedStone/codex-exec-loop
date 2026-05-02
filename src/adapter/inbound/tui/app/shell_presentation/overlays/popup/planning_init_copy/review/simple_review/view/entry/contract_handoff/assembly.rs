// 학습 주석: 이 module은 contract-to-view 변환 절반만 담당하므로 최종 반환 타입인
// `PlanningInitOverlayView`를 명시적으로 가져옵니다.
use super::super::super::super::super::super::super::PlanningInitOverlayView;
// 학습 주석: 실제 final assembly는 view/assembly module의 책임입니다. handoff assembly는 entry
// 계층에서 그 final assembler로 연결되는 adapter 역할을 합니다.
use super::super::super::assembly::assemble_simple_review_overlay_view;
// 학습 주석: assembly contract는 copy에서 이미 section과 metadata가 정리된 중간 산출물입니다.
// 이 함수는 그 contract를 final renderer view로 넘기는 데 집중합니다.
use super::super::super::assembly_contract::PlanningSimpleReviewAssemblyContract;

// 학습 주석: `build_simple_review_overlay_view_from_contract`는 handoff pipeline의 두 번째 단계입니다.
// contract가 만들어진 뒤 이 함수를 통과하면 simple review 전용 구조가 공통 overlay view가 됩니다.
pub(super) fn build_simple_review_overlay_view_from_contract(
    // 학습 주석: `contract`는 더 이상 copy를 참조하지 않는 owned 조립 계약입니다. ownership을 넘겨
    // final view가 내부 line vectors를 그대로 가져가게 합니다.
    contract: PlanningSimpleReviewAssemblyContract,
) -> PlanningInitOverlayView {
    assemble_simple_review_overlay_view(contract)
}
