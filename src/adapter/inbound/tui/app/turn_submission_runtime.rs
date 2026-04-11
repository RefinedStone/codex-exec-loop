use std::sync::mpsc;
use std::thread;

use crate::application::service::planning_reconciliation_service::{
    PlanningReconciliationResult, PlanningRepairRetryReason, build_planning_repair_prompt,
};
use crate::domain::conversation::ConversationStreamEvent;
use crate::domain::planning::{DIRECTIONS_FILE_PATH, TASK_LEDGER_FILE_PATH};

use super::conversation_model::PlanningRepairState;
use super::conversation_runtime::{ConversationPostTurnAction, ConversationPostTurnEvaluation};
use super::*;

const MAX_PLANNING_REPAIR_ATTEMPTS: usize = 2;

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
    pub(super) fn start_turn_submission(&mut self) {
        let inline_command = match &self.conversation_state {
            ConversationState::Ready(conversation) => {
                InlineShellCommand::parse(&conversation.input_buffer)
            }
            _ => None,
        };
        if let Some(command) = inline_command {
            self.execute_inline_shell_command(command);
            return;
        }

        let prompt = match &self.conversation_state {
            ConversationState::Ready(conversation) if conversation.can_submit_prompt() => {
                self.assemble_manual_prompt(conversation)
            }
            _ => return,
        };
        let Some(prompt) = prompt else {
            return;
        };
        self.submit_prompt(prompt, PromptOrigin::Manual);
    }

    pub(super) fn execute_conversation_runtime_effect(
        &mut self,
        effect: ConversationRuntimeEffect,
    ) {
        match effect {
            ConversationRuntimeEffect::StartStream {
                workspace_directory,
                thread_id,
                prompt,
            } => {
                self.active_turn_planning_snapshot =
                    Some(self.load_planning_execution_snapshot(&workspace_directory));
                let outer_tx = self.tx.clone();
                let service = self.conversation_service.clone();
                thread::spawn(move || {
                    let (event_tx, event_rx) = mpsc::channel();

                    let service_thread = thread::spawn(move || {
                        let result = match thread_id {
                            Some(thread_id) => {
                                service.run_turn_stream(&thread_id, &prompt, event_tx)
                            }
                            None => service.run_new_thread_stream(
                                &workspace_directory,
                                &prompt,
                                event_tx,
                            ),
                        };
                        let _ = result;
                    });

                    while let Ok(event) = event_rx.recv() {
                        let should_stop = matches!(
                            event,
                            ConversationStreamEvent::TurnCompleted { .. }
                                | ConversationStreamEvent::Failed { .. }
                        );
                        let _ = outer_tx.send(BackgroundMessage::ConversationStream(event));
                        if should_stop {
                            break;
                        }
                    }

                    let _ = service_thread.join();
                });
            }
            ConversationRuntimeEffect::EvaluateAutoFollowup {
                workspace_directory,
                queued_from_turn_id,
                changed_planning_file_paths,
            } => self.evaluate_auto_followup_after_turn(
                workspace_directory,
                queued_from_turn_id,
                changed_planning_file_paths,
            ),
            ConversationRuntimeEffect::QueueAutoPrompt {
                prompt,
                queued_from_turn_id,
                template_label,
                transcript_text,
            } => {
                self.submit_prompt(
                    prompt,
                    PromptOrigin::AutoFollow(AutoFollowupSubmitContext {
                        queued_from_turn_id,
                        template_label,
                        transcript_text,
                    }),
                );
            }
            ConversationRuntimeEffect::QueuePlanningRepairPrompt {
                prompt,
                queued_from_turn_id,
                attempt_number,
                max_attempts,
            } => {
                self.submit_prompt(
                    prompt,
                    PromptOrigin::PlanningRepair(PlanningRepairSubmitContext {
                        queued_from_turn_id,
                        attempt_number,
                        max_attempts,
                    }),
                );
            }
        }
    }

    fn evaluate_auto_followup_after_turn(
        &mut self,
        workspace_directory: String,
        queued_from_turn_id: String,
        changed_planning_file_paths: Vec<String>,
    ) {
        let Some(conversation) = self.take_ready_conversation_state() else {
            return;
        };

        let evaluation = self.build_post_turn_evaluation(
            &conversation,
            &workspace_directory,
            &queued_from_turn_id,
            &changed_planning_file_paths,
        );
        self.conversation_state = ConversationState::Ready(conversation);
        self.dispatch_conversation_runtime(ConversationRuntimeEvent::PostTurnEvaluated {
            evaluation: Box::new(evaluation),
        });
    }

    fn build_post_turn_evaluation(
        &mut self,
        conversation: &ConversationViewModel,
        workspace_directory: &str,
        queued_from_turn_id: &str,
        changed_planning_file_paths: &[String],
    ) -> ConversationPostTurnEvaluation {
        let reconciliation_result = self.reconcile_planning_after_turn(
            workspace_directory,
            queued_from_turn_id,
            changed_planning_file_paths,
        );
        let planning_repair_resolution = self.resolve_planning_repair_after_turn(
            conversation,
            queued_from_turn_id,
            changed_planning_file_paths,
            &reconciliation_result,
        );
        let planning_runtime_snapshot = if let Some(block_reason) = planning_repair_resolution
            .block_reason
            .clone()
            .or_else(|| reconciliation_result.auto_followup_block_reason.clone())
        {
            crate::application::service::planning_prompt_service::PlanningRuntimeSnapshot::invalid(
                block_reason,
            )
        } else if changed_planning_file_paths.is_empty() {
            conversation.planning_runtime_snapshot.clone()
        } else {
            self.load_planning_runtime_snapshot(workspace_directory)
        };

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
                super::AutoFollowupDecision::QueuePrompt(queued_prompt) => {
                    ConversationPostTurnAction::QueueAutoPrompt {
                        prompt: queued_prompt.prompt,
                        queued_from_turn_id: queued_from_turn_id.to_string(),
                        template_label: conversation.auto_follow_state.template_label().to_string(),
                        transcript_text: queued_prompt.transcript_text,
                    }
                }
                super::AutoFollowupDecision::Skip(reason) => {
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

    fn load_planning_execution_snapshot(
        &self,
        workspace_directory: &str,
    ) -> ActiveTurnPlanningSnapshot {
        match self
            .planning_services
            .runtime_facade
            .load_execution_snapshot(workspace_directory)
        {
            Ok(snapshot) => ActiveTurnPlanningSnapshot::Ready(snapshot),
            Err(error) => ActiveTurnPlanningSnapshot::CaptureFailed(format!(
                "planning reconciliation could not capture the accepted planning snapshot before the turn started: {error}"
            )),
        }
    }

    fn reconcile_planning_after_turn(
        &mut self,
        workspace_directory: &str,
        turn_id: &str,
        changed_planning_file_paths: &[String],
    ) -> PlanningReconciliationResult {
        let requires_execution_snapshot = changed_planning_file_paths
            .iter()
            .any(|path| path == DIRECTIONS_FILE_PATH || path == TASK_LEDGER_FILE_PATH);

        if !requires_execution_snapshot {
            self.active_turn_planning_snapshot = None;
            return PlanningReconciliationResult::default();
        }

        let Some(snapshot_state) = self.active_turn_planning_snapshot.take() else {
            return PlanningReconciliationResult {
                notices: vec![
                    "planning reconciliation could not restore protected planning files because the turn snapshot was unavailable"
                        .to_string(),
                ],
                auto_followup_block_reason: Some(
                    "planning reconciliation could not restore protected planning files because the turn snapshot was unavailable"
                        .to_string(),
                ),
                ..PlanningReconciliationResult::default()
            };
        };

        let execution_snapshot = match snapshot_state {
            ActiveTurnPlanningSnapshot::Ready(snapshot) => snapshot,
            ActiveTurnPlanningSnapshot::CaptureFailed(error_message) => {
                return PlanningReconciliationResult {
                    notices: vec![error_message.clone()],
                    auto_followup_block_reason: Some(error_message),
                    ..PlanningReconciliationResult::default()
                };
            }
        };

        match self.planning_services.runtime_facade.reconcile_after_turn(
            workspace_directory,
            turn_id,
            changed_planning_file_paths,
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
        queued_from_turn_id: &str,
        changed_planning_file_paths: &[String],
        reconciliation_result: &PlanningReconciliationResult,
    ) -> PlanningRepairResolution {
        if let Some(repair_request) = reconciliation_result.repair_request.as_ref() {
            let retry_reason = conversation
                .planning_repair_state
                .as_ref()
                .map(|_| PlanningRepairRetryReason::TaskLedgerStillInvalid);
            return self.queue_planning_repair_attempt(
                conversation,
                queued_from_turn_id,
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

        let task_ledger_changed = changed_planning_file_paths
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
            active_repair_state.root_turn_id.as_str(),
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
        let next_planning_repair_state = Some(PlanningRepairState {
            root_turn_id: root_turn_id.to_string(),
            attempts_used: next_attempt,
            max_attempts,
            latest_request: repair_request.clone(),
        });
        PlanningRepairResolution {
            next_planning_repair_state,
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

    pub(super) fn resolve_startup_submit_queue(&mut self) {
        let (startup_submit_armed, prompt) = match &self.conversation_state {
            ConversationState::Ready(conversation) => (
                conversation.startup_submit_armed,
                self.assemble_manual_prompt(conversation),
            ),
            ConversationState::Loading | ConversationState::Failed(_) => return,
        };
        if !startup_submit_armed {
            return;
        }

        match self.shell_action_availability() {
            super::ShellActionAvailability::Ready if prompt.is_none() => {
                self.dispatch_conversation_input(ConversationInputEvent::StartupSubmitDisarmed {
                    status_text: None,
                });
            }
            super::ShellActionAvailability::Ready => {
                let prompt =
                    prompt.expect("ready startup submit should preserve a non-empty prompt");
                self.submit_prompt(prompt, PromptOrigin::Manual);
            }
            super::ShellActionAvailability::Pending => {}
            super::ShellActionAvailability::Blocked => {
                self.dispatch_conversation_input(ConversationInputEvent::StartupSubmitDisarmed {
                    status_text: Some(format!(
                        "{}; queued prompt kept in buffer",
                        self.submission_blocked_status(PromptOrigin::Manual)
                    )),
                });
            }
        }
    }

    pub(super) fn submit_prompt(&mut self, prompt: String, prompt_origin: PromptOrigin) {
        if matches!(prompt_origin, PromptOrigin::Manual)
            && matches!(
                self.shell_action_availability(),
                super::ShellActionAvailability::Pending
            )
        {
            self.dispatch_conversation_input(ConversationInputEvent::StartupSubmitArmed {
                status_text: "prompt queued until startup checks finish".to_string(),
            });
            return;
        }

        if !self.shell_action_availability().allows_actions() {
            self.dispatch_conversation_input(ConversationInputEvent::StatusMessageShown {
                status_text: self.submission_blocked_status(prompt_origin),
            });
            return;
        }

        self.dispatch_conversation_runtime(ConversationRuntimeEvent::SubmitPrompt {
            prompt,
            origin: prompt_origin,
        });
    }

    fn assemble_manual_prompt(&self, conversation: &ConversationViewModel) -> Option<String> {
        self.planning_services.runtime_facade.build_manual_prompt(
            &conversation.input_buffer,
            &conversation.planning_runtime_snapshot,
        )
    }
}
