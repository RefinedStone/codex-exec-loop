use crate::application::service::planning::PlanningProposalPromotionRequest;
use crate::application::service::planning::PlanningServices;
use crate::application::service::planning::PlanningTaskHandoff;
use crate::application::service::planning::{
    PlanningExecutionSnapshot, PlanningReconciliationResult, PlanningRepairRequest,
    PlanningRepairRetryReason,
};
use crate::application::service::planning::{
    PlanningLedgerRepairRequest, PlanningQueueRefreshMode, PlanningQueueRefreshRequest,
    PlanningWorkerRunOutcome,
};
use crate::application::service::planning::{
    PlanningRuntimeSnapshot, PlanningRuntimeWorkspaceStatus,
};
use crate::domain::planning::QueueIdlePolicy;

#[cfg(not(test))]
use super::super::app_runtime::BackgroundMessage;
#[cfg(test)]
use super::super::conversation_runtime::ConversationRuntimeEvent;
use super::super::conversation_runtime::{
    ConversationPostTurnAction, ConversationPostTurnEvaluation, QueuedAutoPrompt,
};
use super::super::{
    ActiveTurnPlanningCapture, ActiveTurnPlanningSnapshot, AutoFollowupDecision,
    AutoFollowupSkipReason, ConversationState, ConversationViewModel, NativeTuiApp,
    PlannerWorkerPanelState, PlannerWorkerStatus,
};

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
    resolved: bool,
}

#[derive(Debug, Clone)]
struct BuiltinNextTaskRefreshOutcome {
    runtime_snapshot: PlanningRuntimeSnapshot,
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
    planning: PlanningServices,
    active_turn_planning_capture: Option<ActiveTurnPlanningCapture>,
    planner_worker_panel_state: PlannerWorkerPanelState,
}

impl PostTurnEvaluationExecutor {
    fn new(
        planning: PlanningServices,
        active_turn_planning_capture: Option<ActiveTurnPlanningCapture>,
        planner_worker_panel_state: PlannerWorkerPanelState,
    ) -> Self {
        Self {
            planning,
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
        let runtime_notices = reconciliation_result.notices.clone();
        let mut planning_runtime_snapshot = self.planning_runtime_snapshot_after_reconciliation(
            conversation,
            request,
            &reconciliation_result,
        );
        let automation_enabled = conversation.auto_follow_state.enabled;

        if automation_enabled
            && let Some(repair_request) = reconciliation_result.repair_request.as_ref()
        {
            let repair_outcome = self.run_hidden_planning_repairs(
                &request.workspace_directory,
                &request.queued_from_turn_id,
                repair_request,
            );
            planning_runtime_snapshot = repair_outcome.runtime_snapshot;
        }

        if automation_enabled {
            let refresh_outcome = self.run_builtin_next_task_refresh(
                conversation,
                request,
                planning_runtime_snapshot.clone(),
            );
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
            self.planning
                .runtime
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

        match self.planning.runtime.reconcile_after_turn(
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
                attempt_number,
                max_attempts: MAX_PLANNING_REPAIR_ATTEMPTS,
                retry_reason: next_retry_reason,
            };
            let worker_prompt = self
                .planning
                .worker
                .render_repair_task_ledger_prompt(&worker_request);
            self.record_planner_worker_running(
                PlannerWorkerStatus::RepairRunning,
                "repair",
                worker_prompt,
            );
            let worker_outcome = self.planning.worker.repair_task_ledger(worker_request);

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

            next_retry_reason = Some(if outcome.task_ledger_changed {
                PlanningRepairRetryReason::TaskLedgerStillInvalid
            } else {
                PlanningRepairRetryReason::TaskLedgerUnchanged
            });
            next_request = repair_request;
        }

        HiddenPlanningRepairOutcome {
            runtime_snapshot,
            resolved: false,
        }
    }

    fn run_builtin_next_task_refresh(
        &mut self,
        conversation: &ConversationViewModel,
        request: &PostTurnEvaluationRequest,
        current_snapshot: PlanningRuntimeSnapshot,
    ) -> BuiltinNextTaskRefreshOutcome {
        if current_snapshot.workspace_present() && !current_snapshot.plan_enabled() {
            return BuiltinNextTaskRefreshOutcome {
                runtime_snapshot: current_snapshot,
            };
        }

        if !matches!(
            current_snapshot.workspace_status(),
            PlanningRuntimeWorkspaceStatus::ReadyNoTask
                | PlanningRuntimeWorkspaceStatus::ReadyWithTask
        ) {
            return BuiltinNextTaskRefreshOutcome {
                runtime_snapshot: current_snapshot,
            };
        }

        let Some(latest_main_reply) = conversation
            .latest_agent_message_text()
            .map(str::trim)
            .filter(|message: &&str| !message.is_empty())
        else {
            return BuiltinNextTaskRefreshOutcome {
                runtime_snapshot: current_snapshot,
            };
        };

        let review_prompt_markdown = match current_snapshot.workspace_status() {
            PlanningRuntimeWorkspaceStatus::ReadyWithTask => None,
            PlanningRuntimeWorkspaceStatus::ReadyNoTask => {
                let review_context = match self
                    .planning
                    .workspace
                    .load_queue_idle_review_context(&request.workspace_directory)
                {
                    Ok(context) => context,
                    Err(_) => {
                        return BuiltinNextTaskRefreshOutcome {
                            runtime_snapshot: current_snapshot,
                        };
                    }
                };
                match review_context.policy {
                    QueueIdlePolicy::Stop => {
                        return BuiltinNextTaskRefreshOutcome {
                            runtime_snapshot: current_snapshot,
                        };
                    }
                    QueueIdlePolicy::ReviewAndEnqueue => {
                        let Some(prompt_markdown) = review_context.prompt_markdown else {
                            return BuiltinNextTaskRefreshOutcome {
                                runtime_snapshot: current_snapshot,
                            };
                        };
                        Some(prompt_markdown)
                    }
                }
            }
            PlanningRuntimeWorkspaceStatus::Uninitialized
            | PlanningRuntimeWorkspaceStatus::Invalid => {
                return BuiltinNextTaskRefreshOutcome {
                    runtime_snapshot: current_snapshot,
                };
            }
        };
        let mode = match current_snapshot.workspace_status() {
            PlanningRuntimeWorkspaceStatus::ReadyWithTask => {
                PlanningQueueRefreshMode::FromLatestReply
            }
            PlanningRuntimeWorkspaceStatus::ReadyNoTask => {
                let prompt_markdown = review_prompt_markdown
                    .as_deref()
                    .expect("queue-idle review prompt should exist for review_and_enqueue");
                PlanningQueueRefreshMode::DeriveNextTaskWhenQueueIdle {
                    queue_idle_prompt_markdown: prompt_markdown,
                }
            }
            PlanningRuntimeWorkspaceStatus::Uninitialized
            | PlanningRuntimeWorkspaceStatus::Invalid => {
                unreachable!("non-ready planning states return before queue refresh mode is built")
            }
        };

        let worker_request = PlanningQueueRefreshRequest {
            workspace_directory: &request.workspace_directory,
            root_turn_id: &request.queued_from_turn_id,
            latest_user_message: conversation.latest_user_message_text(),
            latest_main_reply,
            previous_handoff_task: conversation.last_planning_task_handoff(),
            mode: mode.clone(),
        };
        let worker_prompt = self
            .planning
            .worker
            .render_refresh_queue_prompt(&worker_request);
        self.record_planner_worker_running(
            PlannerWorkerStatus::RefreshRunning,
            match mode {
                PlanningQueueRefreshMode::FromLatestReply => "refresh",
                PlanningQueueRefreshMode::DeriveNextTaskWhenQueueIdle { .. } => "active-derive",
            },
            worker_prompt,
        );
        let worker_outcome = self
            .planning
            .worker
            .refresh_queue_from_reply(worker_request);

        let outcome = match worker_outcome {
            Ok(outcome) => outcome,
            Err(error) => {
                let detail = match mode {
                    PlanningQueueRefreshMode::FromLatestReply => {
                        format!("planner refresh failed: {error}")
                    }
                    PlanningQueueRefreshMode::DeriveNextTaskWhenQueueIdle { .. } => {
                        format!("planner queue active-derivation failed: {error}")
                    }
                };
                let invalid_snapshot =
                    PlanningRuntimeSnapshot::invalid(PLANNER_REFRESH_FAILURE_BLOCK_REASON);
                self.record_planner_worker_failure(
                    PlannerWorkerStatus::RefreshFailed,
                    &detail,
                    &invalid_snapshot,
                );
                return BuiltinNextTaskRefreshOutcome {
                    runtime_snapshot: invalid_snapshot,
                };
            }
        };

        self.record_planner_worker_outcome(PlannerWorkerStatus::RefreshSucceeded, &outcome);
        let mut runtime_snapshot = outcome.runtime_snapshot.clone();

        if let Some(repair_request) = outcome.repair_request.as_ref() {
            let repair_outcome = self.run_hidden_planning_repairs(
                &request.workspace_directory,
                &request.queued_from_turn_id,
                repair_request,
            );
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
                .planning
                .worker
                .promote_top_proposal_to_ready_if_needed(PlanningProposalPromotionRequest {
                    workspace_directory: &request.workspace_directory,
                    root_turn_id: &request.queued_from_turn_id,
                });

            match promotion_outcome {
                Ok(promotion_outcome) => {
                    runtime_snapshot = promotion_outcome.runtime_snapshot;
                    self.planner_worker_panel_state.last_queue_summary =
                        planner_queue_summary(&runtime_snapshot);
                    self.planner_worker_panel_state.last_host_detail =
                        promotion_outcome.promoted_task_title.map(|title| {
                            format!(
                                "host promoted top follow-up proposal into the executable queue: {title}"
                            )
                        });
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
                    };
                }
            }
        }

        if !runtime_snapshot.has_actionable_queue_head()
            && !runtime_snapshot.has_proposal_candidates()
            && matches!(
                mode,
                PlanningQueueRefreshMode::DeriveNextTaskWhenQueueIdle { .. }
            )
        {
            self.planner_worker_panel_state.last_host_detail = Some(
                "planner derived no justified follow-up task from the latest request and reply"
                    .to_string(),
            );
        }

        if let Some(detail) =
            repeated_queue_head_detail(conversation.last_planning_task_handoff(), &runtime_snapshot)
        {
            self.planner_worker_panel_state.status = PlannerWorkerStatus::RefreshFailed;
            self.planner_worker_panel_state.last_host_detail = Some(detail.clone());
            runtime_snapshot = runtime_snapshot.with_auto_followup_pause_reason(detail.clone());
        }

        BuiltinNextTaskRefreshOutcome { runtime_snapshot }
    }

    fn auto_followup_action_from_snapshot(
        &self,
        conversation: &ConversationViewModel,
        request: &PostTurnEvaluationRequest,
        planning_runtime_snapshot: &PlanningRuntimeSnapshot,
    ) -> ConversationPostTurnAction {
        if planning_runtime_snapshot.workspace_present()
            && !planning_runtime_snapshot.plan_enabled()
        {
            return ConversationPostTurnAction::SkipAutoFollowup {
                reason: AutoFollowupSkipReason::PlanningDisabled,
            };
        }

        if planning_runtime_snapshot.workspace_status()
            == PlanningRuntimeWorkspaceStatus::ReadyNoTask
            && planning_runtime_snapshot.queue_idle_policy() == QueueIdlePolicy::Stop
        {
            return ConversationPostTurnAction::SkipAutoFollowup {
                reason: AutoFollowupSkipReason::PlanningQueueIdlePolicyStop,
            };
        }

        match conversation
            .decide_auto_followup_with_snapshot(&self.planning.runtime, planning_runtime_snapshot)
        {
            AutoFollowupDecision::QueuePrompt(queued_prompt) => {
                ConversationPostTurnAction::QueueAutoPrompt(Box::new(QueuedAutoPrompt {
                    prompt: queued_prompt.prompt,
                    queued_from_turn_id: request.queued_from_turn_id.clone(),
                    mode_label: conversation.auto_follow_state.mode_label().to_string(),
                    transcript_text: queued_prompt.transcript_text,
                    handoff_task: queued_prompt.handoff_task,
                }))
            }
            AutoFollowupDecision::Skip(reason) => {
                ConversationPostTurnAction::SkipAutoFollowup { reason }
            }
        }
    }

    fn record_planner_worker_running(
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
        self.planner_worker_panel_state.last_notice_detail =
            planner_notice_detail(&outcome.notices);
        self.planner_worker_panel_state.last_response = outcome.worker_response.clone();
        self.planner_worker_panel_state.last_host_detail = None;
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
        self.planner_worker_panel_state.last_notice_detail = None;
        self.planner_worker_panel_state.last_response = None;
        self.planner_worker_panel_state.last_host_detail = None;
    }
}

impl NativeTuiApp {
    pub(super) fn execute_post_turn_evaluation(&mut self, request: PostTurnEvaluationRequest) {
        let Some(conversation) = self.ready_conversation_snapshot() else {
            return;
        };

        self.mark_post_turn_evaluation_running(&conversation, &request);
        let executor = PostTurnEvaluationExecutor::new(
            self.planning.clone(),
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
            ConversationState::Ready(conversation) => Some(conversation.as_ref().clone()),
            ConversationState::Loading | ConversationState::Failed(_) => None,
        }
    }

    fn mark_post_turn_evaluation_running(
        &mut self,
        conversation: &ConversationViewModel,
        request: &PostTurnEvaluationRequest,
    ) {
        if !conversation.auto_follow_state.enabled {
            return;
        }

        if request
            .changed_planning_file_paths
            .iter()
            .any(|path| PlanningExecutionSnapshot::captures_path(path))
        {
            self.planner_worker_panel_state.status = PlannerWorkerStatus::RepairRunning;
        } else if conversation.planning_runtime_snapshot.workspace_status()
            == PlanningRuntimeWorkspaceStatus::ReadyNoTask
            && conversation.planning_runtime_snapshot.queue_idle_policy() == QueueIdlePolicy::Stop
        {
        } else {
            self.planner_worker_panel_state.status = PlannerWorkerStatus::RefreshRunning;
        }
    }
}

fn planner_queue_summary(snapshot: &PlanningRuntimeSnapshot) -> Option<String> {
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

fn repeated_queue_head_detail(
    previous_handoff: Option<&PlanningTaskHandoff>,
    snapshot: &PlanningRuntimeSnapshot,
) -> Option<String> {
    let previous_handoff = previous_handoff?;
    let queue_head = snapshot.queue_head()?;
    if queue_head.task_id.trim() != previous_handoff.task_id.trim() {
        return None;
    }

    let unchanged = queue_head.task_title.trim() == previous_handoff.task_title.trim()
        && queue_head.direction_id.trim() == previous_handoff.direction_id.trim()
        && queue_head.combined_priority == previous_handoff.combined_priority
        && queue_head.updated_at.trim() == previous_handoff.updated_at.trim()
        && queue_head.status.label() == previous_handoff.status_label;
    if !unchanged {
        return None;
    }

    Some(format!(
        "automation paused because the queue still points to the previous task: {}",
        previous_handoff.task_title
    ))
}

fn blocked_reconciliation_result(message: String) -> PlanningReconciliationResult {
    PlanningReconciliationResult {
        notices: vec![message.clone()],
        auto_followup_block_reason: Some(message),
        ..PlanningReconciliationResult::default()
    }
}
