use crate::application::service::planning_prompt_service::PlanningRuntimeSnapshot;
use crate::application::service::planning_reconciliation_service::{
    PlanningReconciliationResult, PlanningRepairRetryReason, build_planning_repair_prompt,
};
use crate::domain::planning::{DIRECTIONS_FILE_PATH, TASK_LEDGER_FILE_PATH};

use super::super::conversation_model::PlanningRepairState;
use super::super::conversation_runtime::{
    ConversationPostTurnAction, ConversationPostTurnEvaluation,
};
use super::*;

const MAX_PLANNING_REPAIR_ATTEMPTS: usize = 2;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct PostTurnEvaluationRequest {
    pub workspace_directory: String,
    pub queued_from_turn_id: String,
    pub changed_planning_file_paths: Vec<String>,
}

#[derive(Debug, Clone)]
struct QueuedPlanningRepairPrompt {
    prompt: String,
    queued_from_turn_id: String,
    attempt_number: usize,
    max_attempts: usize,
}

#[derive(Debug, Clone, Default)]
struct PlanningRepairResolution {
    next_planning_repair_state: Option<PlanningRepairState>,
    queued_prompt: Option<QueuedPlanningRepairPrompt>,
    notices: Vec<String>,
    block_reason: Option<String>,
}

impl NativeTuiApp {
    pub(super) fn execute_post_turn_evaluation(&mut self, request: PostTurnEvaluationRequest) {
        let Some(conversation) = self.take_ready_conversation_state() else {
            return;
        };

        let evaluation = self.build_post_turn_evaluation(&conversation, &request);
        self.conversation_state = ConversationState::Ready(conversation);
        self.dispatch_conversation_runtime(ConversationRuntimeEvent::PostTurnEvaluated {
            evaluation: Box::new(evaluation),
        });
    }

    fn build_post_turn_evaluation(
        &mut self,
        conversation: &ConversationViewModel,
        request: &PostTurnEvaluationRequest,
    ) -> ConversationPostTurnEvaluation {
        let reconciliation_result = self.reconcile_planning_after_turn(request);
        let planning_repair_resolution =
            self.resolve_planning_repair_after_turn(conversation, request, &reconciliation_result);
        let planning_runtime_snapshot = self.planning_runtime_snapshot_after_turn(
            conversation,
            request,
            &reconciliation_result,
            &planning_repair_resolution,
        );

        let mut runtime_notices = reconciliation_result.notices;
        runtime_notices.extend(planning_repair_resolution.notices);

        let action = if let Some(queued_prompt) = planning_repair_resolution.queued_prompt {
            if !conversation.input_buffer.trim().is_empty() {
                runtime_notices.push(format!(
                    "planning repair retry {}/{} is waiting because manual input is buffered",
                    queued_prompt.attempt_number, queued_prompt.max_attempts
                ));
                ConversationPostTurnAction::PausePlanningRepair {
                    attempt_number: queued_prompt.attempt_number,
                    max_attempts: queued_prompt.max_attempts,
                }
            } else {
                ConversationPostTurnAction::QueuePlanningRepair {
                    prompt: queued_prompt.prompt,
                    queued_from_turn_id: queued_prompt.queued_from_turn_id,
                    attempt_number: queued_prompt.attempt_number,
                    max_attempts: queued_prompt.max_attempts,
                }
            }
        } else {
            match conversation.decide_auto_followup_with_snapshot(
                &self.planning_services.runtime_facade,
                &planning_runtime_snapshot,
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
        };

        ConversationPostTurnEvaluation {
            planning_runtime_snapshot,
            planning_repair_state: planning_repair_resolution.next_planning_repair_state,
            runtime_notices,
            action,
        }
    }

    fn planning_runtime_snapshot_after_turn(
        &mut self,
        conversation: &ConversationViewModel,
        request: &PostTurnEvaluationRequest,
        reconciliation_result: &PlanningReconciliationResult,
        planning_repair_resolution: &PlanningRepairResolution,
    ) -> PlanningRuntimeSnapshot {
        if let Some(block_reason) = planning_repair_resolution
            .block_reason
            .clone()
            .or_else(|| reconciliation_result.auto_followup_block_reason.clone())
        {
            PlanningRuntimeSnapshot::invalid(block_reason)
        } else if request.changed_planning_file_paths.is_empty() {
            conversation.planning_runtime_snapshot.clone()
        } else {
            self.load_planning_runtime_snapshot(&request.workspace_directory)
        }
    }

    fn reconcile_planning_after_turn(
        &mut self,
        request: &PostTurnEvaluationRequest,
    ) -> PlanningReconciliationResult {
        let requires_execution_snapshot = request
            .changed_planning_file_paths
            .iter()
            .any(|path| path == DIRECTIONS_FILE_PATH || path == TASK_LEDGER_FILE_PATH);

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

    fn resolve_planning_repair_after_turn(
        &self,
        conversation: &ConversationViewModel,
        request: &PostTurnEvaluationRequest,
        reconciliation_result: &PlanningReconciliationResult,
    ) -> PlanningRepairResolution {
        if let Some(repair_request) = reconciliation_result.repair_request.as_ref() {
            let retry_reason = conversation
                .planning_repair_state
                .as_ref()
                .map(|_| PlanningRepairRetryReason::TaskLedgerStillInvalid);
            return self.queue_planning_repair_attempt(
                conversation,
                &request.queued_from_turn_id,
                repair_request,
                retry_reason,
            );
        }

        let Some(active_repair_state) = conversation.planning_repair_state.clone() else {
            return PlanningRepairResolution::default();
        };

        if reconciliation_result.auto_followup_block_reason.is_some() {
            return PlanningRepairResolution::default();
        }

        let task_ledger_changed = request
            .changed_planning_file_paths
            .iter()
            .any(|path| path == TASK_LEDGER_FILE_PATH);
        if task_ledger_changed && !reconciliation_result.rejected_task_ledger {
            return PlanningRepairResolution {
                next_planning_repair_state: None,
                notices: vec![format!(
                    "planning repair accepted task-ledger.json on retry {}/{}",
                    active_repair_state.attempts_used, active_repair_state.max_attempts
                )],
                ..PlanningRepairResolution::default()
            };
        }

        self.queue_planning_repair_attempt(
            conversation,
            &active_repair_state.root_turn_id,
            &active_repair_state.latest_request,
            Some(if task_ledger_changed {
                PlanningRepairRetryReason::TaskLedgerStillInvalid
            } else {
                PlanningRepairRetryReason::TaskLedgerUnchanged
            }),
        )
    }

    fn queue_planning_repair_attempt(
        &self,
        conversation: &ConversationViewModel,
        root_turn_id: &str,
        repair_request: &crate::application::service::planning_reconciliation_service::PlanningRepairRequest,
        retry_reason: Option<PlanningRepairRetryReason>,
    ) -> PlanningRepairResolution {
        let (next_attempt, max_attempts) =
            if let Some(state) = conversation.planning_repair_state.as_ref() {
                (state.attempts_used + 1, state.max_attempts)
            } else {
                (1, MAX_PLANNING_REPAIR_ATTEMPTS)
            };

        if next_attempt > max_attempts {
            return PlanningRepairResolution {
                next_planning_repair_state: None,
                notices: vec![format!(
                    "planning repair exhausted after {max_attempts} attempts; operator intervention is required"
                )],
                block_reason: Some(format!(
                    "planning repair exhausted after {max_attempts} attempts; auto follow-up stays paused until the operator repairs task-ledger.json"
                )),
                queued_prompt: None,
            };
        }

        let prompt =
            build_planning_repair_prompt(repair_request, next_attempt, max_attempts, retry_reason);
        PlanningRepairResolution {
            next_planning_repair_state: Some(PlanningRepairState {
                root_turn_id: root_turn_id.to_string(),
                attempts_used: next_attempt,
                max_attempts,
                latest_request: repair_request.clone(),
            }),
            notices: vec![format!(
                "planning repair queued retry {next_attempt}/{max_attempts} for task-ledger.json"
            )],
            queued_prompt: Some(QueuedPlanningRepairPrompt {
                prompt,
                queued_from_turn_id: root_turn_id.to_string(),
                attempt_number: next_attempt,
                max_attempts,
            }),
            block_reason: None,
        }
    }
}

fn blocked_reconciliation_result(message: String) -> PlanningReconciliationResult {
    PlanningReconciliationResult {
        notices: vec![message.clone()],
        auto_followup_block_reason: Some(message),
        ..PlanningReconciliationResult::default()
    }
}
