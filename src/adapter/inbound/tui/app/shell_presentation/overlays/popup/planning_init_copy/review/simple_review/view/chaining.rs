// chaining 단계의 입력은 아직 line으로 분해되지 않은 simple review copy다. copy는 header, option status,
// handoff text를 만들 원천 presentation data다.
use super::super::super::super::super::copy::PlanningSimpleReviewCopy;
// assembly_contract helpers는 section bundle을 renderer contract로 묶는다. chaining은 section 수집 결과를
// 이 contract builder로 넘기는 흐름을 담당한다.
use super::assembly_contract::{
    PlanningSimpleReviewAssemblyContract, build_simple_review_assembly_contract,
};
// sections collector는 copy를 화면 구역별 line 묶음으로 분해한다. chaining은 그 결과를 contract로
// 승격시키는 다음 단계를 연결한다.
use super::sections::collect_simple_review_overlay_sections;

// `build_simple_review_assembly_contract_for_copy`는 copy에서 바로 final view로 가지 않고
// `copy -> sections -> contract` 순서를 명명한다. 이 작은 함수 덕분에 entry 단계는 조립 순서를 읽기 쉬운
// 한 호출로 표현할 수 있다.
pub(super) fn build_simple_review_assembly_contract_for_copy(
    // `copy`는 section collector가 읽기만 하므로 borrow로 받는다. final contract는 collector가 만든 owned
    // line vectors를 갖게 된다.
    copy: &PlanningSimpleReviewCopy,
) -> PlanningSimpleReviewAssemblyContract {
    build_simple_review_assembly_contract(collect_simple_review_overlay_sections(copy))
}
