// 학습 주석: key_lines module은 status 영역 아래에 붙는 단축키/조작 안내 line을 만듭니다.
#[path = "status/key_lines.rs"]
mod key_lines;
// 학습 주석: lines module은 review status 자체를 설명하는 textual line들을 만듭니다.
#[path = "status/lines.rs"]
mod lines;
// 학습 주석: view module은 status line과 key line을 하나의 DTO로 묶는 조립 함수입니다.
#[path = "status/view.rs"]
mod view;

// 학습 주석: status view는 renderer로 내려갈 styled line 묶음을 보관합니다.
use ratatui::text::Line;

// 학습 주석: copy DTO는 status/key 문구를 만들 때 필요한 review 대상 정보와 label을 제공합니다.
use crate::adapter::inbound::tui::app::shell_presentation::overlays::popup::planning::copy::PlanningSimpleReviewCopy;

// 학습 주석: 이 DTO는 simple review 하단 영역을 두 갈래로 나눕니다. 상태 설명과 단축키 안내를 분리해
// assembly 단계가 최종 overlay의 `status_lines`/`key_lines` field로 바로 옮길 수 있습니다.
pub(super) struct PlanningSimpleReviewStatusView {
    // 학습 주석: status_lines는 현재 review 상태, 선택된 계획, 저장 여부 같은 맥락을 설명합니다.
    pub(super) status_lines: Vec<Line<'static>>,
    // 학습 주석: key_lines는 사용자가 다음에 누를 수 있는 key/action 안내를 담습니다.
    pub(super) key_lines: Vec<Line<'static>>,
}

// 학습 주석: status facade의 entry입니다. caller는 status line과 key line을 따로 만들지 않고 이 DTO 하나를
// 받아 section composition으로 넘깁니다.
pub(super) fn build_simple_review_status_view(
    // 학습 주석: copy를 shared reference로 받는 이유는 status 생성이 presentation line만 읽고 ownership을
    // 가져갈 필요가 없기 때문입니다.
    copy: &PlanningSimpleReviewCopy,
) -> PlanningSimpleReviewStatusView {
    // 학습 주석: 실제 status/key 조립은 view module에 위임해 이 파일을 module surface와 DTO 정의로 유지합니다.
    view::build_simple_review_status_view(copy)
}
