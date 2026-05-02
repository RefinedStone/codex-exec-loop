// `Line`은 TUI가 실제로 그릴 한 줄의 styled text다. assembly contract는 각 section이 만든 line 묶음을
// 최종 overlay view로 넘기기 전에 같은 shape로 보관한다.
use crate::adapter::inbound::tui::app::Line;

// 이 contract는 simple review 화면 조립 단계의 내부 DTO다. section builder들이 만든
// header/summary/options/status/key line을 한 번에 들고 다니게 해 최종 view 변환 함수가 sections 내부
// 구조를 직접 알 필요가 없게 한다.
pub(in super::super) struct PlanningSimpleReviewAssemblyContract {
    // header_lines는 overlay 상단 제목과 context를 담는다. contract에 보존해 최종
    // `PlanningInitOverlayView.header_lines`로 그대로 전달한다.
    pub(in super::super) header_lines: Vec<Line<'static>>,
    // summary_lines는 draft/task 요약처럼 사용자가 먼저 훑어야 하는 본문 정보다.
    pub(in super::super) summary_lines: Vec<Line<'static>>,
    // option_lines는 사용자가 선택할 수 있는 review action들을 담는다. 다른 section과 섞지 않고 따로 두어
    // renderer가 option 영역을 안정적으로 배치한다.
    pub(in super::super) option_lines: Vec<Line<'static>>,
    // status_lines는 현재 review state를 설명하는 line이다. status view에서 꺼내 contract의 평평한 field로
    // 옮기면 최종 overlay DTO와 field shape가 맞아진다.
    pub(in super::super) status_lines: Vec<Line<'static>>,
    // key_lines는 단축키 안내 영역이다. status_lines와 같은 status view에서 오지만 최종 overlay에서는
    // 별도 영역으로 렌더링되므로 독립 field로 유지한다.
    pub(in super::super) key_lines: Vec<Line<'static>>,
}
