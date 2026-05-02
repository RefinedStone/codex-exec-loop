// 학습 주석: options module은 simple review의 선택지/option 설명 line을 만드는 공통 presentation
// builder입니다. 이 helper는 그 결과를 sections assembly 단계로 가져옵니다.
use super::super::super::super::super::options;
// 학습 주석: copy에는 현재 선택지 label, validation 상태, budget 상태처럼 option line 생성에 필요한
// presentation 값이 들어 있습니다.
use super::PlanningSimpleReviewCopy;
// 학습 주석: option line도 최종 renderer가 그릴 `Line` vector입니다. section collector는 이 타입으로
// header/status/entry line들과 균일하게 합칩니다.
use crate::adapter::inbound::tui::app::Line;

// 학습 주석: `collect_simple_review_option_lines`는 option/status section의 option half를 수집합니다.
// 실제 문구 구성은 options module에 두고, 이 함수는 section 위치와 반환 contract를 명명합니다.
pub(super) fn collect_simple_review_option_lines(
    // 학습 주석: `copy`는 status collector와 같은 reference를 공유합니다. 같은 snapshot에서 option과
    // status를 만들어 UI 불일치를 줄입니다.
    copy: &PlanningSimpleReviewCopy,
) -> Vec<Line<'static>> {
    // 학습 주석: 공통 option builder에 그대로 위임해 section helper가 별도 표시 정책을 갖지 않게 합니다.
    options::build_simple_review_option_lines(copy)
}
