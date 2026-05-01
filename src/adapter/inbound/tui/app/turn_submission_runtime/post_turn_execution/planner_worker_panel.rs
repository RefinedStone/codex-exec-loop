use crate::application::service::planning::{PlanningRuntimeSnapshot, PlanningWorkerRunOutcome};

use super::super::super::PlannerWorkerStatus;
use super::PostTurnEvaluationExecutor;

impl PostTurnEvaluationExecutor {
    pub(super) fn record_planner_worker_running(
        &mut self,
        status: PlannerWorkerStatus,
        operation_label: &str,
        prompt: String,
    ) {
        self.planner_worker_panel_state.status = status;
        self.planner_worker_panel_state.last_operation_label = Some(operation_label.to_string());
        self.planner_worker_panel_state.last_summary = None;
        self.planner_worker_panel_state.last_rejected_summary = None;
        self.planner_worker_panel_state.last_notice_detail = None;
        self.planner_worker_panel_state.last_prompt = Some(prompt);
        self.planner_worker_panel_state.last_response = None;
        self.planner_worker_panel_state.last_host_detail = None;
    }

    pub(super) fn record_planner_worker_outcome(
        &mut self,
        success_status: PlannerWorkerStatus,
        outcome: &PlanningWorkerRunOutcome,
    ) {
        self.planner_worker_panel_state.status = if outcome.repair_request.is_some()
            || outcome.runtime_snapshot.blocks_auto_followup()
        {
            match success_status {
                PlannerWorkerStatus::RefreshSucceeded => PlannerWorkerStatus::RefreshFailed,
                PlannerWorkerStatus::RepairSucceeded => PlannerWorkerStatus::RepairFailed,
                _ => success_status,
            }
        } else {
            success_status
        };
        self.planner_worker_panel_state.last_summary = outcome.worker_summary.clone();
        self.planner_worker_panel_state.last_rejected_summary = outcome.rejected_summary.clone();
        self.planner_worker_panel_state.last_queue_summary =
            planner_queue_summary(&outcome.runtime_snapshot);
        self.planner_worker_panel_state.last_notice_detail =
            planner_notice_detail(&outcome.notices);
        self.planner_worker_panel_state.last_response = outcome.worker_response.clone();
        self.planner_worker_panel_state.last_host_detail = None;
    }

    pub(super) fn record_planner_worker_failure(
        &mut self,
        status: PlannerWorkerStatus,
        detail: &str,
        runtime_snapshot: &PlanningRuntimeSnapshot,
    ) {
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

pub(super) fn planner_queue_summary(snapshot: &PlanningRuntimeSnapshot) -> Option<String> {
    snapshot
        .queue_head()
        .map(|queue_head| format!("next task: {}", queue_head.task_title.trim()))
        .or_else(|| snapshot.queue_summary().map(str::to_string))
}

fn planner_notice_detail(notices: &[String]) -> Option<String> {
    let detail = notices
        .iter()
        .filter(|notice| {
            !notice.starts_with("planner refresh summary:")
                && !notice.starts_with("planner repair summary:")
        })
        .map(String::as_str)
        .collect::<Vec<_>>()
        .join(" | ");

    (!detail.is_empty()).then_some(detail)
}
