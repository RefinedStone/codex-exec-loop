#[cfg(not(test))]
use super::super::app_runtime::BackgroundMessage;
use super::super::app_runtime::NativeTuiPlanningHandle;
#[cfg(test)]
use super::super::conversation_runtime::ConversationRuntimeEvent;
use super::super::conversation_runtime::{
    PostTurnContinuationAction, PostTurnEvaluationOutcome, PostTurnEvaluationProvenance,
    PostTurnQueuedPrompt,
};
use super::super::{
    AutoFollowSkipReason, ConversationState, ConversationViewModel, NativeTuiApp,
    PlanningWorkerPanelState, PlanningWorkerStatus,
};
use crate::application::service::parallel_mode::turn::ParallelModeTurnService;
use crate::application::service::planning::PlanningRuntimeSnapshot;
use crate::application::service::planning::PlanningTurnExecutionSnapshotCapture;
use crate::application::service::planning::{
    PLANNING_WORKER_REFRESH_FAILURE_BLOCK_REASON, PlanningPostTurnAutoFollowDecision,
    PlanningPostTurnAutoFollowRequest, PlanningPostTurnAutoFollowSkipReason,
    PlanningPostTurnQueueRefreshFinalizationEvent, PlanningPostTurnQueueRefreshFinalizationRequest,
    PlanningPostTurnQueueRefreshPreparation, PlanningPostTurnQueueRefreshPreparationRequest,
    PlanningPostTurnReconciliationRequest, PlanningPostTurnWorkerPanelStartRequest,
    PlanningPostTurnWorkerPanelStartState, PlanningTaskHandoff,
};
use crate::application::service::post_turn_decision::{
    PostTurnAutoFollowStopReason, PostTurnDecision as ApplicationPostTurnDecision,
    decide_parallel_official_completion_post_turn,
};
use crate::diagnostics::event_log;
use crate::domain::operator_alert::OperatorAlert;
#[cfg(test)]
use crate::domain::parallel_mode::ParallelModePostTurnQueueSignal;
use serde_json::json;
#[cfg(not(test))]
const POST_TURN_EVALUATION_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(600);
#[path = "post_turn_execution/logging.rs"]
mod logging;
#[path = "post_turn_execution/official_completion.rs"]
mod official_completion;
#[path = "post_turn_execution/planning_worker_panel.rs"]
mod planning_worker_panel;
#[path = "post_turn_execution/repair.rs"]
mod repair;
use self::planning_worker_panel::planning_worker_queue_summary;
use logging::{
    PostTurnWorkerLogContext, planning_worker_refresh_skipped_detail, post_turn_action_decision,
    post_turn_action_log_detail, post_turn_event_detail,
};

// Post-turn evaluation is the handoff between a completed Codex turn and the
// planning/parallel-mode continuation that may schedule the next prompt. The
// executor owns a cloned service set so production can run it off the UI thread
// while tests run the same sequence synchronously.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct PostTurnEvaluationRequest {
    pub workspace_directory: String,
    pub completed_turn_id: String,
    pub changed_planning_file_paths: Vec<String>,
    pub execution_snapshot_capture: Option<PlanningTurnExecutionSnapshotCapture>,
}

#[derive(Debug, Clone)]
struct PostTurnEvaluationContext {
    thread_id: String,
    planning_workspace_directory: String,
    latest_user_message: Option<String>,
    latest_main_reply: Option<String>,
    previous_handoff_task: Option<PlanningTaskHandoff>,
    current_runtime_snapshot: PlanningRuntimeSnapshot,
    continuation_paused: bool,
    can_queue_next: bool,
    stop_keyword: String,
    stop_keyword_matched: bool,
    no_file_changes_stop_matched: bool,
    mode_label: String,
}

impl PostTurnEvaluationContext {
    fn from_conversation(
        conversation: &ConversationViewModel,
        request: &PostTurnEvaluationRequest,
    ) -> Self {
        let latest_main_reply = conversation.latest_agent_message_text().map(str::to_string);
        let stop_keyword_matched = latest_main_reply
            .as_deref()
            .map(|message| {
                conversation
                    .auto_follow_state
                    .stop_rules
                    .stop_keyword
                    .matches(message)
            })
            .unwrap_or(false);
        let no_file_changes_stop_matched = conversation
            .auto_follow_state
            .stop_rules
            .should_stop_on_no_file_changes(
                conversation
                    .turn_activity
                    .last_completed_file_change_count(),
            );

        Self {
            thread_id: conversation.thread_id.clone(),
            planning_workspace_directory: planning_workspace_directory(conversation, request)
                .to_string(),
            latest_user_message: conversation.latest_user_message_text().map(str::to_string),
            latest_main_reply,
            previous_handoff_task: conversation.last_planning_task_handoff().cloned(),
            current_runtime_snapshot: conversation.planning_runtime_snapshot.clone(),
            continuation_paused: conversation
                .auto_follow_state
                .post_turn_continuation_paused(),
            can_queue_next: conversation.auto_follow_state.can_queue_next(),
            stop_keyword: conversation
                .auto_follow_state
                .stop_keyword_value()
                .to_string(),
            stop_keyword_matched,
            no_file_changes_stop_matched,
            mode_label: conversation.auto_follow_state.mode_label().to_string(),
        }
    }

    fn log_context<'a>(
        &'a self,
        request: &'a PostTurnEvaluationRequest,
    ) -> PostTurnWorkerLogContext<'a> {
        PostTurnWorkerLogContext::new(
            self.thread_id.as_str(),
            request.completed_turn_id.as_str(),
            request.workspace_directory.as_str(),
        )
    }

    fn previous_handoff_task(&self) -> Option<&PlanningTaskHandoff> {
        self.previous_handoff_task.as_ref()
    }
}
#[derive(Debug, Clone)]
struct HiddenPlanningRepairOutcome {
    runtime_snapshot: PlanningRuntimeSnapshot,
    resolved: bool,
}
#[derive(Debug, Clone)]
struct PlanningQueueRefreshOutcome {
    runtime_snapshot: PlanningRuntimeSnapshot,
}
#[derive(Debug, Clone)]
struct OfficialCompletionRefreshOutcome {
    runtime_snapshot: PlanningRuntimeSnapshot,
    runtime_notices: Vec<String>,
}
#[derive(Debug, Clone)]
struct TuiPostTurnDecision {
    action: PostTurnContinuationAction,
    provenance: PostTurnEvaluationProvenance,
    operator_alerts: Vec<OperatorAlert>,
}
impl TuiPostTurnDecision {
    fn from_action(completed_turn_id: String, action: PostTurnContinuationAction) -> Self {
        let operator_alerts = operator_alerts_for_action(&action);
        Self {
            action,
            provenance: PostTurnEvaluationProvenance::new(completed_turn_id),
            operator_alerts,
        }
    }

    fn from_action_with_provenance(
        action: PostTurnContinuationAction,
        provenance: PostTurnEvaluationProvenance,
    ) -> Self {
        let operator_alerts = operator_alerts_for_action(&action);
        Self {
            action,
            provenance,
            operator_alerts,
        }
    }

    fn from_application_decision(
        completed_turn_id: String,
        decision: ApplicationPostTurnDecision,
    ) -> Self {
        Self {
            action: PostTurnContinuationAction::SkipAutoFollow {
                reason: auto_follow_skip_reason_from_post_turn(decision.auto_follow_stop_reason),
            },
            provenance: PostTurnEvaluationProvenance::new(completed_turn_id)
                .with_parallel_queue_signal(decision.parallel_queue_signal),
            operator_alerts: decision.operator_alerts,
        }
    }
}
#[derive(Debug, Clone)]
#[cfg_attr(test, allow(dead_code))]
struct PostTurnEvaluationExecution {
    thread_id: String,
    completed_turn_id: String,
    evaluation: PostTurnEvaluationOutcome,
    planning_worker_panel_state: PlanningWorkerPanelState,
}
#[derive(Clone)]
struct PostTurnEvaluationExecutor {
    planning_feature: NativeTuiPlanningHandle,
    parallel_mode_turn_service: ParallelModeTurnService,
    planning_worker_panel_state: PlanningWorkerPanelState,
}
impl PostTurnEvaluationExecutor {
    fn new(
        planning_feature: NativeTuiPlanningHandle,
        parallel_mode_turn_service: ParallelModeTurnService,
        planning_worker_panel_state: PlanningWorkerPanelState,
    ) -> Self {
        Self {
            planning_feature,
            parallel_mode_turn_service,
            planning_worker_panel_state,
        }
    }

    // The execution order is deliberate: protect planning files first, repair
    // only when continuation can act on the result, finish official parallel
    // completions before planning queue refreshes, then derive the action
    // from the final runtime snapshot.
    #[tracing::instrument(level = "trace", skip(self, context))]
    fn run(
        mut self,
        context: &PostTurnEvaluationContext,
        request: &PostTurnEvaluationRequest,
    ) -> PostTurnEvaluationExecution {
        let planning_workspace_directory = context.planning_workspace_directory.as_str();
        event_log::emit_lazy("post_turn_evaluation_started", || {
            post_turn_event_detail(
                context.log_context(request),
                "post_turn",
                "started",
                Some("evaluate"),
                Some(&context.current_runtime_snapshot),
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
                        json!(context.continuation_paused),
                    ),
                ],
            )
        });
        let reconciliation_outcome = self.planning_feature.runtime().reconcile_post_turn(
            PlanningPostTurnReconciliationRequest {
                workspace_directory: &request.workspace_directory,
                completed_turn_id: &request.completed_turn_id,
                changed_planning_file_paths: &request.changed_planning_file_paths,
                execution_snapshot_capture: request.execution_snapshot_capture.as_ref(),
                current_runtime_snapshot: &context.current_runtime_snapshot,
            },
        );
        let reconciliation_result = reconciliation_outcome.reconciliation_result;
        let mut runtime_notices = reconciliation_result.notices.clone();
        let mut runtime_snapshot = reconciliation_outcome.runtime_snapshot;
        let continuation_enabled = !context.continuation_paused;
        let official_completion_report = self.begin_official_completion_if_needed(context, request);
        if (continuation_enabled || official_completion_report.is_some())
            && let Some(repair_request) = reconciliation_result.repair_request.as_ref()
        {
            let repair_outcome = self.run_hidden_planning_repairs(
                context.thread_id.as_str(),
                &request.workspace_directory,
                &request.completed_turn_id,
                repair_request,
                context.previous_handoff_task(),
            );
            runtime_snapshot = repair_outcome.runtime_snapshot;
        }
        let handled_parallel_completion =
            if let Some(completion_report) = official_completion_report {
                let official_completion_outcome = self.run_official_completion_refresh(
                    context,
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
                self.run_planning_queue_refresh(context, request, runtime_snapshot.clone());
            runtime_snapshot = refresh_outcome.runtime_snapshot;
        }
        let post_turn_decision = if handled_parallel_completion {
            TuiPostTurnDecision::from_application_decision(
                request.completed_turn_id.clone(),
                decide_parallel_official_completion_post_turn(&runtime_snapshot),
            )
        } else {
            self.auto_follow_decision_from_snapshot(context, request, &runtime_snapshot)
        };
        event_log::emit_lazy("post_turn_evaluation_completed", || {
            post_turn_event_detail(
                context.log_context(request),
                "post_turn",
                "completed",
                Some(post_turn_action_decision(&post_turn_decision.action)),
                Some(&runtime_snapshot),
                [
                    (
                        "handled_parallel_completion",
                        json!(handled_parallel_completion),
                    ),
                    ("runtime_notices_count", json!(runtime_notices.len())),
                    (
                        "operator_alerts_count",
                        json!(post_turn_decision.operator_alerts.len()),
                    ),
                    (
                        "parallel_queue_signal",
                        json!(
                            post_turn_decision
                                .provenance
                                .parallel_queue_signal
                                .map(|signal| format!("{signal:?}"))
                        ),
                    ),
                    (
                        "action",
                        post_turn_action_log_detail(
                            &post_turn_decision.action,
                            &post_turn_decision.provenance,
                        ),
                    ),
                ],
            )
        });

        PostTurnEvaluationExecution {
            thread_id: context.thread_id.clone(),
            completed_turn_id: request.completed_turn_id.clone(),
            evaluation: PostTurnEvaluationOutcome {
                provenance: post_turn_decision.provenance,
                runtime_snapshot,
                planning_repair_state: None,
                runtime_notices,
                action: post_turn_decision.action,
                operator_alerts: post_turn_decision.operator_alerts,
            },
            planning_worker_panel_state: self.planning_worker_panel_state,
        }
    }
    // Planning queue refresh is the normal auto-follow path after a main-session
    // reply. It skips non-ready workspaces, honors queue-idle policy, records
    // worker panel state, and promotes justified proposals into the executable
    // queue when no actionable head exists yet.
    #[tracing::instrument(level = "trace", skip(self, context))]
    fn run_planning_queue_refresh(
        &mut self,
        context: &PostTurnEvaluationContext,
        request: &PostTurnEvaluationRequest,
        current_snapshot: PlanningRuntimeSnapshot,
    ) -> PlanningQueueRefreshOutcome {
        let preparation = self
            .planning_feature
            .worker()
            .prepare_post_turn_queue_refresh(PlanningPostTurnQueueRefreshPreparationRequest {
                workspace_directory: &request.workspace_directory,
                parent_thread_id: Some(context.thread_id.as_str())
                    .filter(|thread_id| !thread_id.trim().is_empty()),
                completed_turn_id: &request.completed_turn_id,
                latest_user_message: context.latest_user_message.as_deref(),
                latest_main_reply: context.latest_main_reply.as_deref(),
                previous_handoff_task: context.previous_handoff_task(),
                current_runtime_snapshot: &current_snapshot,
            });
        let prepared = match preparation {
            PlanningPostTurnQueueRefreshPreparation::Skipped(skipped) => {
                event_log::emit_lazy("planning_worker_refresh_skipped", || {
                    planning_worker_refresh_skipped_detail(
                        context.log_context(request),
                        skipped.reason.log_label(),
                        &skipped.runtime_snapshot,
                    )
                });
                return PlanningQueueRefreshOutcome {
                    runtime_snapshot: skipped.runtime_snapshot,
                };
            }
            PlanningPostTurnQueueRefreshPreparation::Ready(prepared) => prepared,
        };
        event_log::emit_lazy("planning_worker_refresh_started", || {
            post_turn_event_detail(
                context.log_context(request),
                "refresh",
                "started",
                Some("run_worker"),
                Some(&current_snapshot),
                [
                    ("mode", json!(prepared.mode_label())),
                    (
                        "latest_main_reply_chars",
                        json!(prepared.latest_main_reply_char_count()),
                    ),
                    (
                        "has_latest_user_message",
                        json!(prepared.has_latest_user_message()),
                    ),
                    (
                        "has_previous_handoff",
                        json!(prepared.has_previous_handoff()),
                    ),
                    (
                        "worker_prompt_chars",
                        json!(prepared.worker_prompt().chars().count()),
                    ),
                ],
            )
        });
        self.record_planning_worker_running(
            PlanningWorkerStatus::RefreshRunning,
            prepared.panel_operation_label(),
            prepared.worker_prompt().to_string(),
        );
        let worker_outcome = self
            .planning_feature
            .worker()
            .refresh_prepared_queue_from_reply(prepared.as_ref());
        let outcome = match worker_outcome {
            Ok(outcome) => outcome,
            Err(error) => {
                let detail = if prepared.is_queue_idle_derivation() {
                    format!("planning worker queue-idle derivation failed: {error}")
                } else {
                    format!("planning worker refresh failed: {error}")
                };
                let invalid_snapshot =
                    PlanningRuntimeSnapshot::invalid(PLANNING_WORKER_REFRESH_FAILURE_BLOCK_REASON);
                event_log::emit_lazy("planning_worker_refresh_failed", || {
                    post_turn_event_detail(
                        context.log_context(request),
                        "refresh",
                        "worker_failed",
                        Some("block_auto_follow"),
                        Some(&invalid_snapshot),
                        [
                            ("mode", json!(prepared.mode_label())),
                            ("error", json!(error.to_string())),
                            (
                                "invalid_reason",
                                json!(PLANNING_WORKER_REFRESH_FAILURE_BLOCK_REASON),
                            ),
                        ],
                    )
                });
                self.record_planning_worker_failure(
                    PlanningWorkerStatus::RefreshFailed,
                    &detail,
                    &invalid_snapshot,
                );
                return PlanningQueueRefreshOutcome {
                    runtime_snapshot: invalid_snapshot,
                };
            }
        };

        self.record_planning_worker_outcome(PlanningWorkerStatus::RefreshSucceeded, &outcome);
        event_log::emit_lazy("planning_worker_refresh_succeeded", || {
            post_turn_event_detail(
                context.log_context(request),
                "refresh",
                "worker_succeeded",
                Some("apply_outcome"),
                Some(&outcome.runtime_snapshot),
                [
                    ("mode", json!(prepared.mode_label())),
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
                context.thread_id.as_str(),
                &request.workspace_directory,
                &request.completed_turn_id,
                repair_request,
                context.previous_handoff_task(),
            );
            runtime_snapshot = if repair_outcome.resolved {
                repair_outcome.runtime_snapshot
            } else {
                event_log::emit_lazy("planning_worker_refresh_repair_unresolved", || {
                    post_turn_event_detail(
                        context.log_context(request),
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
                                json!(PLANNING_WORKER_REFRESH_FAILURE_BLOCK_REASON),
                            ),
                        ],
                    )
                });
                PlanningRuntimeSnapshot::invalid(PLANNING_WORKER_REFRESH_FAILURE_BLOCK_REASON)
            };
        }
        let finalization = self
            .planning_feature
            .worker()
            .finalize_post_turn_queue_refresh(PlanningPostTurnQueueRefreshFinalizationRequest {
                workspace_directory: &request.workspace_directory,
                previous_handoff_task: context.previous_handoff_task(),
                previous_runtime_snapshot: &context.current_runtime_snapshot,
                refreshed_runtime_snapshot: &runtime_snapshot,
                queue_idle_derivation: prepared.is_queue_idle_derivation(),
            });
        runtime_snapshot = finalization.runtime_snapshot;
        for event in finalization.events {
            match event {
                PlanningPostTurnQueueRefreshFinalizationEvent::ProposalPromotionCompleted {
                    outcome: promotion_outcome,
                } => {
                    event_log::emit_lazy("planning_worker_proposal_promotion_completed", || {
                        post_turn_event_detail(
                            context.log_context(request),
                            "proposal_promotion",
                            "completed",
                            promotion_outcome
                                .promoted_task_title
                                .as_ref()
                                .map(|_| "promoted")
                                .or(Some("no_promotable_proposal")),
                            Some(&promotion_outcome.runtime_snapshot),
                            [(
                                "promoted_task_title",
                                json!(promotion_outcome.promoted_task_title.as_deref()),
                            )],
                        )
                    });
                    self.planning_worker_panel_state.last_queue_summary =
                        planning_worker_queue_summary(&promotion_outcome.runtime_snapshot);
                    self.planning_worker_panel_state.last_host_detail =
                        promotion_outcome.promoted_task_title.map(|title| {
                            format!(
                                "host promoted top follow-up proposal into the executable queue: {title}"
                            )
                        });
                }
                PlanningPostTurnQueueRefreshFinalizationEvent::ProposalPromotionFailed {
                    detail,
                    runtime_snapshot: invalid_snapshot,
                } => {
                    event_log::emit_lazy("planning_worker_proposal_promotion_failed", || {
                        post_turn_event_detail(
                            context.log_context(request),
                            "proposal_promotion",
                            "failed",
                            Some("block_auto_follow"),
                            Some(&invalid_snapshot),
                            [
                                ("error", json!(detail.as_str())),
                                (
                                    "invalid_reason",
                                    json!(PLANNING_WORKER_REFRESH_FAILURE_BLOCK_REASON),
                                ),
                            ],
                        )
                    });
                    self.record_planning_worker_failure(
                        PlanningWorkerStatus::RefreshFailed,
                        &detail,
                        &invalid_snapshot,
                    );
                    return PlanningQueueRefreshOutcome {
                        runtime_snapshot: invalid_snapshot,
                    };
                }
                PlanningPostTurnQueueRefreshFinalizationEvent::QueueIdleDerivationEmpty {
                    detail,
                } => {
                    self.planning_worker_panel_state.last_host_detail = Some(detail);
                }
                PlanningPostTurnQueueRefreshFinalizationEvent::RepeatedQueueHead {
                    detail,
                    runtime_snapshot: guard_snapshot,
                } => {
                    self.planning_worker_panel_state.status = PlanningWorkerStatus::RefreshFailed;
                    self.planning_worker_panel_state.last_host_detail = Some(detail.clone());
                    event_log::emit_lazy(
                        "planning_worker_refresh_paused_repeated_queue_head",
                        || {
                            post_turn_event_detail(
                                context.log_context(request),
                                "refresh",
                                "repeated_queue_head_guard",
                                Some("pause_auto_follow"),
                                Some(&guard_snapshot),
                                [("pause_reason", json!(detail.as_str()))],
                            )
                        },
                    );
                }
            }
        }

        PlanningQueueRefreshOutcome { runtime_snapshot }
    }

    // The final action is always derived from the latest snapshot. Explicit
    // pause states and queue-idle stop policy win before the conversation model
    // is allowed to enqueue another prompt.
    #[tracing::instrument(level = "trace", skip(self, context))]
    fn auto_follow_decision_from_snapshot(
        &self,
        context: &PostTurnEvaluationContext,
        request: &PostTurnEvaluationRequest,
        runtime_snapshot: &PlanningRuntimeSnapshot,
    ) -> TuiPostTurnDecision {
        match self
            .planning_feature
            .runtime()
            .decide_post_turn_auto_follow(PlanningPostTurnAutoFollowRequest {
                continuation_paused: context.continuation_paused,
                can_queue_next: context.can_queue_next,
                latest_agent_message: context.latest_main_reply.as_deref(),
                stop_keyword: context.stop_keyword.as_str(),
                stop_keyword_matched: context.stop_keyword_matched,
                no_file_changes_stop_matched: context.no_file_changes_stop_matched,
                runtime_snapshot,
            }) {
            PlanningPostTurnAutoFollowDecision::QueuePrompt(queued_prompt) => {
                event_log::emit_lazy("auto_follow_decision", || {
                    post_turn_event_detail(
                        context.log_context(request),
                        "auto_follow",
                        "decision",
                        Some("queue"),
                        Some(runtime_snapshot),
                        [
                            ("mode_label", json!(context.mode_label.as_str())),
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
                TuiPostTurnDecision::from_action_with_provenance(
                    PostTurnContinuationAction::QueueAutoPrompt(Box::new(PostTurnQueuedPrompt {
                        prompt: queued_prompt.prompt,
                        mode_label: context.mode_label.clone(),
                        transcript_text: queued_prompt.transcript_text,
                    })),
                    PostTurnEvaluationProvenance::new(request.completed_turn_id.clone())
                        .with_handoff_task(queued_prompt.handoff_task),
                )
            }
            PlanningPostTurnAutoFollowDecision::Skip(reason) => {
                let reason = auto_follow_skip_reason_from_planning(reason);
                event_log::emit_lazy("auto_follow_decision", || {
                    post_turn_event_detail(
                        context.log_context(request),
                        "auto_follow",
                        "decision",
                        Some("skip"),
                        Some(runtime_snapshot),
                        [("reason", json!(format!("{:?}", reason)))],
                    )
                });
                TuiPostTurnDecision::from_action(
                    request.completed_turn_id.clone(),
                    PostTurnContinuationAction::SkipAutoFollow { reason },
                )
            }
        }
    }
}

fn auto_follow_skip_reason_from_post_turn(
    reason: PostTurnAutoFollowStopReason,
) -> AutoFollowSkipReason {
    match reason {
        PostTurnAutoFollowStopReason::PlanningQueueDrained => {
            AutoFollowSkipReason::PlanningQueueDrained
        }
        PostTurnAutoFollowStopReason::ParallelSessionCompleted => {
            AutoFollowSkipReason::ParallelSessionCompleted
        }
    }
}

fn auto_follow_skip_reason_from_planning(
    reason: PlanningPostTurnAutoFollowSkipReason,
) -> AutoFollowSkipReason {
    match reason {
        PlanningPostTurnAutoFollowSkipReason::PostTurnContinuationPaused => {
            AutoFollowSkipReason::PostTurnContinuationPaused
        }
        PlanningPostTurnAutoFollowSkipReason::PlanningQueueDrained => {
            AutoFollowSkipReason::PlanningQueueDrained
        }
        PlanningPostTurnAutoFollowSkipReason::PlanningQueueIdlePolicyStop => {
            AutoFollowSkipReason::PlanningQueueIdlePolicyStop
        }
        PlanningPostTurnAutoFollowSkipReason::LimitReached => AutoFollowSkipReason::LimitReached,
        PlanningPostTurnAutoFollowSkipReason::NoAgentReply => AutoFollowSkipReason::NoAgentReply,
        PlanningPostTurnAutoFollowSkipReason::StopKeywordMatched => {
            AutoFollowSkipReason::StopKeywordMatched
        }
        PlanningPostTurnAutoFollowSkipReason::NoFileChanges => AutoFollowSkipReason::NoFileChanges,
        PlanningPostTurnAutoFollowSkipReason::PlanningBlocked => {
            AutoFollowSkipReason::PlanningBlocked
        }
        PlanningPostTurnAutoFollowSkipReason::PlanningQueueHeadRequired => {
            AutoFollowSkipReason::PlanningQueueHeadRequired
        }
        PlanningPostTurnAutoFollowSkipReason::PlanningRepeatedQueueHead => {
            AutoFollowSkipReason::PlanningRepeatedQueueHead
        }
    }
}

fn operator_alerts_for_action(action: &PostTurnContinuationAction) -> Vec<OperatorAlert> {
    match action {
        PostTurnContinuationAction::SkipAutoFollow {
            reason: AutoFollowSkipReason::PlanningQueueDrained,
        } => vec![OperatorAlert::planning_queue_drained()],
        PostTurnContinuationAction::QueueAutoPrompt(_)
        | PostTurnContinuationAction::SkipAutoFollow { .. } => Vec::new(),
    }
}

impl NativeTuiApp {
    // Production isolates post-turn planning work behind a timeout so a stalled
    // planning worker cannot strand the TUI. Tests execute synchronously to keep
    // assertions deterministic while still exercising the same executor.
    #[tracing::instrument(level = "trace", skip(self))]
    pub(super) fn execute_post_turn_evaluation(&mut self, request: PostTurnEvaluationRequest) {
        let Some(context) = self.ready_post_turn_evaluation_context(&request) else {
            return;
        };
        self.mark_post_turn_evaluation_running(&context, &request);
        let executor = PostTurnEvaluationExecutor::new(
            self.application.planning_handle(),
            self.parallel_mode_turn_service(),
            self.planning_worker_panel_state.clone(),
        );
        #[cfg(test)]
        {
            let execution = executor.run(&context, &request);
            self.planning_worker_panel_state = execution.planning_worker_panel_state;
            self.invalidate_parallel_mode_supervisor_snapshot();
            self.dispatch_conversation_runtime(
                ConversationRuntimeEvent::PostTurnEvaluationCompleted {
                    evaluation: Box::new(execution.evaluation),
                },
            );
        }
        #[cfg(not(test))]
        {
            let tx = self.tx.clone();
            std::thread::spawn(move || {
                let (execution_tx, execution_rx) = std::sync::mpsc::channel();
                let fallback_context = context.clone();
                let fallback_request = request.clone();
                std::thread::spawn(move || {
                    let execution = executor.run(&context, &request);
                    let _ = execution_tx.send(execution);
                });
                let execution = execution_rx
                    .recv_timeout(POST_TURN_EVALUATION_TIMEOUT)
                    .unwrap_or_else(|_| {
                        post_turn_evaluation_timeout_execution(&fallback_context, &fallback_request)
                    });
                let _ = tx.send(BackgroundMessage::PostTurnEvaluationCompleted {
                    thread_id: execution.thread_id,
                    completed_turn_id: execution.completed_turn_id,
                    evaluation: Box::new(execution.evaluation),
                    planning_worker_panel_state: execution.planning_worker_panel_state,
                });
            });
        }
    }
    fn ready_post_turn_evaluation_context(
        &self,
        request: &PostTurnEvaluationRequest,
    ) -> Option<PostTurnEvaluationContext> {
        match &self.conversation_state {
            ConversationState::Ready(conversation) => Some(
                PostTurnEvaluationContext::from_conversation(conversation.as_ref(), request),
            ),
            ConversationState::Loading | ConversationState::Failed(_) => None,
        }
    }

    // The panel enters a running state only when post-turn continuation can make progress.
    // Paused continuations and queue-idle stop policy keep the previous operator
    // context visible instead of flashing a worker state that will not run.
    fn mark_post_turn_evaluation_running(
        &mut self,
        context: &PostTurnEvaluationContext,
        request: &PostTurnEvaluationRequest,
    ) {
        match self
            .application
            .planning()
            .runtime()
            .post_turn_worker_panel_start_state(PlanningPostTurnWorkerPanelStartRequest {
                continuation_paused: context.continuation_paused,
                changed_planning_file_paths: &request.changed_planning_file_paths,
                current_runtime_snapshot: &context.current_runtime_snapshot,
            }) {
            PlanningPostTurnWorkerPanelStartState::PreserveCurrent => {}
            PlanningPostTurnWorkerPanelStartState::RepairRunning => {
                self.planning_worker_panel_state.status = PlanningWorkerStatus::RepairRunning;
            }
            PlanningPostTurnWorkerPanelStartState::RefreshRunning => {
                self.planning_worker_panel_state.status = PlanningWorkerStatus::RefreshRunning;
            }
        }
    }
}
#[cfg(not(test))]
// Timeout fallback reports a failed refresh while returning control to the main
// session. The background worker may still finish later, but the UI receives a
// deterministic blocked evaluation for the completed turn.
fn post_turn_evaluation_timeout_execution(
    context: &PostTurnEvaluationContext,
    request: &PostTurnEvaluationRequest,
) -> PostTurnEvaluationExecution {
    let message = format!(
        "post-turn planning worker evaluation timed out after {} seconds",
        POST_TURN_EVALUATION_TIMEOUT.as_secs()
    );
    PostTurnEvaluationExecution {
        thread_id: context.thread_id.clone(),
        completed_turn_id: request.completed_turn_id.clone(),
        evaluation: PostTurnEvaluationOutcome {
            provenance: PostTurnEvaluationProvenance::new(request.completed_turn_id.clone()),
            runtime_snapshot: PlanningRuntimeSnapshot::invalid(message.clone()),
            planning_repair_state: None,
            runtime_notices: vec![message.clone()],
            action: PostTurnContinuationAction::SkipAutoFollow {
                reason: AutoFollowSkipReason::PostTurnEvaluationTimedOut,
            },
            operator_alerts: Vec::new(),
        },
        planning_worker_panel_state: PlanningWorkerPanelState {
            status: PlanningWorkerStatus::RefreshFailed,
            last_operation_label: Some("post-turn".to_string()),
            last_summary: Some(message),
            last_rejected_summary: None,
            last_queue_summary: Some("planning refresh timed out".to_string()),
            last_notice_detail: None,
            last_prompt: None,
            last_response: None,
            last_host_detail: Some(
                "host recovered the main-session from a stalled post-turn planning worker evaluation"
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
#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::planning::{PriorityQueueProjection, PriorityQueueSkippedTask, TaskStatus};

    #[test]
    fn post_turn_evaluator_boundary_uses_context_not_conversation_model() {
        let source = include_str!("post_turn_execution.rs");
        let official_completion_source = include_str!("post_turn_execution/official_completion.rs");
        let legacy_run_signature =
            ["fn run(\n        mut self,\n        conversation: &ConversationViewModel"].concat();
        let legacy_refresh_signature = [
            "fn run_planning_queue_refresh(\n        &mut self,\n        conversation: &ConversationViewModel",
        ]
        .concat();
        let legacy_fallback_name = ["fallback", "_conversation"].concat();

        assert!(source.contains("struct PostTurnEvaluationContext"));
        assert!(source.contains("fn ready_post_turn_evaluation_context("));
        assert!(!source.contains(&legacy_run_signature));
        assert!(!source.contains(&legacy_refresh_signature));
        assert!(!source.contains(&legacy_fallback_name));
        assert!(!official_completion_source.contains("conversation: &ConversationViewModel"));
    }

    #[test]
    fn parallel_completion_reports_drained_queue_when_official_refresh_finishes_all_work() {
        let runtime_snapshot = PlanningRuntimeSnapshot::ready_with_queue_projection(
            "Planning Context".to_string(),
            "queue idle: no executable planning task".to_string(),
            None,
            None,
            PriorityQueueProjection {
                next_task: None,
                active_tasks: Vec::new(),
                proposed_tasks: Vec::new(),
                skipped_tasks: vec![PriorityQueueSkippedTask {
                    task_id: "done-task".to_string(),
                    task_title: "Finished parallel task".to_string(),
                    direction_id: "general-workstream".to_string(),
                    status: TaskStatus::Done,
                    reason: "status done is not executable".to_string(),
                }],
            },
        );

        let decision = TuiPostTurnDecision::from_application_decision(
            "turn-1".to_string(),
            decide_parallel_official_completion_post_turn(&runtime_snapshot),
        );
        let PostTurnContinuationAction::SkipAutoFollow { reason } = decision.action else {
            panic!("parallel completion should skip auto-follow");
        };

        assert_eq!(reason, AutoFollowSkipReason::PlanningQueueDrained);
        assert_eq!(decision.provenance.completed_turn_id, "turn-1");
        assert_eq!(decision.provenance.parallel_queue_signal, None);
        assert_eq!(decision.operator_alerts.len(), 1);
        assert_eq!(
            decision.operator_alerts[0].title,
            "All planning tasks complete"
        );
    }

    #[test]
    fn parallel_completion_keeps_supervisor_handoff_when_queue_still_has_work() {
        let runtime_snapshot = PlanningRuntimeSnapshot::invalid("planning still blocked");

        let decision = TuiPostTurnDecision::from_application_decision(
            "turn-1".to_string(),
            decide_parallel_official_completion_post_turn(&runtime_snapshot),
        );
        let PostTurnContinuationAction::SkipAutoFollow { reason } = decision.action else {
            panic!("parallel completion should skip auto-follow");
        };

        assert_eq!(reason, AutoFollowSkipReason::ParallelSessionCompleted);
        assert_eq!(
            decision.provenance.parallel_queue_signal,
            Some(ParallelModePostTurnQueueSignal::ParallelCompletionFinalized)
        );
        assert!(decision.operator_alerts.is_empty());
    }
}
