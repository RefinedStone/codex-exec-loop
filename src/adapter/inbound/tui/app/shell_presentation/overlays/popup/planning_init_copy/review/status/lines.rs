// 학습 주석: editing module은 turn budget 입력을 수정 중일 때만 나타나는 상태 줄을 만듭니다.
// status/lines는 mode별 line builder를 한곳에서 조립하는 index입니다.
#[path = "lines/editing.rs"]
mod editing;
// 학습 주석: first_error_tail은 validation error가 있을 때 status 끝에 붙는 조건부 줄을 만듭니다.
// prefix와 mode lines 뒤에 붙기 때문에 별도 module로 둡니다.
#[path = "lines/first_error_tail.rs"]
mod first_error_tail;
// 학습 주석: non_editing module은 일반 review 상태에서 가능한 promotion/close/detail actions를
// 설명하는 줄을 만듭니다. editing mode와 다른 controls를 보여 주기 위해 분리합니다.
#[path = "lines/non_editing.rs"]
mod non_editing;
// 학습 주석: prefix module은 validation state와 turn budget처럼 mode와 무관하게 항상 앞쪽에
// 표시할 상태 줄을 담당합니다.
#[path = "lines/prefix.rs"]
mod prefix;

// 학습 주석: 모든 status builder는 ratatui `Line` vector를 반환합니다. 이 layer는 문자열 상태를
// renderer가 바로 배치할 수 있는 presentation primitive로 정규화합니다.
use ratatui::text::Line;

// 학습 주석: copy는 simple review popup의 상태 snapshot입니다. validation, budget label, editing flag,
// first error를 읽어 status area를 조립합니다.
use crate::adapter::inbound::tui::app::shell_presentation::overlays::popup::planning::copy::PlanningSimpleReviewCopy;

// 학습 주석: `build_simple_review_status_lines`는 status area의 text lines를 순서대로 구성합니다.
// 항상 보이는 prefix를 먼저 만들고, editing 여부에 따라 control 안내를 바꾼 뒤, 첫 validation
// error가 있으면 tail로 덧붙입니다.
pub(super) fn build_simple_review_status_lines(
    // 학습 주석: `copy`는 화면에 표시할 현재 simple review 상태입니다. 이 함수는 읽기만 하므로
    // borrow로 받아 상위 view builder가 같은 copy로 key line도 만들 수 있게 합니다.
    copy: &PlanningSimpleReviewCopy,
) -> Vec<Line<'static>> {
    // 학습 주석: prefix는 validation state와 turn budget label이라 editing/non-editing 양쪽에서
    // 공통으로 보입니다. 이후 mode별 줄과 error tail을 같은 vector에 이어 붙입니다.
    let mut status_lines = prefix::build_simple_review_status_prefix_lines(
        copy.validation_ok,
        &copy.max_auto_turns_label,
    );
    // 학습 주석: turn budget을 편집 중이면 Enter/Esc 중심의 입력 안내가 필요하고, 평상시에는
    // promote/detail/close action 안내가 필요합니다.
    if copy.is_turn_budget_editing {
        status_lines.extend(editing::build_simple_review_editing_status_lines(
            copy.turn_budget_buffer.as_str(),
        ));
    } else {
        status_lines.extend(non_editing::build_simple_review_non_editing_status_lines());
    }
    status_lines.extend(first_error_tail::build_simple_review_first_error_tail_line(
        copy.first_error.as_deref(),
    ));
    // 학습 주석: status_lines는 prefix -> mode-specific lines -> optional first error 순서로 반환됩니다.
    // 이 순서가 popup 하단에서 사용자가 먼저 전체 상태를 보고, 다음 행동과 오류를 이어 읽게 합니다.
    status_lines
}
