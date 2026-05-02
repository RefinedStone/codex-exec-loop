// status view builder는 copy를 읽고 두 종류의 line group을 만든다. `lines`는 상태 설명, `key_lines`는
// shortcut guide라 서로 다른 하위 builder로 나누어진다.
use super::{PlanningSimpleReviewCopy, PlanningSimpleReviewStatusView, key_lines, lines};

// `build_simple_review_status_view`는 simple review popup 하단 status 영역의 최종 view DTO를 조립한다.
// caller는 copy 하나만 넘기고, 이 함수가 상태 text와 key guide를 같은 snapshot 기준으로 계산한다.
pub(super) fn build_simple_review_status_view(
    // `copy`에는 validation 결과, editing 여부, budget label, first error가 모두 들어 있어 status_lines와
    // key_lines를 일관된 상태에서 만들 수 있다.
    copy: &PlanningSimpleReviewCopy,
) -> PlanningSimpleReviewStatusView {
    PlanningSimpleReviewStatusView {
        // status_lines는 현재 상태와 다음 행동을 설명하는 본문 줄이다.
        status_lines: lines::build_simple_review_status_lines(copy),
        // key_lines는 현재 editing mode에 맞는 shortcut hints다. 같은 copy flag를 써서 status text와
        // shortcut guide가 서로 다른 mode를 말하지 않게 한다.
        key_lines: key_lines::build_simple_review_key_lines(copy.is_turn_budget_editing),
    }
}
