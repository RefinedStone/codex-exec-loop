use crate::application::service::planning::{
    PlanningApplicationProjection, PlanningRuntimeSnapshot, PlanningWorkerRunOutcome,
};

use super::super::super::PlanningWorkerStatus;
use super::PostTurnEvaluationExecutor;

/*
 * This extension owns the observability side of post-turn planning work.
 * Other PostTurnEvaluationExecutor methods call refresh/repair services and apply host-side follow-up
 * rules; this file keeps the TUI panel cache coherent so normal surfaces show the latest compact state
 * while debug visibility can still expose raw worker prompt/response text.
 */
impl PostTurnEvaluationExecutor {
    // Record the start of a refresh or repair worker call before the potentially long operation begins.
    pub(super) fn record_planning_worker_running(
        &mut self,
        status: PlanningWorkerStatus,
        operation_label: &str,
        prompt: String,
    ) {
        // Running state must clear result-only fields so stale rejection/error text cannot masquerade as the new call.
        self.planning_worker_panel_state.status = status;
        self.planning_worker_panel_state.last_operation_label = Some(operation_label.to_string());
        self.planning_worker_panel_state.last_summary = None;
        self.planning_worker_panel_state.last_rejected_summary = None;
        self.planning_worker_panel_state.last_notice_detail = None;
        self.planning_worker_panel_state.last_prompt = Some(prompt);
        self.planning_worker_panel_state.last_response = None;
        self.planning_worker_panel_state.last_host_detail = None;
    }

    // Fold a successful worker RPC into the panel cache.
    // A syntactically successful response can still be a failed post-turn outcome when it requests repair or
    // produces a runtime snapshot that blocks auto-follow, so the status is corrected before rendering sees it.
    pub(super) fn record_planning_worker_outcome(
        &mut self,
        success_status: PlanningWorkerStatus,
        outcome: &PlanningWorkerRunOutcome,
    ) {
        self.planning_worker_panel_state.status =
            if outcome.repair_request.is_some() || outcome.runtime_snapshot.blocks_auto_follow() {
                // Preserve the caller's operation lane while downgrading success to the matching attention state.
                match success_status {
                    PlanningWorkerStatus::RefreshSucceeded => PlanningWorkerStatus::RefreshFailed,
                    PlanningWorkerStatus::RepairSucceeded => PlanningWorkerStatus::RepairFailed,
                    _ => success_status,
                }
            } else {
                success_status
            };
        // Accepted and rejected summaries stay separate so the panel can explain both the chosen path and discarded alternatives.
        self.planning_worker_panel_state.last_summary = outcome.worker_summary.clone();
        self.planning_worker_panel_state.last_rejected_summary = outcome.rejected_summary.clone();
        // Queue summary is derived from the applied runtime snapshot, not from worker prose.
        self.planning_worker_panel_state.last_queue_summary =
            planning_worker_queue_summary(&outcome.runtime_snapshot);
        // Notice detail keeps diagnostics after summary-prefixed notices have been de-duplicated.
        self.planning_worker_panel_state.last_notice_detail =
            planning_worker_notice_detail(&outcome.notices);
        // Raw worker response is retained for debug visibility, but normal copy reads the projected fields above.
        self.planning_worker_panel_state.last_response = outcome.worker_response.clone();
        // Host details are set by later orchestration steps such as proposal promotion or repeated-head detection.
        self.planning_worker_panel_state.last_host_detail = None;
    }

    // Record worker or host-side failure while preserving the best runtime snapshot available at the failure point.
    pub(super) fn record_planning_worker_failure(
        &mut self,
        status: PlanningWorkerStatus,
        detail: &str,
        runtime_snapshot: &PlanningRuntimeSnapshot,
    ) {
        // Keep the prompt from the running phase, but clear successful-response fields that no longer apply.
        self.planning_worker_panel_state.status = status;
        self.planning_worker_panel_state.last_summary = Some(detail.to_string());
        self.planning_worker_panel_state.last_rejected_summary = None;
        self.planning_worker_panel_state.last_queue_summary =
            planning_worker_queue_summary(runtime_snapshot);
        self.planning_worker_panel_state.last_notice_detail = None;
        self.planning_worker_panel_state.last_response = None;
        self.planning_worker_panel_state.last_host_detail = None;
    }
}

// Compact queue state used by both the planning worker panel and post-turn host diagnostics.
pub(super) fn planning_worker_queue_summary(snapshot: &PlanningRuntimeSnapshot) -> Option<String> {
    let projection = PlanningApplicationProjection::from_runtime_snapshot(snapshot);
    planning_worker_queue_summary_from_projection(&projection)
}

fn planning_worker_queue_summary_from_projection(
    projection: &PlanningApplicationProjection,
) -> Option<String> {
    projection
        // The executable queue head is more actionable than aggregate queue text, so it wins when present.
        .queue_head
        .as_ref()
        .map(|queue_head| format!("queue head: {}", queue_head.task_title.trim()))
        // If there is no executable head, fall back to the projection's own idle/invalid summary.
        .or_else(|| projection.queue_summary.clone())
}

// Collapse non-summary worker notices into the secondary diagnostic lane.
fn planning_worker_notice_detail(notices: &[String]) -> Option<String> {
    let detail = notices
        .iter()
        // Refresh/repair summary notices duplicate dedicated fields, so keep only extra reasons and warnings.
        .filter(|notice| {
            !notice.starts_with("planning worker refresh summary:")
                && !notice.starts_with("planning worker repair summary:")
        })
        .map(String::as_str)
        .collect::<Vec<_>>()
        .join(" | ");

    (!detail.is_empty()).then_some(detail)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::planning::{PriorityQueueTask, TaskStatus};

    #[test]
    fn planning_worker_queue_summary_projects_queue_head_from_application_projection() {
        let snapshot = PlanningRuntimeSnapshot::ready(
            "Planning Context".to_string(),
            "queue has 2 ready tasks".to_string(),
            Some(queue_task("task-1", "Implement projection")),
        );
        let projection = PlanningApplicationProjection::from_runtime_snapshot(&snapshot);

        assert_eq!(
            planning_worker_queue_summary_from_projection(&projection).as_deref(),
            Some("queue head: Implement projection")
        );
        assert_eq!(
            planning_worker_queue_summary(&snapshot).as_deref(),
            Some("queue head: Implement projection")
        );
    }

    #[test]
    fn planning_worker_queue_summary_projection_falls_back_to_queue_summary() {
        let snapshot = PlanningRuntimeSnapshot::ready(
            "Planning Context".to_string(),
            "no executable tasks; proposals available".to_string(),
            None,
        );
        let projection = PlanningApplicationProjection::from_runtime_snapshot(&snapshot);

        assert_eq!(
            planning_worker_queue_summary_from_projection(&projection).as_deref(),
            Some("no executable tasks; proposals available")
        );
    }

    fn queue_task(task_id: &str, task_title: &str) -> PriorityQueueTask {
        PriorityQueueTask {
            rank: 1,
            task_id: task_id.to_string(),
            direction_id: "direction-a".to_string(),
            direction_title: "Direction A".to_string(),
            task_title: task_title.to_string(),
            status: TaskStatus::Ready,
            combined_priority: 90,
            updated_at: "2026-05-08T00:00:00Z".to_string(),
            rank_reasons: vec!["priority".to_string()],
        }
    }
}
