// `AkraTheme::key_line`은 key/action 안내 line에 일관된 styling을 적용한다. 이 파일은 raw `Line`보다
// theme helper를 써서 review overlay의 조작 안내가 다른 TUI 영역과 같은 톤을 갖게 한다.
use crate::adapter::inbound::tui::app::{AkraTheme, Line};

// key lines는 현재 입력 mode에 따라 달라지는 조작 안내다. turn budget 편집 중이면 숫자 입력/저장/취소
// 안내를 보여 주고, 평상시에는 promote/detail/edit/close 흐름을 보여 준다.
pub(super) fn build_simple_review_key_lines(is_turn_budget_editing: bool) -> Vec<Line<'static>> {
    // turn budget editing mode에서는 overlay의 primary action이 promote가 아니라 input 편집이다.
    // 그래서 일반 key map을 숨기고 편집 완료/취소/삭제 안내만 반환한다.
    if is_turn_budget_editing {
        // early return을 쓰면 편집 mode의 key map과 일반 mode의 key map이 섞이지 않는다.
        return vec![
            // 첫 줄은 지금 키 입력이 command가 아니라 turn budget 값 입력으로 처리됨을 알린다.
            AkraTheme::key_line("next action: type the new turn budget directly."),
            // 두 번째 줄은 편집 session을 저장하거나 취소하는 control contract를 표시한다.
            AkraTheme::key_line(
                "controls: Enter saves  |  Esc/Ctrl+C cancels  |  Backspace deletes",
            ),
            // 세 번째 줄은 validation rule을 UI에 드러내 잘못된 budget 입력을 줄인다.
            AkraTheme::key_line("validation: use a whole number greater than 0, or type infinite."),
        ];
    }

    vec![
        // 일반 mode의 primary action은 staged scaffold promote다.
        AkraTheme::key_line("Enter or Ctrl+P promotes the staged scaffold."),
        // 두 번째 줄은 대체 authoring path와 budget/draft inspection path를 함께 보여 준다.
        AkraTheme::key_line(
            "D opens detail-mode authoring. Ctrl+L edits turn budget. Ctrl+E inspects or edits the draft.",
        ),
        // 마지막 줄은 review를 수락하지 않고 닫는 탈출 동작을 제공한다.
        AkraTheme::key_line("Esc/Ctrl+C closes this review."),
    ]
}
