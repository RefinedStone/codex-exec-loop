use crate::application::service::planning::{PlanningRuntimeSnapshot, PlanningWorkerRunOutcome};

use super::super::super::PlannerWorkerStatus;
use super::PostTurnEvaluationExecutor;

/*
 * This extension owns the observability side of post-turn planning work.
 * Other PostTurnEvaluationExecutor methods call refresh/repair services and apply host-side follow-up
 * rules; this file keeps the TUI panel cache coherent so normal surfaces show the latest compact state
 * while debug visibility can still expose raw worker prompt/response text.
 */
impl PostTurnEvaluationExecutor {
    // Record the start of a refresh or repair worker call before the potentially long operation begins.
    pub(super) fn record_planner_worker_running(
        &mut self,
        status: PlannerWorkerStatus,
        operation_label: &str,
        prompt: String,
    ) {
        // Running state must clear result-only fields so stale rejection/error text cannot masquerade as the new call.
        self.planner_worker_panel_state.status = status;
        self.planner_worker_panel_state.last_operation_label = Some(operation_label.to_string());
        self.planner_worker_panel_state.last_summary = None;
        self.planner_worker_panel_state.last_rejected_summary = None;
        self.planner_worker_panel_state.last_notice_detail = None;
        self.planner_worker_panel_state.last_prompt = Some(prompt);
        self.planner_worker_panel_state.last_response = None;
        self.planner_worker_panel_state.last_host_detail = None;
    }

    // Fold a successful worker RPC into the panel cache.
    // A syntactically successful response can still be a failed post-turn outcome when it requests repair or
    // produces a runtime snapshot that blocks auto-follow, so the status is corrected before rendering sees it.
    pub(super) fn record_planner_worker_outcome(
        &mut self,
        success_status: PlannerWorkerStatus,
        outcome: &PlanningWorkerRunOutcome,
    ) {
        self.planner_worker_panel_state.status = if outcome.repair_request.is_some()
            || outcome.runtime_snapshot.blocks_auto_followup()
        {
            // Preserve the caller's operation lane while downgrading success to the matching attention state.
            match success_status {
                PlannerWorkerStatus::RefreshSucceeded => PlannerWorkerStatus::RefreshFailed,
                PlannerWorkerStatus::RepairSucceeded => PlannerWorkerStatus::RepairFailed,
                _ => success_status,
            }
        } else {
            success_status
        };
        // Accepted and rejected summaries stay separate so the panel can explain both the chosen path and discarded alternatives.
        self.planner_worker_panel_state.last_summary = outcome.worker_summary.clone();
        self.planner_worker_panel_state.last_rejected_summary = outcome.rejected_summary.clone();
        // Queue summary is derived from the applied runtime snapshot, not from worker prose.
        self.planner_worker_panel_state.last_queue_summary =
            planner_queue_summary(&outcome.runtime_snapshot);
        // Notice detail keeps diagnostics after summary-prefixed notices have been de-duplicated.
        self.planner_worker_panel_state.last_notice_detail =
            planner_notice_detail(&outcome.notices);
        // Raw worker response is retained for debug visibility, but normal copy reads the projected fields above.
        self.planner_worker_panel_state.last_response = outcome.worker_response.clone();
        // Host details are set by later orchestration steps such as proposal promotion or repeated-head detection.
        self.planner_worker_panel_state.last_host_detail = None;
    }

    // Record worker or host-side failure while preserving the best runtime snapshot available at the failure point.
    pub(super) fn record_planner_worker_failure(
        &mut self,
        status: PlannerWorkerStatus,
        detail: &str,
        runtime_snapshot: &PlanningRuntimeSnapshot,
    ) {
        // Keep the prompt from the running phase, but clear successful-response fields that no longer apply.
        self.planner_worker_panel_state.status = status;
        self.planner_worker_panel_state.last_summary = Some(detail.to_string());
        self.planner_worker_panel_state.last_rejected_summary = None;
        self.planner_worker_panel_state.last_queue_summary =
            planner_queue_summary(runtime_snapshot);
        self.planner_worker_panel_state.last_notice_detail = None;
        self.planner_worker_panel_state.last_response = None;
        self.planner_worker_panel_state.last_host_detail = None;
    }
}

// Compact queue state used by both the planner worker panel and post-turn host diagnostics.
pub(super) fn planner_queue_summary(snapshot: &PlanningRuntimeSnapshot) -> Option<String> {
    snapshot
        // The executable queue head is more actionable than aggregate queue text, so it wins when present.
        .queue_head()
        .map(|queue_head| format!("next task: {}", queue_head.task_title.trim()))
        // If there is no executable head, fall back to the snapshot's own idle/invalid summary.
        .or_else(|| snapshot.queue_summary().map(str::to_string))
}

// Collapse non-summary worker notices into the secondary diagnostic lane.
fn planner_notice_detail(notices: &[String]) -> Option<String> {
    let detail = notices
        .iter()
        // Refresh/repair summary notices duplicate dedicated fields, so keep only extra reasons and warnings.
        .filter(|notice| {
            !notice.starts_with("planner refresh summary:")
                && !notice.starts_with("planner repair summary:")
        })
        .map(String::as_str)
        .collect::<Vec<_>>()
        .join(" | ");

    (!detail.is_empty()).then_some(detail)
}
