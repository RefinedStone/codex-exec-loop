// 학습 주석: non-editing status도 renderer가 바로 배치할 수 있는 ratatui `Line`으로 반환합니다.
use ratatui::text::Line;

// 학습 주석: `build_simple_review_non_editing_status_lines`는 사용자가 turn budget 입력창에 있지 않을 때
// 가능한 주요 action들을 설명합니다. promote, close, detail authoring이 서로 다른 결과를 만들기 때문에
// status area에서 명시적으로 나눠 보여 줍니다.
pub(super) fn build_simple_review_non_editing_status_lines() -> Vec<Line<'static>> {
    vec![
        // 학습 주석: Enter/Ctrl+P는 staged simple scaffold를 실제 planning draft로 promote하는
        // 가장 중요한 happy path입니다.
        Line::from("next action: Enter or Ctrl+P promotes the staged simple scaffold."),
        // 학습 주석: Esc는 popup만 닫고 디스크의 staged draft는 유지합니다. 사용자가 검토를 중단해도
        // 생성된 초안이 사라지지 않는다는 점을 알려 줍니다.
        Line::from("alternate action: Esc closes this review and leaves the staged draft on disk."),
        // 학습 주석: D는 simple scaffold promotion 대신 detail-mode authoring으로 넘어가는 고급 경로입니다.
        Line::from(
            "advanced action: D opens detail-mode authoring without promoting the simple scaffold.",
        ),
    ]
}
