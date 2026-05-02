// 학습 주석: final assembly 단계는 common `PlanningInitOverlayView`를 만들어 상위 popup renderer에
// 반환합니다. 이 타입 import가 simple review 전용 contract와 공통 overlay view를 연결합니다.
use crate::adapter::inbound::tui::app::shell_presentation::overlays::PlanningInitOverlayView;
// 학습 주석: surface module은 contract를 실제 overlay view 구조로 펼치는 rendering surface helper를
// 담습니다. assembly index는 그 helper를 안정적인 함수 이름으로 감쌉니다.
#[path = "assembly/surface.rs"]
mod surface;
// 학습 주석: assembly contract는 이전 단계가 수집한 section line들과 metadata를 담은 중간 결과입니다.
// final assembly는 이 contract만 보고 renderer용 view를 만듭니다.
use super::assembly_contract::PlanningSimpleReviewAssemblyContract;
// 학습 주석: surface helper가 실제 field-to-view mapping을 담당합니다. 이 파일은 조립 단계의
// entry point를 제공하고 세부 mapping을 하위 module에 둡니다.
use surface::build_simple_review_overlay_view_from_contract;

// 학습 주석: `assemble_simple_review_overlay_view`는 simple review 전용 contract를 공통 overlay view로
// 바꾸는 마지막 변환입니다. 이후 단계에서는 simple review 내부 section 구조를 더 이상 알 필요가 없습니다.
pub(super) fn assemble_simple_review_overlay_view(
    // 학습 주석: `contract` ownership을 넘겨 final view가 section vectors를 복제 없이 소유하게 합니다.
    contract: PlanningSimpleReviewAssemblyContract,
) -> PlanningInitOverlayView {
    build_simple_review_overlay_view_from_contract(contract)
}
