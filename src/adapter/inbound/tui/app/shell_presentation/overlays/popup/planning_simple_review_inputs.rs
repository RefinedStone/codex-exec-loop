use super::super::super::super::{FOOTER_NOTICE_DETAIL_LIMIT, NativeTuiApp, compact_inline_detail};
use super::copy::PlanningSimpleReviewCopy;

// simple review 화면은 두 UI state를 함께 보여 준다. planning init 쪽 staged draft
// metadata/validation 상태와, promote 이후 이어질 auto-follow turn budget editor 상태다.
// 이 projection이 두 source를 copy DTO로 고정해 아래 review builder가 app을 다시 읽지 않게 한다.
pub(super) fn build_simple_review_copy(app: &NativeTuiApp) -> PlanningSimpleReviewCopy {
    // simple_review가 없다는 것은 router step과 UI-local staged result가 잠깐 어긋난
    // degraded 상태다. rendering path에서는 panic보다 unknown/0 fallback을 택해
    // operator가 상태 불일치를 화면에서 확인할 수 있게 한다.
    let simple_review = app.planning_init_overlay_ui_state.simple_review();
    let validation_report = simple_review.map(|review| review.validation_report());

    PlanningSimpleReviewCopy {
        // draft name과 staged file count는 promotion 대상이 어떤 scaffold인지 식별하는
        // header/summary anchor다. review state가 없을 때 성공처럼 보이지 않도록
        // 이름은 unknown, 파일 수는 0으로 낮춘다.
        draft_name: simple_review
            .map(|review| review.draft_name().to_string())
            .unwrap_or_else(|| "unknown".to_string()),
        staged_file_count: simple_review
            .map(|review| review.staged_file_count())
            .unwrap_or_default(),
        // validation_ok는 promote key guidance와 status tone을 가르는 coarse gate다. report가
        // 아예 없으면 blocking error도 확인되지 않았다는 fallback으로 ready 쪽에 둔다.
        validation_ok: validation_report.is_none_or(|report| report.is_valid()),
        // status 영역은 좁으므로 전체 validation issue 목록 대신 첫 blocking error만 보낸다.
        // 긴 메시지는 footer detail limit과 같은 폭 정책으로 줄여 popup layout을 보호한다.
        first_error: validation_report
            .and_then(|report| report.errors().into_iter().next())
            .map(|issue| compact_inline_detail(issue.message.as_str(), FOOTER_NOTICE_DETAIL_LIMIT)),
        // auto-turn budget은 planning init 자체의 산출물은 아니지만, promote 직후 이어질
        // 자동 실행량을 같은 decision surface에서 조정하게 해 주는 adjacent control이다.
        max_auto_turns_label: app.current_max_auto_turns_label(),
        is_turn_budget_editing: app.is_max_auto_turns_editing(),
        // label은 committed budget이고 buffer는 editing mode의 raw draft input이다. 둘을
        // 함께 넘겨 status/key copy가 review controls와 text-input controls를 정확히 전환한다.
        turn_budget_buffer: app
            .auto_follow_overlay_ui_state
            .max_auto_turns_editor
            .buffer
            .clone(),
    }
}
