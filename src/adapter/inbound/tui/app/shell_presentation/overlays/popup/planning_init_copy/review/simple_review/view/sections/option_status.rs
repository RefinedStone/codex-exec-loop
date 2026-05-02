// option_lines module은 simple review section 중 option 설명 줄을 수집한다. 이 index는 option half와
// status half를 하나의 section bundle로 묶는다.
#[path = "option_status/option_lines.rs"]
mod option_lines;
// status_view module은 같은 copy에서 하단 status view를 수집한다. option과 status는 서로 같은 UI 영역
// 가까이에 놓이므로 같은 section bundle로 이동한다.
#[path = "option_status/status_view.rs"]
mod status_view;

pub(super) use super::{PlanningSimpleReviewCopy, PlanningSimpleReviewStatusView};
// option half는 raw `Line` vector로 보관되고, status half는 별도 status view DTO로 보관된다.
use crate::adapter::inbound::tui::app::Line;
// option line collector와 status view collector를 이 index로 가져와 한 함수에서 함께 호출한다.
use option_lines::collect_simple_review_option_lines;
use status_view::collect_simple_review_status_view;

// `PlanningSimpleReviewOptionStatusSections`는 simple review overlay의 중간 section bundle이다.
// final assembly contract는 이 struct를 받아 option 설명과 status view를 각각 필요한 위치에 배치한다.
pub(super) struct PlanningSimpleReviewOptionStatusSections {
    // option_lines는 사용자가 현재 선택지를 이해하도록 돕는 설명 줄이다.
    pub(super) option_lines: Vec<Line<'static>>,
    // status_view는 validation/key/action 상태를 함께 담는 하단 상태 영역 DTO다.
    pub(super) status_view: PlanningSimpleReviewStatusView,
}

// `collect_simple_review_option_status_sections`는 같은 copy snapshot에서 option lines와 status view를 동시에
// 수집한다. 이렇게 묶어 두면 이후 assembly 단계가 option/status 영역을 하나의 cohesive section으로 다룰 수
// 있다.
pub(super) fn collect_simple_review_option_status_sections(
    // `copy`는 두 collector에 모두 borrow로 전달되어, option 설명과 status text가 같은 UI 상태를 기준으로
    // 만들어진다.
    copy: &PlanningSimpleReviewCopy,
) -> PlanningSimpleReviewOptionStatusSections {
    PlanningSimpleReviewOptionStatusSections {
        // option half는 선택지 설명 line vector로 채운다.
        option_lines: collect_simple_review_option_lines(copy),
        // status half는 같은 copy에서 만든 status DTO로 채운다.
        status_view: collect_simple_review_status_view(copy),
    }
}
