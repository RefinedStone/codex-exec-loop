// 학습 주석: contract 생성 단계의 입력은 simple review copy입니다. 아직 renderer view가 아니라
// 화면 문구와 상태를 담은 presentation source입니다.
use super::super::super::super::super::super::super::copy::PlanningSimpleReviewCopy;
// 학습 주석: 출력은 section 수집이 끝난 assembly contract입니다. 이 contract는 다음 handoff 단계에서
// final overlay view로 변환됩니다.
use super::super::super::assembly_contract::PlanningSimpleReviewAssemblyContract;
// 학습 주석: chaining helper가 실제 `copy -> sections -> contract` 순서를 수행합니다. 이 파일은
// contract handoff namespace에서 그 helper를 안정적인 이름으로 노출합니다.
use super::super::super::chaining::build_simple_review_assembly_contract_for_copy;

// 학습 주석: `build_simple_review_assembly_contract_from_copy`는 handoff pipeline의 첫 번째 단계입니다.
// copy를 읽어 renderer가 이해할 수 있는 assembly contract로 정규화합니다.
pub(super) fn build_simple_review_assembly_contract_from_copy(
    // 학습 주석: copy는 여기서 소유하지 않고 빌려 읽습니다. contract 생성은 필요한 line data를 새로
    // 만들기 때문에 caller가 copy ownership을 유지한 채 단계 순서를 제어할 수 있습니다.
    copy: &PlanningSimpleReviewCopy,
) -> PlanningSimpleReviewAssemblyContract {
    build_simple_review_assembly_contract_for_copy(copy)
}
