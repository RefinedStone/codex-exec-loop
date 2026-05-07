#[cfg(not(test))]
use super::super::app_runtime::BackgroundMessage;
#[cfg(test)]
use super::super::conversation_runtime::ConversationRuntimeEvent;
use super::super::conversation_runtime::{
    ConversationPostTurnAction, ConversationPostTurnEvaluation, QueuedAutoPrompt,
};
use super::super::{
    ActiveTurnExecutionSnapshotCapture, ActiveTurnExecutionSnapshotState, AutoFollowupDecision,
    AutoFollowupSkipReason, ConversationState, ConversationViewModel, NativeTuiApp,
    PlannerWorkerPanelState, PlannerWorkerStatus,
};
use crate::application::service::parallel_mode::turn::ParallelModeTurnService;
use crate::application::service::planning::PlanningProposalPromotionRequest;
use crate::application::service::planning::PlanningServices;
use crate::application::service::planning::{
    PlanningExecutionSnapshot, PlanningReconciliationResult,
};
use crate::application::service::planning::{
    PlanningQueueRefreshMode, PlanningQueueRefreshRequest,
};
use crate::application::service::planning::{
    PlanningRuntimeSnapshot, PlanningRuntimeWorkspaceStatus,
};
use crate::diagnostics::event_log;
use crate::domain::planning::QueueIdlePolicy;
use serde_json::json;
const MAX_PLANNING_REPAIR_ATTEMPTS: usize = 2;
#[cfg(not(test))]
const POST_TURN_EVALUATION_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(600);
const PLANNER_REFRESH_FAILURE_BLOCK_REASON: &str =
    "planner refresh failed; auto follow-up stays paused until the next accepted planner refresh";
const OFFICIAL_COMPLETION_REFRESH_FAILURE_BLOCK_REASON: &str =
    "official completion refresh failed; the leased slot stays reserved until planning is repaired";
#[path = "post_turn_execution/logging.rs"]
mod logging;
#[path = "post_turn_execution/official_completion.rs"]
mod official_completion;
#[path = "post_turn_execution/planner_worker_panel.rs"]
mod planner_worker_panel;
#[path = "post_turn_execution/queue_head_detail.rs"]
mod queue_head_detail;
#[path = "post_turn_execution/repair.rs"]
mod repair;
use self::planner_worker_panel::planner_queue_summary;
use self::queue_head_detail::repeated_queue_head_detail;
use logging::{
    planner_refresh_skipped_detail, planning_refresh_mode_label, post_turn_action_decision,
    post_turn_action_log_detail, post_turn_event_detail,
};

// Post-turn evaluation is the handoff between a completed Codex turn and the
// planning/parallel-mode automation that may schedule the next prompt. The
// executor owns a cloned service set so production can run it off the UI thread
// while tests run the same sequence synchronously.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct PostTurnEvaluationRequest {
    pub workspace_directory: String,
    pub completed_turn_id: String,
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
struct OfficialCompletionRefreshOutcome {
    runtime_snapshot: PlanningRuntimeSnapshot,
    runtime_notices: Vec<String>,
}
#[derive(Debug, Clone)]
#[cfg_attr(test, allow(dead_code))]
struct PostTurnEvaluationExecution {
    thread_id: String,
    completed_turn_id: String,
    evaluation: ConversationPostTurnEvaluation,
    planner_worker_panel_state: PlannerWorkerPanelState,
}
#[derive(Clone)]
struct PostTurnEvaluationExecutor {
    planning: PlanningServices,
    parallel_mode_turn_service: ParallelModeTurnService,
    active_turn_execution_snapshot_capture: Option<ActiveTurnExecutionSnapshotCapture>,
    planner_worker_panel_state: PlannerWorkerPanelState,
}
impl PostTurnEvaluationExecutor {
    fn new(
        planning: PlanningServices,
        parallel_mode_turn_service: ParallelModeTurnService,
        active_turn_execution_snapshot_capture: Option<ActiveTurnExecutionSnapshotCapture>,
        planner_worker_panel_state: PlannerWorkerPanelState,
    ) -> Self {
        Self {
            planning,
            parallel_mode_turn_service,
            active_turn_execution_snapshot_capture,
            planner_worker_panel_state,
        }
    }

    // The execution order is deliberate: protect planning files first, repair
    // only when automation can act on the result, finish official parallel
    // completions before built-in follow-up refreshes, then derive the action
    // from the final runtime snapshot.
    #[tracing::instrument(level = "trace", skip(self, conversation))]
    fn run(
        mut self,
        conversation: &ConversationViewModel,
        request: &PostTurnEvaluationRequest,
    ) -> PostTurnEvaluationExecution {
        let planning_workspace_directory = planning_workspace_directory(conversation, request);
        event_log::emit_lazy("post_turn_evaluation_started", || {
            post_turn_event_detail(
                conversation,
                request,
                "post_turn",
                "started",
                Some("evaluate"),
                Some(&conversation.planning_runtime_snapshot),
                [
                    (
                        "planning_workspace_directory",
                        json!(planning_workspace_directory),
                    ),
                    (
                        "changed_planning_file_count",
                        json!(request.changed_planning_file_paths.len()),
                    ),
                    (
                        "post_turn_continuation_paused",
                        json!(
                            conversation
                                .auto_follow_state
                                .post_turn_continuation_paused()
                        ),
                    ),
                ],
            )
        });
        let reconciliation_result = self.reconcile_planning_after_turn(request);
        let mut runtime_notices = reconciliation_result.notices.clone();
        let mut runtime_snapshot = self.runtime_snapshot_after_reconciliation(
            conversation,
            request,
            &reconciliation_result,
        );
        let continuation_enabled = !conversation
            .auto_follow_state
            .post_turn_continuation_paused();
        let official_completion_report =
            self.begin_official_completion_if_needed(conversation, request);
        if (continuation_enabled || official_completion_report.is_some())
            && let Some(repair_request) = reconciliation_result.repair_request.as_ref()
        {
            let repair_outcome = self.run_hidden_planning_repairs(
                &conversation.thread_id,
                &request.workspace_directory,
                &request.completed_turn_id,
                repair_request,
                conversation.last_planning_task_handoff(),
            );
            runtime_snapshot = repair_outcome.runtime_snapshot;
        }
        let handled_parallel_completion =
            if let Some(completion_report) = official_completion_report {
                let official_completion_outcome = self.run_official_completion_refresh(
                    conversation,
                    request,
                    planning_workspace_directory,
                    &runtime_snapshot,
                    &completion_report,
                );
                runtime_notices.extend(official_completion_outcome.runtime_notices.clone());
                runtime_snapshot = official_completion_outcome.runtime_snapshot;
                true
            } else {
                false
            };
        if !handled_parallel_completion && continuation_enabled {
            let refresh_outcome =
                self.run_builtin_next_task_refresh(conversation, request, runtime_snapshot.clone());
            runtime_snapshot = refresh_outcome.runtime_snapshot;
        }
        let action = if handled_parallel_completion {
            ConversationPostTurnAction::SkipAutoFollowup {
                reason: AutoFollowupSkipReason::ParallelSessionCompleted,
            }
        } else {
            self.auto_followup_action_from_snapshot(conversation, request, &runtime_snapshot)
        };
        event_log::emit_lazy("post_turn_evaluation_completed", || {
            post_turn_event_detail(
                conversation,
                request,
                "post_turn",
                "completed",
                Some(post_turn_action_decision(&action)),
                Some(&runtime_snapshot),
                [
                    (
                        "handled_parallel_completion",
                        json!(handled_parallel_completion),
                    ),
                    ("runtime_notices_count", json!(runtime_notices.len())),
                    ("action", post_turn_action_log_detail(&action)),
                ],
            )
        });

        PostTurnEvaluationExecution {
            thread_id: conversation.thread_id.clone(),
            completed_turn_id: request.completed_turn_id.clone(),
            evaluation: ConversationPostTurnEvaluation {
                runtime_snapshot,
                planning_repair_state: None,
                runtime_notices,
                action,
            },
            planner_worker_panel_state: self.planner_worker_panel_state,
        }
    }
    fn runtime_snapshot_after_reconciliation(
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

    // Reconciliation runs only for paths covered by the protected execution
    // snapshot. Without a matching snapshot, auto follow-up is blocked because
    // the host cannot safely restore planning authority after a turn mutation.
    #[tracing::instrument(level = "trace", skip(self))]
    fn reconcile_planning_after_turn(
        &mut self,
        request: &PostTurnEvaluationRequest,
    ) -> PlanningReconciliationResult {
        let requires_execution_snapshot = request
            .changed_planning_file_paths
            .iter()
            .any(|path| PlanningExecutionSnapshot::captures_path(path));
        if !requires_execution_snapshot {
            self.active_turn_execution_snapshot_capture = None;
            return PlanningReconciliationResult::default();
        }
        let Some(capture) = self.active_turn_execution_snapshot_capture.take() else {
            return blocked_reconciliation_result(
                "planning reconciliation could not restore protected planning files because the execution snapshot was unavailable"
                    .to_string(),
            );
        };
        if capture.workspace_directory != request.workspace_directory {
            return blocked_reconciliation_result(format!(
                "planning reconciliation ignored a stale execution snapshot captured for {} while the completed turn resolved in {}",
                capture.workspace_directory, request.workspace_directory
            ));
        }
        let execution_snapshot = match capture.snapshot {
            ActiveTurnExecutionSnapshotState::Ready(snapshot) => snapshot,
            ActiveTurnExecutionSnapshotState::CaptureFailed(error_message) => {
                return blocked_reconciliation_result(error_message);
            }
        };
        match self.planning.runtime.reconcile_after_turn(
            &request.workspace_directory,
            &request.completed_turn_id,
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

    // Built-in refresh is the normal auto-follow path after a main-session
    // reply. It skips non-ready workspaces, honors queue-idle policy, records
    // worker panel state, and promotes justified proposals into the executable
    // queue when no actionable head exists yet.
    #[tracing::instrument(level = "trace", skip(self, conversation))]
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
            event_log::emit_lazy("planner_refresh_skipped", || {
                planner_refresh_skipped_detail(
                    conversation,
                    request,
                    "planning_runtime_not_ready",
                    &current_snapshot,
                )
            });
            return BuiltinNextTaskRefreshOutcome {
                runtime_snapshot: current_snapshot,
            };
        }
        let Some(latest_main_reply) = conversation
            .latest_agent_message_text()
            .map(str::trim)
            .filter(|message: &&str| !message.is_empty())
        else {
            event_log::emit_lazy("planner_refresh_skipped", || {
                planner_refresh_skipped_detail(
                    conversation,
                    request,
                    "latest_main_reply_empty",
                    &current_snapshot,
                )
            });
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
                        event_log::emit_lazy("planner_refresh_skipped", || {
                            planner_refresh_skipped_detail(
                                conversation,
                                request,
                                "queue_idle_review_context_unavailable",
                                &current_snapshot,
                            )
                        });
                        return BuiltinNextTaskRefreshOutcome {
                            runtime_snapshot: current_snapshot,
                        };
                    }
                };
                match review_context.policy {
                    QueueIdlePolicy::Stop => {
                        event_log::emit_lazy("planner_refresh_skipped", || {
                            planner_refresh_skipped_detail(
                                conversation,
                                request,
                                "queue_idle_policy_stop",
                                &current_snapshot,
                            )
                        });
                        return BuiltinNextTaskRefreshOutcome {
                            runtime_snapshot: current_snapshot,
                        };
                    }
                    QueueIdlePolicy::ReviewAndEnqueue => {
                        let Some(prompt_markdown) = review_context.prompt_markdown else {
                            event_log::emit_lazy("planner_refresh_skipped", || {
                                planner_refresh_skipped_detail(
                                    conversation,
                                    request,
                                    "queue_idle_prompt_missing",
                                    &current_snapshot,
                                )
                            });
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
            parent_thread_id: Some(conversation.thread_id.as_str())
                .filter(|thread_id| !thread_id.trim().is_empty()),
            completed_turn_id: &request.completed_turn_id,
            latest_user_message: conversation.latest_user_message_text(),
            latest_main_reply,
            previous_handoff_task: conversation.last_planning_task_handoff(),
            mode: mode.clone(),
        };
        let worker_prompt = self
            .planning
            .worker
            .render_refresh_queue_prompt(&worker_request);
        event_log::emit_lazy("planner_refresh_started", || {
            post_turn_event_detail(
                conversation,
                request,
                "refresh",
                "started",
                Some("run_worker"),
                Some(&current_snapshot),
                [
                    ("mode", json!(planning_refresh_mode_label(&mode))),
                    (
                        "latest_main_reply_chars",
                        json!(latest_main_reply.chars().count()),
                    ),
                    (
                        "has_latest_user_message",
                        json!(worker_request.latest_user_message.is_some()),
                    ),
                    (
                        "has_previous_handoff",
                        json!(worker_request.previous_handoff_task.is_some()),
                    ),
                    ("worker_prompt_chars", json!(worker_prompt.chars().count())),
                ],
            )
        });
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
                event_log::emit_lazy("planner_refresh_failed", || {
                    post_turn_event_detail(
                        conversation,
                        request,
                        "refresh",
                        "worker_failed",
                        Some("block_auto_follow"),
                        Some(&invalid_snapshot),
                        [
                            ("mode", json!(planning_refresh_mode_label(&mode))),
                            ("error", json!(error.to_string())),
                            (
                                "invalid_reason",
                                json!(PLANNER_REFRESH_FAILURE_BLOCK_REASON),
                            ),
                        ],
                    )
                });
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
        event_log::emit_lazy("planner_refresh_succeeded", || {
            post_turn_event_detail(
                conversation,
                request,
                "refresh",
                "worker_succeeded",
                Some("apply_outcome"),
                Some(&outcome.runtime_snapshot),
                [
                    ("mode", json!(planning_refresh_mode_label(&mode))),
                    ("repair_requested", json!(outcome.repair_request.is_some())),
                    (
                        "task_authority_changed",
                        json!(outcome.task_authority_changed),
                    ),
                    ("notices_count", json!(outcome.notices.len())),
                    (
                        "has_worker_summary",
                        json!(outcome.worker_summary.is_some()),
                    ),
                    (
                        "has_rejected_summary",
                        json!(outcome.rejected_summary.is_some()),
                    ),
                ],
            )
        });
        let mut runtime_snapshot = outcome.runtime_snapshot.clone();
        if let Some(repair_request) = outcome.repair_request.as_ref() {
            let repair_outcome = self.run_hidden_planning_repairs(
                &conversation.thread_id,
                &request.workspace_directory,
                &request.completed_turn_id,
                repair_request,
                conversation.last_planning_task_handoff(),
            );
            runtime_snapshot = if repair_outcome.resolved {
                repair_outcome.runtime_snapshot
            } else {
                event_log::emit_lazy("planner_refresh_repair_unresolved", || {
                    post_turn_event_detail(
                        conversation,
                        request,
                        "repair",
                        "unresolved_after_refresh",
                        Some("block_auto_follow"),
                        Some(&repair_outcome.runtime_snapshot),
                        [
                            (
                                "repair_failure_summary",
                                json!(repair_request.failure_summary.as_str()),
                            ),
                            (
                                "invalid_reason",
                                json!(PLANNER_REFRESH_FAILURE_BLOCK_REASON),
                            ),
                        ],
                    )
                });
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
                });
            match promotion_outcome {
                Ok(promotion_outcome) => {
                    runtime_snapshot = promotion_outcome.runtime_snapshot;
                    event_log::emit_lazy("planner_proposal_promotion_completed", || {
                        post_turn_event_detail(
                            conversation,
                            request,
                            "proposal_promotion",
                            "completed",
                            promotion_outcome
                                .promoted_task_title
                                .as_ref()
                                .map(|_| "promoted")
                                .or(Some("no_promotable_proposal")),
                            Some(&runtime_snapshot),
                            [(
                                "promoted_task_title",
                                json!(promotion_outcome.promoted_task_title.as_deref()),
                            )],
                        )
                    });
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
                    event_log::emit_lazy("planner_proposal_promotion_failed", || {
                        post_turn_event_detail(
                            conversation,
                            request,
                            "proposal_promotion",
                            "failed",
                            Some("block_auto_follow"),
                            Some(&invalid_snapshot),
                            [
                                ("error", json!(error.to_string())),
                                (
                                    "invalid_reason",
                                    json!(PLANNER_REFRESH_FAILURE_BLOCK_REASON),
                                ),
                            ],
                        )
                    });
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
        if let Some(detail) = repeated_queue_head_detail(
            conversation.last_planning_task_handoff(),
            &conversation.planning_runtime_snapshot,
            &runtime_snapshot,
        ) {
            self.planner_worker_panel_state.status = PlannerWorkerStatus::RefreshFailed;
            self.planner_worker_panel_state.last_host_detail = Some(detail.clone());
            event_log::emit_lazy("planner_refresh_paused_repeated_queue_head", || {
                post_turn_event_detail(
                    conversation,
                    request,
                    "refresh",
                    "repeated_queue_head_guard",
                    Some("pause_auto_follow"),
                    Some(&runtime_snapshot),
                    [("pause_reason", json!(detail.as_str()))],
                )
            });
            runtime_snapshot = runtime_snapshot.with_auto_followup_pause_reason(detail.clone());
        }

        BuiltinNextTaskRefreshOutcome { runtime_snapshot }
    }

    // The final action is always derived from the latest snapshot. Explicit
    // pause states and queue-idle stop policy win before the conversation model
    // is allowed to enqueue another prompt.
    #[tracing::instrument(level = "trace", skip(self))]
    fn auto_followup_action_from_snapshot(
        &self,
        conversation: &ConversationViewModel,
        request: &PostTurnEvaluationRequest,
        runtime_snapshot: &PlanningRuntimeSnapshot,
    ) -> ConversationPostTurnAction {
        if conversation
            .auto_follow_state
            .post_turn_continuation_paused()
        {
            event_log::emit_lazy("auto_follow_decision", || {
                post_turn_event_detail(
                    conversation,
                    request,
                    "auto_follow",
                    "decision",
                    Some("skip"),
                    Some(runtime_snapshot),
                    [("reason", json!("PostTurnContinuationPaused"))],
                )
            });
            return ConversationPostTurnAction::SkipAutoFollowup {
                reason: AutoFollowupSkipReason::PostTurnContinuationPaused,
            };
        }
        if runtime_snapshot.queue_is_drained() {
            event_log::emit_lazy("auto_follow_decision", || {
                post_turn_event_detail(
                    conversation,
                    request,
                    "auto_follow",
                    "decision",
                    Some("skip"),
                    Some(runtime_snapshot),
                    [("reason", json!("PlanningQueueDrained"))],
                )
            });
            return ConversationPostTurnAction::SkipAutoFollowup {
                reason: AutoFollowupSkipReason::PlanningQueueDrained,
            };
        }
        if runtime_snapshot.workspace_status() == PlanningRuntimeWorkspaceStatus::ReadyNoTask
            && runtime_snapshot.queue_idle_policy() == QueueIdlePolicy::Stop
        {
            event_log::emit_lazy("auto_follow_decision", || {
                post_turn_event_detail(
                    conversation,
                    request,
                    "auto_follow",
                    "decision",
                    Some("skip"),
                    Some(runtime_snapshot),
                    [("reason", json!("PlanningQueueIdlePolicyStop"))],
                )
            });
            return ConversationPostTurnAction::SkipAutoFollowup {
                reason: AutoFollowupSkipReason::PlanningQueueIdlePolicyStop,
            };
        }
        match conversation
            .decide_auto_followup_with_snapshot(&self.planning.runtime, runtime_snapshot)
        {
            AutoFollowupDecision::QueuePrompt(queued_prompt) => {
                event_log::emit_lazy("auto_follow_decision", || {
                    post_turn_event_detail(
                        conversation,
                        request,
                        "auto_follow",
                        "decision",
                        Some("queue"),
                        Some(runtime_snapshot),
                        [
                            (
                                "mode_label",
                                json!(conversation.auto_follow_state.mode_label()),
                            ),
                            ("prompt_chars", json!(queued_prompt.prompt.chars().count())),
                            (
                                "transcript_text_chars",
                                json!(queued_prompt.transcript_text.chars().count()),
                            ),
                            (
                                "handoff_task_id",
                                json!(
                                    queued_prompt
                                        .handoff_task
                                        .as_ref()
                                        .map(|task| task.task_id.as_str())
                                ),
                            ),
                        ],
                    )
                });
                ConversationPostTurnAction::QueueAutoPrompt(Box::new(QueuedAutoPrompt {
                    prompt: queued_prompt.prompt,
                    completed_turn_id: request.completed_turn_id.clone(),
                    mode_label: conversation.auto_follow_state.mode_label().to_string(),
                    transcript_text: queued_prompt.transcript_text,
                    handoff_task: queued_prompt.handoff_task,
                }))
            }
            AutoFollowupDecision::Skip(reason) => {
                event_log::emit_lazy("auto_follow_decision", || {
                    post_turn_event_detail(
                        conversation,
                        request,
                        "auto_follow",
                        "decision",
                        Some("skip"),
                        Some(runtime_snapshot),
                        [("reason", json!(format!("{:?}", reason)))],
                    )
                });
                ConversationPostTurnAction::SkipAutoFollowup { reason }
            }
        }
    }
}
impl NativeTuiApp {
    // Production isolates post-turn planning work behind a timeout so a stalled
    // planner worker cannot strand the TUI. Tests execute synchronously to keep
    // assertions deterministic while still exercising the same executor.
    #[tracing::instrument(level = "trace", skip(self))]
    pub(super) fn execute_post_turn_evaluation(&mut self, request: PostTurnEvaluationRequest) {
        let Some(conversation) = self.ready_conversation_snapshot() else {
            return;
        };
        self.mark_post_turn_evaluation_running(&conversation, &request);
        let executor = PostTurnEvaluationExecutor::new(
            self.planning.clone(),
            self.parallel_mode_turn_service(),
            self.active_turn_execution_snapshot_capture.take(),
            self.planner_worker_panel_state.clone(),
        );
        #[cfg(test)]
        {
            let execution = executor.run(&conversation, &request);
            self.planner_worker_panel_state = execution.planner_worker_panel_state;
            self.invalidate_parallel_mode_supervisor_snapshot();
            self.dispatch_conversation_runtime(ConversationRuntimeEvent::PostTurnEvaluated {
                evaluation: Box::new(execution.evaluation),
            });
        }
        #[cfg(not(test))]
        {
            let tx = self.tx.clone();
            std::thread::spawn(move || {
                let (execution_tx, execution_rx) = std::sync::mpsc::channel();
                let fallback_conversation = conversation.clone();
                let fallback_request = request.clone();
                std::thread::spawn(move || {
                    let execution = executor.run(&conversation, &request);
                    let _ = execution_tx.send(execution);
                });
                let execution = execution_rx
                    .recv_timeout(POST_TURN_EVALUATION_TIMEOUT)
                    .unwrap_or_else(|_| {
                        post_turn_evaluation_timeout_execution(
                            &fallback_conversation,
                            &fallback_request,
                        )
                    });
                let _ = tx.send(BackgroundMessage::PostTurnEvaluated {
                    thread_id: execution.thread_id,
                    completed_turn_id: execution.completed_turn_id,
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

    // The panel enters a running state only when automation can make progress.
    // Paused continuations and queue-idle stop policy keep the previous operator
    // context visible instead of flashing a worker state that will not run.
    fn mark_post_turn_evaluation_running(
        &mut self,
        conversation: &ConversationViewModel,
        request: &PostTurnEvaluationRequest,
    ) {
        if conversation
            .auto_follow_state
            .post_turn_continuation_paused()
        {
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
#[cfg(not(test))]
// Timeout fallback reports a failed refresh while returning control to the main
// session. The background worker may still finish later, but the UI receives a
// deterministic blocked evaluation for the completed turn.
fn post_turn_evaluation_timeout_execution(
    conversation: &ConversationViewModel,
    request: &PostTurnEvaluationRequest,
) -> PostTurnEvaluationExecution {
    let message = format!(
        "post-turn planner evaluation timed out after {} seconds",
        POST_TURN_EVALUATION_TIMEOUT.as_secs()
    );
    PostTurnEvaluationExecution {
        thread_id: conversation.thread_id.clone(),
        completed_turn_id: request.completed_turn_id.clone(),
        evaluation: ConversationPostTurnEvaluation {
            runtime_snapshot: PlanningRuntimeSnapshot::invalid(message.clone()),
            planning_repair_state: None,
            runtime_notices: vec![message.clone()],
            action: ConversationPostTurnAction::SkipAutoFollowup {
                reason: AutoFollowupSkipReason::PostTurnEvaluationTimedOut,
            },
        },
        planner_worker_panel_state: PlannerWorkerPanelState {
            status: PlannerWorkerStatus::RefreshFailed,
            last_operation_label: Some("post-turn".to_string()),
            last_summary: Some(message),
            last_rejected_summary: None,
            last_queue_summary: Some("planning refresh timed out".to_string()),
            last_notice_detail: None,
            last_prompt: None,
            last_response: None,
            last_host_detail: Some(
                "host recovered the main session from a stalled post-turn planner evaluation"
                    .to_string(),
            ),
        },
    }
}

// Draft workspace overrides let resumed or prepared turns evaluate planning in
// the workspace selected by the conversation, falling back to the request path
// when no draft override is active.
fn planning_workspace_directory<'a>(
    conversation: &'a ConversationViewModel,
    request: &'a PostTurnEvaluationRequest,
) -> &'a str {
    let draft_workspace_directory = conversation.draft_workspace_directory.trim();
    if draft_workspace_directory.is_empty() {
        request.workspace_directory.as_str()
    } else {
        draft_workspace_directory
    }
}
fn blocked_reconciliation_result(message: String) -> PlanningReconciliationResult {
    PlanningReconciliationResult {
        notices: vec![message.clone()],
        auto_followup_block_reason: Some(message),
        ..PlanningReconciliationResult::default()
    }
}
