use crate::application::service::parallel_mode::ParallelModeOfficialCompletionReport;
use crate::application::service::planning::{
    PlanningOfficialCompletionRefreshRequest, PlanningRuntimeSnapshot,
    PlanningRuntimeWorkspaceStatus,
};

use super::super::super::{ConversationViewModel, PlannerWorkerStatus};
use super::queue_head_detail::repeated_queue_head_detail;
use super::{
    OFFICIAL_COMPLETION_REFRESH_FAILURE_BLOCK_REASON, OfficialCompletionRefreshOutcome,
    PostTurnEvaluationExecutor, PostTurnEvaluationRequest,
};

impl PostTurnEvaluationExecutor {
    pub(super) fn begin_official_completion_if_needed(
        &mut self,
        conversation: &ConversationViewModel,
        request: &PostTurnEvaluationRequest,
    ) -> Option<ParallelModeOfficialCompletionReport> {
        let latest_main_reply = conversation
            .latest_agent_message_text()
            .map(str::trim)
            .filter(|message| !message.is_empty());
        let validation_summary = if request.changed_planning_file_paths.is_empty() {
            "turn completed without planning file changes"
        } else {
            "turn completed with planning file changes; protected planning files were reconciled before official refresh"
        };

        match self.parallel_mode_turn_service.begin_official_completion(
            &request.workspace_directory,
            &request.queued_from_turn_id,
            None,
            latest_main_reply,
            Some(validation_summary),
        ) {
            Ok(report) => report,
            Err(error) => {
                self.record_planner_worker_failure(
                    PlannerWorkerStatus::RefreshFailed,
                    &format!("parallel completion capture failed: {error}"),
                    &conversation.planning_runtime_snapshot,
                );
                None
            }
        }
    }

    pub(super) fn run_official_completion_refresh(
        &mut self,
        conversation: &ConversationViewModel,
        request: &PostTurnEvaluationRequest,
        planning_workspace_directory: &str,
        current_snapshot: &PlanningRuntimeSnapshot,
        completion_report: &ParallelModeOfficialCompletionReport,
    ) -> OfficialCompletionRefreshOutcome {
        let planning_workspace_snapshot =
            if planning_workspace_directory == request.workspace_directory {
                current_snapshot.clone()
            } else {
                self.planning
                    .runtime
                    .load_runtime_snapshot_or_invalid(planning_workspace_directory)
            };

        if matches!(
            planning_workspace_snapshot.workspace_status(),
            PlanningRuntimeWorkspaceStatus::Invalid | PlanningRuntimeWorkspaceStatus::Uninitialized
        ) {
            let failure_detail = planning_workspace_snapshot.preview_detail().unwrap_or(
                "official completion refresh is blocked because the planning workspace is unavailable",
            );
            self.parallel_mode_turn_service
                .mark_official_completion_failed(&request.workspace_directory, failure_detail);
            let failure_snapshot =
                official_completion_failure_snapshot(&planning_workspace_snapshot, failure_detail);
            self.record_planner_worker_failure(
                PlannerWorkerStatus::RefreshFailed,
                failure_detail,
                &failure_snapshot,
            );
            return OfficialCompletionRefreshOutcome {
                runtime_snapshot: failure_snapshot,
                runtime_notices: Vec::new(),
            };
        }

        let mut runtime_notices = Vec::new();
        if let Some(notice) = self
            .parallel_mode_turn_service
            .mark_official_completion_refreshing(&request.workspace_directory)
        {
            runtime_notices.push(notice);
        }
        let latest_main_reply = conversation
            .latest_agent_message_text()
            .map(str::trim)
            .filter(|message| !message.is_empty())
            .unwrap_or(completion_report.completion.final_response_summary.as_str());
        let worker_request = PlanningOfficialCompletionRefreshRequest {
            workspace_directory: planning_workspace_directory,
            latest_user_message: conversation.latest_user_message_text(),
            latest_main_reply,
            previous_handoff_task: conversation.last_planning_task_handoff(),
            contract: completion_report,
        };
        let worker_prompt = self
            .planning
            .worker
            .render_official_completion_refresh_prompt(&worker_request);
        self.record_planner_worker_running(
            PlannerWorkerStatus::RefreshRunning,
            "official-refresh",
            worker_prompt,
        );

        let worker_outcome = self
            .planning
            .worker
            .refresh_queue_from_official_completion(worker_request);
        let outcome = match worker_outcome {
            Ok(outcome) => outcome,
            Err(error) => {
                let detail = format!("official completion refresh failed: {error}");
                self.parallel_mode_turn_service
                    .mark_official_completion_failed(&request.workspace_directory, &detail);
                let failure_snapshot =
                    official_completion_failure_snapshot(&planning_workspace_snapshot, &detail);
                self.record_planner_worker_failure(
                    PlannerWorkerStatus::RefreshFailed,
                    &detail,
                    &failure_snapshot,
                );
                return OfficialCompletionRefreshOutcome {
                    runtime_snapshot: failure_snapshot,
                    runtime_notices,
                };
            }
        };

        self.record_planner_worker_outcome(PlannerWorkerStatus::RefreshSucceeded, &outcome);
        let mut runtime_snapshot = outcome.runtime_snapshot.clone();

        if let Some(repair_request) = outcome.repair_request.as_ref() {
            let repair_outcome = self.run_hidden_planning_repairs(
                planning_workspace_directory,
                &request.queued_from_turn_id,
                repair_request,
                conversation.last_planning_task_handoff(),
            );
            runtime_snapshot = if repair_outcome.resolved {
                repair_outcome.runtime_snapshot
            } else {
                official_completion_failure_snapshot(
                    &repair_outcome.runtime_snapshot,
                    OFFICIAL_COMPLETION_REFRESH_FAILURE_BLOCK_REASON,
                )
            };
        }

        if let Some(detail) = repeated_queue_head_detail(
            conversation.last_planning_task_handoff(),
            &planning_workspace_snapshot,
            &runtime_snapshot,
        ) {
            runtime_snapshot = runtime_snapshot.with_auto_followup_pause_reason(detail);
        }

        if runtime_snapshot.blocks_auto_followup() {
            let failure_detail = runtime_snapshot
                .preview_detail()
                .unwrap_or(OFFICIAL_COMPLETION_REFRESH_FAILURE_BLOCK_REASON);
            self.parallel_mode_turn_service
                .mark_official_completion_failed(&request.workspace_directory, failure_detail);
            let failure_snapshot =
                official_completion_failure_snapshot(&runtime_snapshot, failure_detail);
            self.record_planner_worker_failure(
                PlannerWorkerStatus::RefreshFailed,
                failure_detail,
                &failure_snapshot,
            );
            return OfficialCompletionRefreshOutcome {
                runtime_snapshot: failure_snapshot,
                runtime_notices,
            };
        }

        let authority_refresh_outcome = outcome
            .worker_summary
            .as_deref()
            .map(|summary| format!("official ledger refresh succeeded: {summary}"))
            .unwrap_or_else(|| "official ledger refresh succeeded".to_string());
        runtime_notices.extend(
            self.parallel_mode_turn_service
                .finalize_official_completion_success(
                    &request.workspace_directory,
                    &authority_refresh_outcome,
                ),
        );

        OfficialCompletionRefreshOutcome {
            runtime_snapshot,
            runtime_notices,
        }
    }
}

fn official_completion_failure_snapshot(
    current_snapshot: &PlanningRuntimeSnapshot,
    failure_detail: &str,
) -> PlanningRuntimeSnapshot {
    let detail = if failure_detail.trim().is_empty() {
        OFFICIAL_COMPLETION_REFRESH_FAILURE_BLOCK_REASON
    } else {
        failure_detail
    };
    current_snapshot.with_auto_followup_pause_reason(detail.to_string())
}
