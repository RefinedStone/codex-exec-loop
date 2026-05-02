// 학습 주석: runtime snapshot은 existing workspace 화면이 보여 줄 planning runtime state의 source입니다.
use crate::application::service::planning::PlanningRuntimeSnapshot;

// 학습 주석: substate label helper는 runtime snapshot의 내부 상태를 footer/status copy와 같은 문구 체계로 바꿉니다.
use super::super::super::super::status_panels::plan_runtime_substate_label;
// 학습 주석: existing workspace overlay는 작은 modal이라 runtime detail을 footer와 같은 길이 제한으로 압축합니다.
use super::super::super::super::{FOOTER_NOTICE_DETAIL_LIMIT, compact_inline_detail};
// 학습 주석: copy DTO는 app/runtime state를 renderer-facing text로 옮기는 intermediate object입니다.
use super::copy::PlanningExistingWorkspaceCopy;

// 학습 주석: 이 함수는 existing planning workspace detection 결과와 runtime snapshot을 modal copy로 변환합니다.
// router/view builder가 application snapshot type에 직접 의존하지 않게, 필요한 presentation field만 추출합니다.
pub(super) fn build_existing_workspace_copy(
    // 학습 주석: workspace_directory는 이미 accepted planning artifacts가 발견된 root path입니다.
    workspace_directory: &str,
    // 학습 주석: snapshot은 current plan state, queue summary, failure reason, queue idle policy를 제공합니다.
    snapshot: &PlanningRuntimeSnapshot,
) -> PlanningExistingWorkspaceCopy {
    // 학습 주석: plan_state_label은 fixed "Plan / ..." prefix와 runtime substate를 합쳐 option line에 들어갈
    // compact state copy를 만듭니다.
    let plan_state_label = format!("Plan / {}", plan_runtime_substate_label(snapshot));
    // 학습 주석: queue summary는 optional입니다. snapshot이 queue 정보를 제공하면 inline 길이로 압축하고,
    // 없으면 unavailable 문구로 상태 공백을 명시합니다.
    let queue_summary = snapshot
        .queue_summary()
        .map(|summary| compact_inline_detail(summary, FOOTER_NOTICE_DETAIL_LIMIT))
        .unwrap_or_else(|| "queue state unavailable".to_string());
    // 학습 주석: failure summary도 optional입니다. failure가 있을 때만 status area에 추가 warning line을
    // 렌더링할 수 있도록 copy에 Option으로 유지합니다.
    let failure_summary = snapshot
        .failure_reason()
        .map(|summary| compact_inline_detail(summary, FOOTER_NOTICE_DETAIL_LIMIT));

    PlanningExistingWorkspaceCopy {
        // 학습 주석: workspace path는 option line에 그대로 표시되므로 owned String으로 copy에 담습니다.
        workspace_directory: workspace_directory.to_string(),
        plan_state_label,
        queue_summary,
        // 학습 주석: queue idle policy는 runtime snapshot enum label을 문자열로 고정해 view layer가 enum을 몰라도 되게 합니다.
        queue_idle_policy: snapshot.queue_idle_policy().label().to_string(),
        failure_summary,
    }
}
