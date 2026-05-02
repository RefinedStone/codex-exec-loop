// 학습 주석: option_lines module은 simple review section 중 option 설명 줄을 수집합니다. 이 index는
// option half와 status half를 하나의 section bundle로 묶습니다.
#[path = "option_status/option_lines.rs"]
mod option_lines;
// 학습 주석: status_view module은 같은 copy에서 하단 status view를 수집합니다. option과 status는
// 서로 같은 UI 영역 가까이에 놓이므로 같은 section bundle로 이동합니다.
#[path = "option_status/status_view.rs"]
mod status_view;

pub(super) use super::{PlanningSimpleReviewCopy, PlanningSimpleReviewStatusView};
// 학습 주석: option half는 raw `Line` vector로 보관되고, status half는 별도 status view DTO로 보관됩니다.
use crate::adapter::inbound::tui::app::Line;
// 학습 주석: option line collector와 status view collector를 이 index로 가져와 한 함수에서 함께 호출합니다.
use option_lines::collect_simple_review_option_lines;
use status_view::collect_simple_review_status_view;

// 학습 주석: `PlanningSimpleReviewOptionStatusSections`는 simple review overlay의 중간 section bundle입니다.
// final assembly contract는 이 struct를 받아 option 설명과 status view를 각각 필요한 위치에 배치합니다.
pub(super) struct PlanningSimpleReviewOptionStatusSections {
    // 학습 주석: option_lines는 사용자가 현재 선택지를 이해하도록 돕는 설명 줄입니다.
    pub(super) option_lines: Vec<Line<'static>>,
    // 학습 주석: status_view는 validation/key/action 상태를 함께 담는 하단 상태 영역 DTO입니다.
    pub(super) status_view: PlanningSimpleReviewStatusView,
}

// 학습 주석: `collect_simple_review_option_status_sections`는 같은 copy snapshot에서 option lines와
// status view를 동시에 수집합니다. 이렇게 묶어 두면 이후 assembly 단계가 option/status 영역을
// 하나의 cohesive section으로 다룰 수 있습니다.
pub(super) fn collect_simple_review_option_status_sections(
    // 학습 주석: `copy`는 두 collector에 모두 borrow로 전달되어, option 설명과 status text가 같은
    // UI 상태를 기준으로 만들어집니다.
    copy: &PlanningSimpleReviewCopy,
) -> PlanningSimpleReviewOptionStatusSections {
    PlanningSimpleReviewOptionStatusSections {
        // 학습 주석: option half는 선택지 설명 line vector로 채웁니다.
        option_lines: collect_simple_review_option_lines(copy),
        // 학습 주석: status half는 같은 copy에서 만든 status DTO로 채웁니다.
        status_view: collect_simple_review_status_view(copy),
    }
}
