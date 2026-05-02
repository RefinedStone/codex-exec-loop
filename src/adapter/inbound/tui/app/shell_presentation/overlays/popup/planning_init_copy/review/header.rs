// `Line`은 ratatui가 그릴 styled text 한 줄이다. review header/summary builder는 최종
// `PlanningInitOverlayView`의 상단 영역에 들어갈 line vector를 만든다.
use super::super::super::super::super::super::Line;
// planning setup title helper는 planning init overlay 전체가 공유하는 제목 스타일을 적용한다.
// 이 파일은 suffix만 붙여 simple review가 operator inspection 단계임을 드러낸다.
use super::super::super::copy::planning_setup_title_line;

// header lines는 simple review 화면의 목적을 가장 먼저 설명한다. 사용자가 지금 보는 화면이 계획 작성이
// 아니라 "가벼운 baseline을 promote할지 검토하는 gate"임을 고정 문구로 전달한다.
pub(super) fn build_simple_review_header_lines() -> Vec<Line<'static>> {
    vec![
        // 공통 planning setup title에 operator inspection suffix를 붙여, router가 같은 planning init overlay 안에서도
        // review 단계임을 한눈에 구분하게 한다.
        planning_setup_title_line(" / operator inspection"),
        // 이 문장은 simple mode의 제품 의도를 설명한다. 즉시 상세 authoring으로 들어가기보다 최소 baseline을
        // 먼저 승격할 수 있다는 선택지를 사용자에게 노출한다.
        Line::from(
            "Simple mode review: promote the lightest planning baseline before you invest in richer authoring.",
        ),
    ]
}

// summary lines는 promote 이후 시스템 상태가 어떻게 바뀌는지 설명한다. option/status 영역이 action과 현재
// 상태를 다룬다면, 이 영역은 promote 결과의 의미를 사전에 풀어 준다.
pub(super) fn build_simple_review_summary_lines() -> Vec<Line<'static>> {
    vec![
        // 첫 줄은 promote가 generic direction과 빈 task ledger로 시작한다는 구조적 결과를 알려 준다.
        Line::from(
            "After promote, planning starts with one generic direction and no active queue task yet.",
        ),
        // 두 번째 줄은 queue가 비어 있을 때도 후속 작업 근거를 남길 default prompt가 준비되어 있음을 설명한다.
        // 이는 simple scaffold가 완전히 비어 있지 않다는 신호다.
        Line::from(
            "The default queue-idle review prompt is already staged so the first reply can justify follow-up work when needed.",
        ),
        // 마지막 줄은 이 화면이 아직 commit/accept 전이라는 안전 경계를 명확히 한다.
        Line::from("No accepted planning state changes until you explicitly promote this review."),
    ]
}
