// prefix lines도 popup renderer가 바로 그릴 수 있는 `Line`으로 만들어진다. 이 깊은 relative import는
// shell presentation layer의 Line type alias로 돌아간다.
use super::super::super::super::super::super::super::super::Line;

// `build_simple_review_status_prefix_lines`는 editing 여부와 무관하게 항상 보여 줄 상태 summary를 만든다.
// validation 결과와 turn budget은 사용자가 promotion 가능성을 판단하는 첫 정보다.
pub(super) fn build_simple_review_status_prefix_lines(
    // validation_ok는 staged simple scaffold를 바로 promote할 수 있는지 나타낸다.
    validation_ok: bool,
    // max_auto_turns_label은 이미 presentation-friendly 문자열로 만들어진 turn budget 표시다.
    max_auto_turns_label: &str,
) -> Vec<Line<'static>> {
    vec![
        // 첫 줄은 validation state를 사용자 언어로 압축한다. bool을 그대로 보이지 않고 ok/needs attention으로
        // 바꿔 action 가능성을 바로 읽게 한다.
        Line::from(format!(
            "validation state: {}",
            // validation이 실패했을 때는 뒤쪽 first error tail이 구체적인 이유를 보완한다.
            if validation_ok {
                "ok"
            } else {
                "needs attention"
            }
        )),
        // 두 번째 줄은 현재 자동 턴 예산을 보여 준다. editing mode에 들어가기 전후 모두 같은 위치에서
        // 예산 상태를 확인할 수 있게 하는 고정 prefix다.
        Line::from(format!("turn budget: {max_auto_turns_label}")),
    ]
}
