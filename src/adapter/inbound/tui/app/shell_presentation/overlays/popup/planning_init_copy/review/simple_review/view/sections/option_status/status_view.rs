// status module은 simple review popup 하단 status view를 실제로 만드는 공통 builder다.
// 이 section helper는 그 builder를 option/status section assembly 위치로 끌어온다.
use super::super::super::super::super::status;
// copy는 status 계산의 input이고, PlanningSimpleReviewStatusView는 section bundle에 들어갈 output이다.
// 이 둘을 같은 section namespace에서 다루도록 가져온다.
use super::{PlanningSimpleReviewCopy, PlanningSimpleReviewStatusView};

// `collect_simple_review_status_view`는 section collector가 status 하위 builder의 파일 구조를 몰라도 status
// view를 얻도록 해 주는 adapter다. 실제 text/key line 조립은 status module에 남긴다.
pub(super) fn collect_simple_review_status_view(
    // `copy`를 borrow로 받아 option lines collector와 같은 snapshot을 공유한다.
    copy: &PlanningSimpleReviewCopy,
) -> PlanningSimpleReviewStatusView {
    // status builder에 그대로 위임해 이 helper가 section 위치 명명 외의 표시 정책을 추가하지 않게 한다.
    status::build_simple_review_status_view(copy)
}
