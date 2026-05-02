// sections DTO는 header/summary/options와 status view를 아직 section 단위로 나눠 들고 있다.
// 이 builder는 그 구조를 assembly contract의 평평한 field로 바꾼다.
use super::super::sections::composition::PlanningSimpleReviewOverlaySections;
// assembly contract는 최종 overlay view를 만들기 직전의 내부 전달 객체다.
use super::PlanningSimpleReviewAssemblyContract;

// 이 함수는 simple review section composition과 final overlay assembly 사이의 adapter다.
// section builder들이 어떤 내부 묶음을 반환하든, 아래 단계는 이 contract만 바라보게 된다.
pub(super) fn build_simple_review_assembly_contract_from_sections(
    // sections는 이미 각 화면 영역의 line을 모두 계산한 값이다. 여기서는 새 text를 만들지 않고 ownership을
    // contract로 이동해 조립 단계의 책임을 분리한다.
    sections: PlanningSimpleReviewOverlaySections,
) -> PlanningSimpleReviewAssemblyContract {
    PlanningSimpleReviewAssemblyContract {
        // header section은 최종 overlay의 header 영역과 1:1로 대응하므로 그대로 이동한다.
        header_lines: sections.header_lines,
        // summary section도 변환 없이 이동한다. 이 단계가 layout text를 다시 해석하지 않게 유지하는 것이
        // assembly contract의 역할이다.
        summary_lines: sections.summary_lines,
        // option_lines는 action 선택지 영역이다. sections에서 contract로 field 이름을 유지해 최종 DTO mapping이
        // 단순 복사로 끝나게 한다.
        option_lines: sections.option_lines,
        // status view는 내부적으로 status/key를 함께 들고 있다. contract에서는 최종 overlay field에 맞춰
        // status_lines만 먼저 꺼낸다.
        status_lines: sections.status_view.status_lines,
        // key_lines도 같은 status view에서 분리한다. 이렇게 하면 renderer는 key 영역을 status text와 독립적으로
        // 배치할 수 있다.
        key_lines: sections.status_view.key_lines,
    }
}
