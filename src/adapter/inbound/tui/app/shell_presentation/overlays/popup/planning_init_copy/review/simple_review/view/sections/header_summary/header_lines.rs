// header text 자체는 popup 공통 header module에 있다. 이 section helper는 깊은 simple_review view
// 위치에서 그 공통 header builder를 가져와 section composition에 연결한다.
use super::super::super::super::super::header;
// `Line`은 TUI renderer가 그릴 수 있는 presentation 단위다. helper 반환 타입을 Line vector로
// 고정해 상위 section collector가 다른 section과 같은 형태로 합치게 한다.
use crate::adapter::inbound::tui::app::Line;

// `collect_simple_review_header_lines`는 simple review popup의 제목/상단 안내 줄을 section bundle에
// 넣기 위한 얇은 adapter다. 실제 문구 생성은 공통 header module에 남겨 header copy가 여러 view 조립
// 경로에서 중복되지 않게 한다.
pub(super) fn collect_simple_review_header_lines() -> Vec<Line<'static>> {
    // 공통 builder 결과를 그대로 반환한다. 이 함수의 의미는 변환이 아니라 header section이라는
    // view assembly 위치를 명명하는 데 있다.
    header::build_simple_review_header_lines()
}
