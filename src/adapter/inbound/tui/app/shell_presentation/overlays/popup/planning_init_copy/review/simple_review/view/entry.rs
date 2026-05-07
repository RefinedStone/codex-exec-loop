// entry 단계는 simple review view build 요청을 contract pipeline으로 넘긴다.
// path attribute는 `entry/contract_pipeline.rs`를 이 entry module의 내부 구현으로 연결한다.
#[path = "entry/contract_pipeline.rs"]
mod contract_pipeline;

// entry 함수의 반환 타입은 이미 공통 planning init overlay view다. 즉 entry 바깥에서는 simple review 전용
// 조립 단계가 보이지 않고, renderer가 이해하는 최종 view만 받는다.
use super::super::super::super::super::PlanningInitOverlayView;
// copy는 simple review popup의 text/option 상태 입력이다. entry는 이 값을 받아 contract pipeline에
// 넘기는 첫 함수다.
use super::super::super::super::super::copy::PlanningSimpleReviewCopy;

// `build_simple_review_overlay_view`는 view index가 호출하는 내부 entry point다.
// 함수 이름은 최종 산출물을 말하지만, 실제 단계 분리는 아래 contract_pipeline module에 맡긴다.
pub(super) fn build_simple_review_overlay_view(
    // `copy` ownership을 넘겨 contract pipeline이 section/contract/view 생성 과정에서 필요한 데이터를
    // 안전하게 소유하거나 빌릴 수 있게 한다.
    copy: PlanningSimpleReviewCopy,
) -> PlanningInitOverlayView {
    // copy에서 바로 view를 만들지 않고 pipeline을 거치게 해, copy-to-contract와 contract-to-view
    // 책임을 별도 파일에서 설명하고 테스트할 수 있는 구조를 유지한다.
    contract_pipeline::build_simple_review_overlay_view_from_copy(copy)
}
