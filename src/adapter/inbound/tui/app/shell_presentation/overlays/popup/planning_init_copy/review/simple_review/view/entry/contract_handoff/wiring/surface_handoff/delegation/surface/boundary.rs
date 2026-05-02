// 학습 주석: boundary 단계도 최종적으로는 공통 overlay view를 반환합니다. boundary라는 이름은
// surface_handoff 내부에서 delegation implementation으로 넘어가는 경계임을 표현합니다.
use super::super::super::super::super::super::super::super::super::super::super::PlanningInitOverlayView;
// 학습 주석: copy는 아직 변환되지 않은 simple review presentation source입니다. 이 경계는 값을
// 읽지 않고 다음 delegation layer로 넘깁니다.
use super::super::super::super::super::super::super::super::super::super::super::copy::PlanningSimpleReviewCopy;
// 학습 주석: sibling delegation module이 실제 다음 호출을 수행합니다. boundary는 그 delegation으로
// 들어가기 전 public wrapper 이름을 제공합니다.
use super::delegation;

// 학습 주석: `build_simple_review_overlay_view_from_copy`는 boundary layer의 단일 entry입니다. 이름과
// signature를 유지해 상위 surface가 이 boundary 아래 구현 세부를 몰라도 됩니다.
pub(super) fn build_simple_review_overlay_view_from_copy(
    // 학습 주석: `copy` ownership을 그대로 delegation에 전달해 wrapper layer가 data를 복제하거나
    // 변경하지 않는다는 점을 보장합니다.
    copy: PlanningSimpleReviewCopy,
) -> PlanningInitOverlayView {
    // 학습 주석: 다음 단계인 delegation으로 넘기며, 실제 surface 연결은 그 아래에서 이어집니다.
    delegation::build_simple_review_overlay_view_from_copy(copy)
}
