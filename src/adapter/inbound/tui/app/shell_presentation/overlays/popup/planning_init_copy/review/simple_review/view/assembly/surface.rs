// 학습 주석: `PlanningInitOverlayView`는 popup renderer가 소비하는 최종 presentation DTO입니다. simple
// review 전용 조립 결과도 이 공통 planning init overlay shape로 반환됩니다.
use super::super::super::super::super::super::super::PlanningInitOverlayView;
// 학습 주석: assembly contract는 section composition 결과를 최종 DTO field와 거의 같은 구조로 보관한
// 내부 값입니다. 이 파일은 contract에서 renderer-facing DTO로 넘어가는 마지막 경계입니다.
use super::super::assembly_contract::PlanningSimpleReviewAssemblyContract;

// 학습 주석: 이 함수는 simple review 조립의 마지막 mapping입니다. 새 business rule을 적용하지 않고
// contract의 line 묶음을 `PlanningInitOverlayView` field에 복사해 presentation boundary를 닫습니다.
pub(super) fn build_simple_review_overlay_view_from_contract(
    // 학습 주석: contract는 이미 모든 line 계산을 끝낸 owned 값입니다. 여기서 ownership을 최종 view로
    // 넘기므로 caller가 중간 DTO를 다시 사용할 수 없습니다.
    contract: PlanningSimpleReviewAssemblyContract,
) -> PlanningInitOverlayView {
    PlanningInitOverlayView {
        // 학습 주석: header 영역은 contract와 최종 DTO가 같은 field 의미를 공유하므로 그대로 이동합니다.
        header_lines: contract.header_lines,
        // 학습 주석: summary 영역도 renderer가 기대하는 DTO field에 직접 연결됩니다.
        summary_lines: contract.summary_lines,
        // 학습 주석: option 영역은 사용자 action 안내를 담으므로 최종 overlay에서도 독립 field로 유지합니다.
        option_lines: contract.option_lines,
        // 학습 주석: status 영역은 review state 설명을 renderer-facing DTO에 전달합니다.
        status_lines: contract.status_lines,
        // 학습 주석: key 영역은 단축키 안내 line입니다. 마지막에 옮겨 최종 overlay가 모든 화면 영역을
        // 갖춘 상태로 반환됩니다.
        key_lines: contract.key_lines,
    }
}
