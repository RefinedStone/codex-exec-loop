use crate::application::service::planning::{
    PlanningLedgerRepairRequest, PlanningRepairRequest, PlanningRepairRetryReason,
    PlanningTaskHandoff,
};

use super::super::super::PlannerWorkerStatus;
use super::{
    HiddenPlanningRepairOutcome, MAX_PLANNING_REPAIR_ATTEMPTS, PostTurnEvaluationExecutor,
};

impl PostTurnEvaluationExecutor {
    pub(super) fn run_hidden_planning_repairs(
        &mut self,
        workspace_directory: &str,
        root_turn_id: &str,
        repair_request: &PlanningRepairRequest,
        previous_handoff_task: Option<&PlanningTaskHandoff>,
    ) -> HiddenPlanningRepairOutcome {
        let mut runtime_snapshot = self
            .planning
            .runtime
            .load_runtime_snapshot_or_invalid(workspace_directory);
        let mut next_request = repair_request.clone();
        let mut next_retry_reason = None;

        for attempt_number in 1..=MAX_PLANNING_REPAIR_ATTEMPTS {
            let worker_request = PlanningLedgerRepairRequest {
                workspace_directory,
                root_turn_id,
                repair_request: &next_request,
                previous_handoff_task,
                attempt_number,
                max_attempts: MAX_PLANNING_REPAIR_ATTEMPTS,
                retry_reason: next_retry_reason,
            };
            let worker_prompt = self
                .planning
                .worker
                .render_repair_task_authority_prompt(&worker_request);
            self.record_planner_worker_running(
                PlannerWorkerStatus::RepairRunning,
                "repair",
                worker_prompt,
            );
            let worker_outcome = self.planning.worker.repair_task_authority(worker_request);

            let outcome = match worker_outcome {
                Ok(outcome) => outcome,
                Err(error) => {
                    let detail = format!(
                        "planner repair attempt {attempt_number}/{} failed: {error}",
                        MAX_PLANNING_REPAIR_ATTEMPTS
                    );
                    self.record_planner_worker_failure(
                        PlannerWorkerStatus::RepairFailed,
                        &detail,
                        &runtime_snapshot,
                    );
                    return HiddenPlanningRepairOutcome {
                        runtime_snapshot,
                        resolved: false,
                    };
                }
            };

            self.record_planner_worker_outcome(PlannerWorkerStatus::RepairSucceeded, &outcome);
            runtime_snapshot = outcome.runtime_snapshot.clone();

            let Some(repair_request) = outcome.repair_request else {
                return HiddenPlanningRepairOutcome {
                    runtime_snapshot,
                    resolved: true,
                };
            };

            if attempt_number == MAX_PLANNING_REPAIR_ATTEMPTS {
                let detail = format!(
                    "planner repair exhausted after {} attempts; the last accepted planning state was kept",
                    MAX_PLANNING_REPAIR_ATTEMPTS
                );
                self.record_planner_worker_failure(
                    PlannerWorkerStatus::RepairFailed,
                    &detail,
                    &runtime_snapshot,
                );
                return HiddenPlanningRepairOutcome {
                    runtime_snapshot,
                    resolved: false,
                };
            }

            next_retry_reason = Some(if outcome.task_authority_changed {
                PlanningRepairRetryReason::TaskAuthorityStillInvalid
            } else {
                PlanningRepairRetryReason::TaskAuthorityUnchanged
            });
            next_request = repair_request;
        }

        HiddenPlanningRepairOutcome {
            runtime_snapshot,
            resolved: false,
        }
    }
}
