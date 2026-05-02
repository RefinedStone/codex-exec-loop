// 학습 주석: status view builder는 copy를 읽고 두 종류의 line group을 만듭니다. `lines`는 상태 설명,
// `key_lines`는 shortcut guide라 서로 다른 하위 builder로 나누어집니다.
use super::{PlanningSimpleReviewCopy, PlanningSimpleReviewStatusView, key_lines, lines};

// 학습 주석: `build_simple_review_status_view`는 simple review popup 하단 status 영역의 최종 view DTO를
// 조립합니다. caller는 copy 하나만 넘기고, 이 함수가 상태 text와 key guide를 같은 snapshot 기준으로
// 계산합니다.
pub(super) fn build_simple_review_status_view(
    // 학습 주석: `copy`에는 validation 결과, editing 여부, budget label, first error가 모두 들어 있어
    // status_lines와 key_lines를 일관된 상태에서 만들 수 있습니다.
    copy: &PlanningSimpleReviewCopy,
) -> PlanningSimpleReviewStatusView {
    PlanningSimpleReviewStatusView {
        // 학습 주석: status_lines는 현재 상태와 다음 행동을 설명하는 본문 줄입니다.
        status_lines: lines::build_simple_review_status_lines(copy),
        // 학습 주석: key_lines는 현재 editing mode에 맞는 shortcut hints입니다. 같은 copy flag를 써서
        // status text와 shortcut guide가 서로 다른 mode를 말하지 않게 합니다.
        key_lines: key_lines::build_simple_review_key_lines(copy.is_turn_budget_editing),
    }
}
