use crate::application::service::planning::{
    PlanningApplicationProjection, PlanningRuntimeSnapshot, PlanningRuntimeWorkspaceStatus,
};

use super::super::super::super::status_panels::plan_runtime_substate_label;
use super::super::super::super::{FOOTER_NOTICE_DETAIL_LIMIT, compact_inline_detail};
use super::copy::PlanningExistingWorkspaceCopy;

// existing workspace popup은 이미 planning artifact가 있는 directory에서 새 init을
// 진행하려 할 때 뜨는 guard 화면이다. 여기서 runtime snapshot을 문자열 copy로
// 낮춰 두면 view builder는 application enum이나 queue policy shape를 몰라도 된다.
pub(super) fn build_existing_workspace_copy(
    workspace_directory: &str,
    snapshot: &PlanningRuntimeSnapshot,
) -> PlanningExistingWorkspaceCopy {
    let projection = PlanningApplicationProjection::from_runtime_snapshot(snapshot);
    // footer/status panel과 같은 substate vocabulary를 써서 modal의 상태 문구가
    // shell 하단의 live planning 표시와 어긋나지 않게 한다.
    let plan_state_label = format!("Plan / {}", plan_runtime_substate_label(snapshot));
    // queue/failure detail은 작은 modal 안에 들어가므로 footer notice와 같은 제한으로
    // 자른다. 정보가 없을 때는 빈 문자열 대신 unavailable copy를 넣어 감지 실패와
    // 정상 idle 상태가 구분되게 한다.
    let queue_summary = projection
        .queue_summary
        .as_deref()
        .map(|summary| compact_inline_detail(summary, FOOTER_NOTICE_DETAIL_LIMIT))
        .unwrap_or_else(|| "queue state unavailable".to_string());
    let failure_summary = (projection.workspace_status == PlanningRuntimeWorkspaceStatus::Invalid)
        .then_some(projection.status_detail.as_deref())
        .flatten()
        .map(|summary| compact_inline_detail(summary, FOOTER_NOTICE_DETAIL_LIMIT));

    PlanningExistingWorkspaceCopy {
        workspace_directory: workspace_directory.to_string(),
        plan_state_label,
        queue_summary,
        // queue-idle policy는 renderer가 분기하지 않도록 여기서 최종 label로 고정한다.
        queue_idle_policy: projection.queue_idle_policy.label().to_string(),
        failure_summary,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::planning::QueueIdlePolicy;

    #[test]
    fn existing_workspace_copy_reads_queue_and_policy_from_application_projection() {
        let snapshot = PlanningRuntimeSnapshot::ready(
            "Planning Context".to_string(),
            "queue has one ready task".to_string(),
            None,
        )
        .with_queue_idle_policy(
            QueueIdlePolicy::ReviewAndEnqueue,
            Some(".codex-exec-loop/planning/prompts/queue-idle-review.md".to_string()),
        );

        let copy = build_existing_workspace_copy("/tmp/workspace", &snapshot);

        assert_eq!(copy.plan_state_label, "Plan / idle");
        assert_eq!(copy.queue_summary, "queue has one ready task");
        assert_eq!(copy.queue_idle_policy, "review_and_enqueue");
        assert!(copy.failure_summary.is_none());
    }

    #[test]
    fn existing_workspace_copy_projects_invalid_detail_as_failure_summary() {
        let snapshot = PlanningRuntimeSnapshot::invalid("planning invalid");

        let copy = build_existing_workspace_copy("/tmp/workspace", &snapshot);

        assert_eq!(copy.plan_state_label, "Plan / invalid");
        assert_eq!(copy.queue_summary, "queue state unavailable");
        assert_eq!(copy.failure_summary.as_deref(), Some("planning invalid"));
    }
}
