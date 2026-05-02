// 학습 주석: status view는 status line과 key line을 함께 담는 하단 영역 DTO입니다. overlay section DTO는
// 이 값을 통째로 보존한 뒤 다음 assembly contract 단계에서 status/key field로 분리합니다.
use super::PlanningSimpleReviewStatusView;
// 학습 주석: header/summary sections는 copy와 무관하게 수집된 상단 영역 묶음입니다.
use super::header_summary::PlanningSimpleReviewHeaderSummarySections;
// 학습 주석: option/status sections는 review copy를 읽어 action option과 status view를 만든 하단 영역 묶음입니다.
use super::option_status::PlanningSimpleReviewOptionStatusSections;
// 학습 주석: section DTO는 renderer로 갈 styled text line을 영역별 vector로 들고 있습니다.
use crate::adapter::inbound::tui::app::Line;

// 학습 주석: 이 DTO는 simple review overlay를 구성하는 모든 section을 한 번에 운반하는 내부 contract입니다.
// 위 단계에서는 header/summary와 option/status가 별도 collector에서 오고, 아래 단계에서는 assembly contract가
// 이 field들을 최종 `PlanningInitOverlayView` shape로 평탄화합니다.
pub(in super::super) struct PlanningSimpleReviewOverlaySections {
    // 학습 주석: header_lines는 review 화면의 제목과 목적을 담는 상단 영역입니다.
    pub(in super::super) header_lines: Vec<Line<'static>>,
    // 학습 주석: summary_lines는 promote 결과와 안전 경계를 설명하는 본문 요약 영역입니다.
    pub(in super::super) summary_lines: Vec<Line<'static>>,
    // 학습 주석: option_lines는 staged draft, artifact count, promote/detail 선택지 같은 action context를 담습니다.
    pub(in super::super) option_lines: Vec<Line<'static>>,
    // 학습 주석: status_view는 status_lines와 key_lines를 아직 묶은 상태로 보관합니다. 이 구조는 하단 영역
    // 생성 책임을 status module에 남기면서 assembly 단계가 필요한 시점에만 분리하게 합니다.
    pub(in super::super) status_view: PlanningSimpleReviewStatusView,
}

// 학습 주석: 이 함수는 두 collector의 결과를 simple review 전체 section contract로 합칩니다. 새 line을
// 생성하지 않고 ownership만 옮기므로, text 생성 책임과 composition 책임이 분리됩니다.
pub(super) fn compose_simple_review_overlay_sections(
    // 학습 주석: 상단 collector 결과입니다. header와 summary는 copy 없이 static line 중심으로 만들어졌습니다.
    header_summary_sections: PlanningSimpleReviewHeaderSummarySections,
    // 학습 주석: 하단 collector 결과입니다. option/status는 review copy와 edit/budget state를 반영합니다.
    option_status_sections: PlanningSimpleReviewOptionStatusSections,
) -> PlanningSimpleReviewOverlaySections {
    PlanningSimpleReviewOverlaySections {
        // 학습 주석: header field는 상단 DTO에서 그대로 이동해 최종 overlay header로 이어집니다.
        header_lines: header_summary_sections.header_lines,
        // 학습 주석: summary field도 상단 DTO에서 그대로 이동합니다.
        summary_lines: header_summary_sections.summary_lines,
        // 학습 주석: option field는 하단 DTO에서 이동해 action context 영역으로 유지됩니다.
        option_lines: option_status_sections.option_lines,
        // 학습 주석: status_view는 status/key split 전 상태 그대로 넘겨 status module의 contract를 보존합니다.
        status_view: option_status_sections.status_view,
    }
}
