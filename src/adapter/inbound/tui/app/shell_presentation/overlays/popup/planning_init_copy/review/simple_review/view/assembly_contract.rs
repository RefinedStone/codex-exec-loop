// 학습 주석: builder module은 이미 계산된 section bundle을 최종 assembly contract로 변환합니다.
// path attribute로 하위 디렉터리 파일을 이 view module의 내부 구현으로 연결합니다.
#[path = "assembly_contract/builder.rs"]
mod builder;
// 학습 주석: surface module은 renderer가 소비하는 contract type을 정의합니다. builder와 surface를
// 분리해 "어떻게 만든다"와 "무엇을 넘긴다"를 구분합니다.
#[path = "assembly_contract/surface.rs"]
mod surface;

// 학습 주석: sections composition은 header, option status, entry 같은 화면 조각을 이미 모아 둔
// 중간 결과입니다. 이 파일은 그 중간 결과를 renderer contract로 한 단계 더 감쌉니다.
use super::sections::composition::PlanningSimpleReviewOverlaySections;
// 학습 주석: 실제 field 이동과 contract 조립은 builder에 맡기고, 이 index 함수는 view 계층의
// 안정적인 entry point 이름을 제공합니다.
use builder::build_simple_review_assembly_contract_from_sections;
pub(super) use surface::PlanningSimpleReviewAssemblyContract;

// 학습 주석: `build_simple_review_assembly_contract`는 simple review popup view 조립의 마지막 공개
// helper입니다. caller는 section 구성 세부 파일을 몰라도 이 함수 하나로 renderer가 받을 contract를
// 얻습니다.
pub(super) fn build_simple_review_assembly_contract(
    // 학습 주석: `sections`는 앞 단계에서 모은 화면 조각 묶음입니다. ownership을 넘겨 contract가
    // section line vectors를 다시 복제하지 않고 그대로 소유하게 합니다.
    sections: PlanningSimpleReviewOverlaySections,
) -> PlanningSimpleReviewAssemblyContract {
    build_simple_review_assembly_contract_from_sections(sections)
}
