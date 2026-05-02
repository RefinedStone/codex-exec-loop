// 학습 주석: simple review copy는 app 전체 state에서 planning init review 상태와 followup turn-budget
// editor 상태를 함께 읽습니다. compact_inline_detail은 긴 validation error를 footer 폭에 맞게 줄입니다.
use super::super::super::super::{FOOTER_NOTICE_DETAIL_LIMIT, NativeTuiApp, compact_inline_detail};
// 학습 주석: PlanningSimpleReviewCopy는 이후 review/view pipeline으로 넘어가는 raw presentation input입니다.
// 이 파일은 Line을 만들지 않고, 화면 문구를 만들 재료만 모읍니다.
use super::copy::PlanningSimpleReviewCopy;

// 학습 주석: build_simple_review_copy는 NativeTuiApp에서 simple init review 화면에 필요한 상태 snapshot을 추출합니다.
// planning_init_router가 이 copy를 만든 뒤 simple_review view builder에 넘기고, 하위 builder들이 header/options/status로 분해합니다.
pub(super) fn build_simple_review_copy(app: &NativeTuiApp) -> PlanningSimpleReviewCopy {
    // 학습 주석: simple_review는 staged simple planning scaffold를 promote하기 전의 review state입니다.
    // None이면 UI step과 state가 잠깐 어긋난 fallback 상황이므로 아래에서 안전한 default copy를 만듭니다.
    let simple_review = app.planning_init_overlay_ui_state.simple_review();
    // 학습 주석: validation_report는 draft files가 planning schema/semantic validation을 통과했는지 알려 줍니다.
    // Option으로 둬 simple_review가 없을 때도 validation_ok fallback을 계산할 수 있습니다.
    let validation_report = simple_review.map(|review| review.validation_report());

    // 학습 주석: 반환 copy는 staged draft metadata, validation state, turn budget editor state를 한 구조로 묶습니다.
    // 이후 status/key line builders는 app을 다시 읽지 않고 이 snapshot만 사용합니다.
    PlanningSimpleReviewCopy {
        // 학습 주석: draft_name은 promotion 대상 draft를 사용자에게 식별시킵니다. review state가 없으면 unknown으로 표시해
        // panic 대신 degraded presentation을 유지합니다.
        draft_name: simple_review
            // 학습 주석: review object에서 borrowed name을 가져와 copy DTO가 소유할 String으로 바꿉니다.
            .map(|review| review.draft_name().to_string())
            // 학습 주석: missing review fallback은 화면이 깨지는 대신 상태 불일치를 눈에 보이게 합니다.
            .unwrap_or_else(|| "unknown".to_string()),
        // 학습 주석: staged_file_count는 simple scaffold가 몇 개의 planning artifact를 만들었는지 summary/options에 쓰입니다.
        staged_file_count: simple_review
            // 학습 주석: review state가 있으면 service result에서 저장해 둔 staged file count를 그대로 사용합니다.
            .map(|review| review.staged_file_count())
            // 학습 주석: review state가 없으면 0개로 두어 summary가 과장된 성공 상태를 보여 주지 않게 합니다.
            .unwrap_or_default(),
        // 학습 주석: validation_ok는 status prefix와 accept/promotion 판단 안내에 쓰이는 핵심 boolean입니다.
        // report가 없으면 오류가 확인되지 않았다는 fallback으로 true를 사용합니다.
        validation_ok: validation_report.is_none_or(|report| report.is_valid()),
        // 학습 주석: first_error는 validation errors 중 첫 항목만 compact text로 보냅니다.
        // status area는 좁기 때문에 전체 오류 목록 대신 첫 blocking issue를 tail line에 노출합니다.
        first_error: validation_report
            // 학습 주석: validation report에서 error iterator를 만들고 첫 번째 error만 선택합니다.
            .and_then(|report| report.errors().into_iter().next())
            // 학습 주석: 긴 error message는 footer detail limit에 맞게 줄여 하단 status layout을 보호합니다.
            .map(|issue| compact_inline_detail(issue.message.as_str(), FOOTER_NOTICE_DETAIL_LIMIT)),
        // 학습 주석: simple review에서도 followup auto-turn budget을 보여 줍니다. promote 이후 이어질 자동 진행량을
        // 사용자가 같은 popup에서 조정할 수 있게 하기 위한 값입니다.
        max_auto_turns_label: app.current_max_auto_turns_label(),
        // 학습 주석: editing flag는 status/key line builder가 일반 review controls와 text input controls 중 무엇을
        // 보여 줄지 결정하는 switch입니다.
        is_turn_budget_editing: app.is_max_auto_turns_editing(),
        // 학습 주석: turn_budget_buffer는 사용자가 입력 중인 raw text입니다. label은 committed value이고,
        // buffer는 editing mode에서 아직 검증/확정되지 않은 draft input을 보여 줍니다.
        turn_budget_buffer: app
            // 학습 주석: followup overlay state가 auto-turn budget editor를 소유합니다. planning init review는 그 상태를 공유합니다.
            .followup_overlay_ui_state
            // 학습 주석: max_auto_turns_editor는 turn budget 입력 버퍼와 cursor/editing 상태를 담는 substate입니다.
            .max_auto_turns_editor
            // 학습 주석: status copy에는 cursor가 아니라 표시할 buffer text만 필요합니다.
            .buffer
            // 학습 주석: copy DTO가 app borrow와 독립적으로 view pipeline을 통과하도록 String을 clone합니다.
            .clone(),
    }
}
