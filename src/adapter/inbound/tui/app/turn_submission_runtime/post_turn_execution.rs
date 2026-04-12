use crate::application::service::planning_prompt_service::{
    PlanningRuntimeSnapshot, PlanningRuntimeWorkspaceStatus,
};
use crate::application::service::planning_proposal_promotion_service::PlanningProposalPromotionRequest;
use crate::application::service::planning_reconciliation_service::{
    PlanningExecutionSnapshot, PlanningReconciliationResult, PlanningRepairRequest,
    PlanningRepairRetryReason,
};
use crate::application::service::planning_services::PlanningServices;
use crate::application::service::planning_worker_orchestration_service::{
    PlanningLedgerRepairRequest, PlanningQueueRefreshRequest, PlanningWorkerRunOutcome,
};

#[cfg(not(test))]
use super::super::app_runtime::BackgroundMessage;
use super::super::conversation_runtime::{
    ConversationPostTurnAction, ConversationPostTurnEvaluation,
};
use super::*;

const MAX_PLANNING_REPAIR_ATTEMPTS: usize = 2;
const PLANNER_REFRESH_FAILURE_BLOCK_REASON: &str =
    "planner refresh failed; auto follow-up stays paused until the next accepted planner refresh";

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct PostTurnEvaluationRequest {
    pub workspace_directory: String,
    pub queued_from_turn_id: String,
    pub changed_planning_file_paths: Vec<String>,
}

#[derive(Debug, Clone)]
struct HiddenPlanningRepairOutcome {
    runtime_snapshot: PlanningRuntimeSnapshot,
    notices: Vec<String>,
    resolved: bool,
}

#[derive(Debug, Clone)]
struct BuiltinNextTaskRefreshOutcome {
    runtime_snapshot: PlanningRuntimeSnapshot,
    notices: Vec<String>,
}

#[derive(Debug, Clone)]
#[cfg_attr(test, allow(dead_code))]
struct PostTurnEvaluationExecution {
    thread_id: String,
    queued_from_turn_id: String,
    evaluation: ConversationPostTurnEvaluation,
    planner_worker_panel_state: PlannerWorkerPanelState,
}

#[derive(Clone)]
struct PostTurnEvaluationExecutor {
    planning_services: PlanningServices,
    active_turn_planning_capture: Option<ActiveTurnPlanningCapture>,
    planner_worker_panel_state: PlannerWorkerPanelState,
}

impl PostTurnEvaluationExecutor {
    fn new(
        planning_services: PlanningServices,
        active_turn_planning_capture: Option<ActiveTurnPlanningCapture>,
        planner_worker_panel_state: PlannerWorkerPanelState,
    ) -> Self {
        Self {
            planning_services,
            active_turn_planning_capture,
            planner_worker_panel_state,
        }
    }

    fn run(
        mut self,
        conversation: &ConversationViewModel,
        request: &PostTurnEvaluationRequest,
    ) -> PostTurnEvaluationExecution {
        let reconciliation_result = self.reconcile_planning_after_turn(request);
        let mut runtime_notices = reconciliation_result.notices.clone();
        let mut planning_runtime_snapshot = self.planning_runtime_snapshot_after_reconciliation(
            conversation,
            request,
            &reconciliation_result,
        );

        if let Some(repair_request) = reconciliation_result.repair_request.as_ref() {
            let repair_outcome = self.run_hidden_planning_repairs(
                &request.workspace_directory,
                &request.queued_from_turn_id,
                repair_request,
            );
            runtime_notices.extend(repair_outcome.notices);
            planning_runtime_snapshot = repair_outcome.runtime_snapshot;
        }

        if conversation
            .auto_follow_state
            .selected_template()
            .is_builtin_next_task()
        {
            let refresh_outcome = self.run_builtin_next_task_refresh(
                conversation,
                request,
                planning_runtime_snapshot.clone(),
            );
            runtime_notices.extend(refresh_outcome.notices);
            planning_runtime_snapshot = refresh_outcome.runtime_snapshot;
        }

        let action = self.auto_followup_action_from_snapshot(
            conversation,
            request,
            &planning_runtime_snapshot,
        );

        PostTurnEvaluationExecution {
            thread_id: conversation.thread_id.clone(),
            queued_from_turn_id: request.queued_from_turn_id.clone(),
            evaluation: ConversationPostTurnEvaluation {
                planning_runtime_snapshot,
                planning_repair_state: None,
                runtime_notices,
                action,
            },
            planner_worker_panel_state: self.planner_worker_panel_state,
        }
    }

    fn planning_runtime_snapshot_after_reconciliation(
        &self,
        conversation: &ConversationViewModel,
        request: &PostTurnEvaluationRequest,
        reconciliation_result: &PlanningReconciliationResult,
    ) -> PlanningRuntimeSnapshot {
        if let Some(block_reason) = reconciliation_result.auto_followup_block_reason.clone() {
            PlanningRuntimeSnapshot::invalid(block_reason)
        } else if request.changed_planning_file_paths.is_empty() {
            conversation.planning_runtime_snapshot.clone()
        } else {
            self.planning_services
                .runtime_facade
                .load_runtime_snapshot_or_invalid(&request.workspace_directory)
        }
    }

    fn reconcile_planning_after_turn(
        &mut self,
        request: &PostTurnEvaluationRequest,
    ) -> PlanningReconciliationResult {
        let requires_execution_snapshot = request
            .changed_planning_file_paths
            .iter()
            .any(|path| PlanningExecutionSnapshot::captures_path(path));

        if !requires_execution_snapshot {
            self.active_turn_planning_capture = None;
            return PlanningReconciliationResult::default();
        }

        let Some(capture) = self.active_turn_planning_capture.take() else {
            return blocked_reconciliation_result(
                "planning reconciliation could not restore protected planning files because the turn snapshot was unavailable"
                    .to_string(),
            );
        };

        if capture.workspace_directory != request.workspace_directory {
            return blocked_reconciliation_result(format!(
                "planning reconciliation ignored a stale planning snapshot captured for {} while the completed turn resolved in {}",
                capture.workspace_directory, request.workspace_directory
            ));
        }

        let execution_snapshot = match capture.snapshot {
            ActiveTurnPlanningSnapshot::Ready(snapshot) => snapshot,
            ActiveTurnPlanningSnapshot::CaptureFailed(error_message) => {
                return blocked_reconciliation_result(error_message);
            }
        };

        match self.planning_services.runtime_facade.reconcile_after_turn(
            &request.workspace_directory,
            &request.queued_from_turn_id,
            &request.changed_planning_file_paths,
            &execution_snapshot,
        ) {
            Ok(result) => result,
            Err(error) => PlanningReconciliationResult {
                notices: vec![format!("planning reconciliation failed: {error}")],
                auto_followup_block_reason: Some(
                    "planning reconciliation failed; auto follow-up stays paused until the planning workspace is repaired"
                        .to_string(),
                ),
                ..PlanningReconciliationResult::default()
            },
        }
    }

    fn run_hidden_planning_repairs(
        &mut self,
        workspace_directory: &str,
        root_turn_id: &str,
        repair_request: &PlanningRepairRequest,
    ) -> HiddenPlanningRepairOutcome {
        let mut notices = Vec::new();
        let mut runtime_snapshot = self
            .planning_services
            .runtime_facade
            .load_runtime_snapshot_or_invalid(workspace_directory);
        let mut next_request = repair_request.clone();
        let mut next_retry_reason = None;

        for attempt_number in 1..=MAX_PLANNING_REPAIR_ATTEMPTS {
            self.record_planner_worker_running(PlannerWorkerStatus::RepairRunning);
            let worker_outcome = self
                .planning_services
                .worker_orchestration
                .repair_task_ledger(PlanningLedgerRepairRequest {
                    workspace_directory,
                    root_turn_id,
                    repair_request: &next_request,
                    attempt_number,
                    max_attempts: MAX_PLANNING_REPAIR_ATTEMPTS,
                    retry_reason: next_retry_reason,
                });

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
                    notices.push(detail);
                    return HiddenPlanningRepairOutcome {
                        runtime_snapshot,
                        notices,
                        resolved: false,
                    };
                }
            };

            self.record_planner_worker_outcome(PlannerWorkerStatus::RepairSucceeded, &outcome);
            runtime_snapshot = outcome.runtime_snapshot.clone();
            notices.extend(outcome.notices);

            let Some(repair_request) = outcome.repair_request else {
                return HiddenPlanningRepairOutcome {
                    runtime_snapshot,
                    notices,
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
                notices.push(detail);
                return HiddenPlanningRepairOutcome {
                    runtime_snapshot,
                    notices,
                    resolved: false,
                };
            }

            next_retry_reason = Some(if outcome.task_ledger_changed {
                PlanningRepairRetryReason::TaskLedgerStillInvalid
            } else {
                PlanningRepairRetryReason::TaskLedgerUnchanged
            });
            next_request = repair_request;
        }

        HiddenPlanningRepairOutcome {
            runtime_snapshot,
            notices,
            resolved: false,
        }
    }

    fn run_builtin_next_task_refresh(
        &mut self,
        conversation: &ConversationViewModel,
        request: &PostTurnEvaluationRequest,
        current_snapshot: PlanningRuntimeSnapshot,
    ) -> BuiltinNextTaskRefreshOutcome {
        if !matches!(
            current_snapshot.workspace_status(),
            PlanningRuntimeWorkspaceStatus::ReadyNoTask
                | PlanningRuntimeWorkspaceStatus::ReadyWithTask
        ) {
            return BuiltinNextTaskRefreshOutcome {
                runtime_snapshot: current_snapshot,
                notices: Vec::new(),
            };
        }

        let Some(latest_main_reply) = conversation
            .latest_agent_message_text()
            .map(str::trim)
            .filter(|message| !message.is_empty())
        else {
            return BuiltinNextTaskRefreshOutcome {
                runtime_snapshot: current_snapshot,
                notices: Vec::new(),
            };
        };

        self.record_planner_worker_running(PlannerWorkerStatus::RefreshRunning);
        let worker_outcome = self
            .planning_services
            .worker_orchestration
            .refresh_queue_from_reply(PlanningQueueRefreshRequest {
                workspace_directory: &request.workspace_directory,
                root_turn_id: &request.queued_from_turn_id,
                latest_main_reply,
            });

        let outcome = match worker_outcome {
            Ok(outcome) => outcome,
            Err(error) => {
                let detail = format!("planner refresh failed: {error}");
                let invalid_snapshot =
                    PlanningRuntimeSnapshot::invalid(PLANNER_REFRESH_FAILURE_BLOCK_REASON);
                self.record_planner_worker_failure(
                    PlannerWorkerStatus::RefreshFailed,
                    &detail,
                    &invalid_snapshot,
                );
                return BuiltinNextTaskRefreshOutcome {
                    runtime_snapshot: invalid_snapshot,
                    notices: vec![detail],
                };
            }
        };

        self.record_planner_worker_outcome(PlannerWorkerStatus::RefreshSucceeded, &outcome);
        let mut notices = outcome.notices.clone();
        let mut runtime_snapshot = outcome.runtime_snapshot.clone();

        if let Some(repair_request) = outcome.repair_request.as_ref() {
            let repair_outcome = self.run_hidden_planning_repairs(
                &request.workspace_directory,
                &request.queued_from_turn_id,
                repair_request,
            );
            notices.extend(repair_outcome.notices);
            runtime_snapshot = if repair_outcome.resolved {
                repair_outcome.runtime_snapshot
            } else {
                PlanningRuntimeSnapshot::invalid(PLANNER_REFRESH_FAILURE_BLOCK_REASON)
            };
        }

        if !runtime_snapshot.has_actionable_queue_head()
            && runtime_snapshot.has_proposal_candidates()
        {
            let promotion_outcome = self
                .planning_services
                .proposal_promotion
                .promote_top_proposal_to_ready_if_needed(PlanningProposalPromotionRequest {
                    workspace_directory: &request.workspace_directory,
                    root_turn_id: &request.queued_from_turn_id,
                });

            match promotion_outcome {
                Ok(promotion_outcome) => {
                    notices.extend(promotion_outcome.notices);
                    runtime_snapshot = promotion_outcome.runtime_snapshot;
                    self.planner_worker_panel_state.last_queue_summary =
                        planner_queue_summary(&runtime_snapshot);
                }
                Err(error) => {
                    let detail = format!("host proposal promotion failed: {error}");
                    let invalid_snapshot =
                        PlanningRuntimeSnapshot::invalid(PLANNER_REFRESH_FAILURE_BLOCK_REASON);
                    self.record_planner_worker_failure(
                        PlannerWorkerStatus::RefreshFailed,
                        &detail,
                        &invalid_snapshot,
                    );
                    return BuiltinNextTaskRefreshOutcome {
                        runtime_snapshot: invalid_snapshot,
                        notices: vec![detail],
                    };
                }
            }
        }

        BuiltinNextTaskRefreshOutcome {
            runtime_snapshot,
            notices,
        }
    }

    fn auto_followup_action_from_snapshot(
        &self,
        conversation: &ConversationViewModel,
        request: &PostTurnEvaluationRequest,
        planning_runtime_snapshot: &PlanningRuntimeSnapshot,
    ) -> ConversationPostTurnAction {
        match conversation.decide_auto_followup_with_snapshot(
            &self.planning_services.runtime_facade,
            planning_runtime_snapshot,
        ) {
            AutoFollowupDecision::QueuePrompt(queued_prompt) => {
                ConversationPostTurnAction::QueueAutoPrompt {
                    prompt: queued_prompt.prompt,
                    queued_from_turn_id: request.queued_from_turn_id.clone(),
                    template_label: conversation.auto_follow_state.template_label().to_string(),
                    transcript_text: queued_prompt.transcript_text,
                }
            }
            AutoFollowupDecision::Skip(reason) => {
                ConversationPostTurnAction::SkipAutoFollowup { reason }
            }
        }
    }

    fn record_planner_worker_running(&mut self, status: PlannerWorkerStatus) {
        self.planner_worker_panel_state.status = status;
    }

    fn record_planner_worker_outcome(
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
    }

    fn record_planner_worker_failure(
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
    }
}

impl NativeTuiApp {
    pub(super) fn execute_post_turn_evaluation(&mut self, request: PostTurnEvaluationRequest) {
        let Some(conversation) = self.ready_conversation_snapshot() else {
            return;
        };

        self.mark_post_turn_evaluation_running(&conversation, &request);
        let executor = PostTurnEvaluationExecutor::new(
            self.planning_services.clone(),
            self.active_turn_planning_capture.take(),
            self.planner_worker_panel_state.clone(),
        );

        #[cfg(test)]
        {
            let execution = executor.run(&conversation, &request);
            self.planner_worker_panel_state = execution.planner_worker_panel_state;
            self.dispatch_conversation_runtime(ConversationRuntimeEvent::PostTurnEvaluated {
                evaluation: Box::new(execution.evaluation),
            });
        }

        #[cfg(not(test))]
        {
            let tx = self.tx.clone();
            std::thread::spawn(move || {
                let execution = executor.run(&conversation, &request);
                let _ = tx.send(BackgroundMessage::PostTurnEvaluated {
                    thread_id: execution.thread_id,
                    queued_from_turn_id: execution.queued_from_turn_id,
                    evaluation: Box::new(execution.evaluation),
                    planner_worker_panel_state: execution.planner_worker_panel_state,
                });
            });
        }
    }

    fn ready_conversation_snapshot(&self) -> Option<ConversationViewModel> {
        match &self.conversation_state {
            ConversationState::Ready(conversation) => Some(conversation.clone()),
            ConversationState::Loading | ConversationState::Failed(_) => None,
        }
    }

    fn mark_post_turn_evaluation_running(
        &mut self,
        conversation: &ConversationViewModel,
        request: &PostTurnEvaluationRequest,
    ) {
        if conversation
            .auto_follow_state
            .selected_template()
            .is_builtin_next_task()
        {
            self.planner_worker_panel_state.status = PlannerWorkerStatus::RefreshRunning;
        } else if request
            .changed_planning_file_paths
            .iter()
            .any(|path| PlanningExecutionSnapshot::captures_path(path))
        {
            self.planner_worker_panel_state.status = PlannerWorkerStatus::RepairRunning;
        }
    }
}

fn planner_queue_summary(snapshot: &PlanningRuntimeSnapshot) -> Option<String> {
    snapshot
        .queue_head()
        .map(|queue_head| format!("next task: {}", queue_head.task_title.trim()))
        .or_else(|| snapshot.queue_summary().map(str::to_string))
}

fn blocked_reconciliation_result(message: String) -> PlanningReconciliationResult {
    PlanningReconciliationResult {
        notices: vec![message.clone()],
        auto_followup_block_reason: Some(message),
        ..PlanningReconciliationResult::default()
    }
}
