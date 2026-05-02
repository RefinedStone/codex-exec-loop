// header_lines module은 simple review 상단 제목/목적 line을 독립적으로 수집한다.
#[path = "header_summary/header_lines.rs"]
mod header_lines;
// summary_lines module은 promote 결과 설명 line을 독립적으로 수집한다.
#[path = "header_summary/summary_lines.rs"]
mod summary_lines;

// section DTO는 최종 overlay로 전달될 styled line 묶음을 field별로 보존한다.
use crate::adapter::inbound::tui::app::Line;
// header collector는 copy에 의존하지 않는 static header line을 반환한다.
use header_lines::collect_simple_review_header_lines;
// summary collector도 copy 없이 simple review promote 의미를 설명하는 static line을 반환한다.
use summary_lines::collect_simple_review_summary_lines;

// 이 DTO는 상단 section 두 종류를 함께 이동시키는 내부 contract다. option/status section과 합쳐지기 전
// 단계에서 header와 summary를 명확히 구분해 assembly mapping을 단순하게 만든다.
pub(super) struct PlanningSimpleReviewHeaderSummarySections {
    // header_lines는 overlay의 목적과 현재 review 단계의 이름을 담는다.
    pub(super) header_lines: Vec<Line<'static>>,
    // summary_lines는 promote 이후의 planning baseline 상태를 설명한다.
    pub(super) summary_lines: Vec<Line<'static>>,
}

// 이 함수는 copy 없이 만들 수 있는 simple review 상단 section을 한 번에 수집한다. 하위 collector를 분리해
// 둔 덕분에 header와 summary 문구를 독립적으로 테스트/교체할 수 있다.
pub(super) fn collect_simple_review_header_summary_sections()
-> PlanningSimpleReviewHeaderSummarySections {
    PlanningSimpleReviewHeaderSummarySections {
        // header line collection은 최종 section DTO의 header field로 그대로 이동한다.
        header_lines: collect_simple_review_header_lines(),
        // summary line collection도 별도 변환 없이 summary field로 이동한다.
        summary_lines: collect_simple_review_summary_lines(),
    }
}
