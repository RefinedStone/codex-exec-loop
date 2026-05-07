// contract 생성 단계의 입력은 simple review copy다. 아직 renderer view가 아니라 화면 문구와 상태를 담은
// presentation source다.
use super::super::super::super::super::super::super::copy::PlanningSimpleReviewCopy;
// 출력은 section 수집이 끝난 assembly contract다. 이 contract는 다음 pipeline 단계에서 final overlay view로
// 변환된다.
use super::super::super::assembly_contract::PlanningSimpleReviewAssemblyContract;
// chaining helper가 실제 `copy -> sections -> contract` 순서를 수행한다. 이 파일은 contract pipeline
// namespace에서 그 helper를 안정적인 이름으로 노출한다.
use super::super::super::chaining::build_simple_review_assembly_contract_for_copy;

// `build_simple_review_assembly_contract_from_copy`는 contract pipeline의 첫 번째 단계다.
// copy를 읽어 renderer가 이해할 수 있는 assembly contract로 정규화한다.
pub(super) fn build_simple_review_assembly_contract_from_copy(
    // copy는 여기서 소유하지 않고 빌려 읽는다. contract 생성은 필요한 line data를 새로 만들기 때문에
    // caller가 copy ownership을 유지한 채 단계 순서를 제어할 수 있다.
    copy: &PlanningSimpleReviewCopy,
) -> PlanningSimpleReviewAssemblyContract {
    build_simple_review_assembly_contract_for_copy(copy)
}
