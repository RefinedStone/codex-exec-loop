// composition module은 header/summary/options/status 조각을 최종 section DTO로 합치는 단계다.
#[path = "sections/composition.rs"]
pub(super) mod composition;
// header_summary module은 copy와 무관한 상단 고정 설명 영역을 수집한다.
#[path = "sections/header_summary.rs"]
mod header_summary;
// option_status module은 copy를 읽어 action options와 status view를 함께 수집한다.
#[path = "sections/option_status.rs"]
mod option_status;

// sections 하위 module들이 같은 copy type을 짧은 경로로 쓰도록 이 surface에서 재-export한다.
pub(super) use super::super::super::super::super::copy::PlanningSimpleReviewCopy;
// status view DTO도 section composition contract의 일부라 이 module surface에서 공유한다.
pub(super) use super::super::super::status::PlanningSimpleReviewStatusView;
// composition은 수집된 section 조각을 `PlanningSimpleReviewOverlaySections`로 묶는 마지막 단계다.
use composition::{PlanningSimpleReviewOverlaySections, compose_simple_review_overlay_sections};
// header/summary 수집은 copy에 의존하지 않는 고정 text path다.
use header_summary::collect_simple_review_header_summary_sections;
// option/status 수집은 copy에 담긴 review 대상 상태를 읽는 path다.
use option_status::collect_simple_review_option_status_sections;

// simple review overlay의 section 수집 entry다. 화면 조립 pipeline에서 copy DTO를 section groups로 바꾸고,
// 다음 assembly contract 단계가 이 결과만 소비하게 한다.
pub(super) fn collect_simple_review_overlay_sections(
    // copy는 option/status 쪽에서만 읽히지만 entry 함수가 copy를 받아 전체 section collection을 한 번에
    // 끝내도록 한다.
    copy: &PlanningSimpleReviewCopy,
) -> PlanningSimpleReviewOverlaySections {
    // header/summary는 copy 없이 고정 안내를 만들기 때문에 먼저 독립적으로 수집한다.
    let header_summary_sections = collect_simple_review_header_summary_sections();
    // option/status는 현재 review copy에 따라 달라지는 action과 상태 line을 수집한다.
    let option_status_sections = collect_simple_review_option_status_sections(copy);

    // 두 section 묶음을 composition DTO로 합쳐 assembly contract builder가 field 단위로 옮길 수 있는 안정적인
    // shape를 만든다.
    compose_simple_review_overlay_sections(header_summary_sections, option_status_sections)
}
