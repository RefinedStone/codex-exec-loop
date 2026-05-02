// 학습 주석: validation tail도 최종적으로 ratatui `Line`으로 전달됩니다. helper가 text-to-Line 변환을
// 맡으면 상위 view 조립 코드는 Option line을 붙일지만 결정하면 됩니다.
use ratatui::text::Line;

// 학습 주석: `build_simple_review_first_error_tail_line`은 validation error 목록 중 첫 오류만 status
// 영역 끝에 붙이는 presentation helper입니다. 모든 오류를 보여 주는 대신 첫 오류만 말해 사용자가
// 지금 고쳐야 할 입력을 빠르게 찾게 합니다.
pub(super) fn build_simple_review_first_error_tail_line(
    // 학습 주석: `first_error`가 None이면 validation 문제가 없다는 뜻이므로 tail line 자체를 만들지
    // 않습니다. Some이면 message를 그대로 화면용 문장에 끼워 넣습니다.
    first_error: Option<&str>,
) -> Option<Line<'static>> {
    /*
    학습 주석: Option::map을 쓰면 "오류가 있을 때만 Line을 만든다"는 조건을 분기문 없이 표현합니다.
    반환도 Option<Line>이라 caller는 다른 status line 뒤에 conditional tail을 자연스럽게 합칠 수
    있습니다.
    */
    first_error.map(|message| Line::from(format!("first validation error: {message}")))
}
