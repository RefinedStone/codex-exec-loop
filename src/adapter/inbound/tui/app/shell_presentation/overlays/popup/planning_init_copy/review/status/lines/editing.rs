// 학습 주석: status helper는 ratatui가 그릴 수 있는 `Line`을 반환합니다. 여기서 문자열을
// presentation primitive로 바꾸어 상위 overlay renderer가 layout만 책임지게 합니다.
use ratatui::text::Line;

// 학습 주석: `build_simple_review_editing_status_lines`는 simple review popup이 turn budget 입력을
// 편집 중일 때 하단 상태 영역에 보여 줄 안내 문구를 만듭니다. 상태 판단은 caller가 끝내고,
// 이 함수는 "editing mode를 어떻게 말로 보여 줄지"만 담당합니다.
pub(super) fn build_simple_review_editing_status_lines(
    // 학습 주석: `turn_budget_buffer`는 아직 저장되지 않은 입력창 버퍼입니다. 사용자가 Enter를 누르기
    // 전까지 domain 값이 아니라 UI draft 값이므로, 문구도 current value가 아닌 buffer 상태를 보여 줍니다.
    turn_budget_buffer: &str,
) -> Vec<Line<'static>> {
    /*
    학습 주석: 반환 타입을 Vec으로 유지하는 이유는 status 영역의 다른 mode들이 여러 줄을 만들 수
    있기 때문입니다. editing mode는 현재 한 줄만 필요하지만 caller는 mode와 상관없이 같은
    "status lines" contract를 받아 overlay에 붙일 수 있습니다.
    */
    vec![Line::from(format!(
        "current state: editing turn budget / value: {} / controls: Enter saves, Esc/Ctrl+C cancels",
        turn_budget_buffer
    ))]
}
