use crate::application::service::planning::PlanningRuntimeSnapshot;

use super::super::super::super::status_panels::plan_runtime_substate_label;
use super::super::super::super::{FOOTER_NOTICE_DETAIL_LIMIT, compact_inline_detail};
use super::copy::PlanningExistingWorkspaceCopy;

pub(super) fn build_existing_workspace_copy(
    workspace_directory: &str,
    snapshot: &PlanningRuntimeSnapshot,
) -> PlanningExistingWorkspaceCopy {
    let plan_state_label = format!("Plan / {}", plan_runtime_substate_label(snapshot));
    let queue_summary = snapshot
        .queue_summary()
        .map(|summary| compact_inline_detail(summary, FOOTER_NOTICE_DETAIL_LIMIT))
        .unwrap_or_else(|| "queue state unavailable".to_string());
    let failure_summary = snapshot
        .failure_reason()
        .map(|summary| compact_inline_detail(summary, FOOTER_NOTICE_DETAIL_LIMIT));

    PlanningExistingWorkspaceCopy {
        workspace_directory: workspace_directory.to_string(),
        plan_state_label,
        queue_summary,
        queue_idle_policy: snapshot.queue_idle_policy().label().to_string(),
        failure_summary,
    }
}
