// summary copy도 popup 공통 header module에서 만들어진다. 이 파일은 summary section이 그 공통 copy를
// 사용한다는 assembly 연결점을 명확히 둔다.
use super::super::super::super::super::header;
// summary도 header와 같은 ratatui `Line` vector로 반환되어, section composition이 화면 영역별 line
// 묶음을 균일하게 다룰 수 있다.
use crate::adapter::inbound::tui::app::Line;

// `collect_simple_review_summary_lines`는 simple review popup의 목적/현재 검토 요약을 section bundle로
// 넘기는 helper다. header module의 문구 정책을 view assembly 단계로 끌어오는 얇은 경계 역할을 한다.
pub(super) fn collect_simple_review_summary_lines() -> Vec<Line<'static>> {
    // summary builder 결과를 그대로 반환해, 이 함수가 layout 위치 명명 외의 표시 정책을 추가하지 않는다는
    // 점을 분명히 한다.
    header::build_simple_review_summary_lines()
}
