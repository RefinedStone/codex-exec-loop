use std::time::Duration;

use crate::application::service::parallel_mode::turn::ParallelModeTurnService;
use crate::application::service::planning::PlanningTurnExecutionSnapshotCapture;
use crate::application::service::planning::{
    PLANNING_WORKER_REFRESH_FAILURE_BLOCK_REASON, PlanningPostTurnAutoFollowDecision,
    PlanningPostTurnAutoFollowRequest, PlanningPostTurnAutoFollowSkipReason,
    PlanningPostTurnQueueRefreshFinalizationEvent, PlanningPostTurnQueueRefreshFinalizationRequest,
    PlanningPostTurnQueueRefreshPreparation, PlanningPostTurnQueueRefreshPreparationRequest,
    PlanningPostTurnReconciliationRequest, PlanningPostTurnWorkerPanelStartRequest,
    PlanningPostTurnWorkerPanelStartState, PlanningRuntimeProjection, PlanningServices,
    PlanningTaskHandoff,
};
use crate::application::service::post_turn_decision::{
    PostTurnAutoFollowStopReason, PostTurnDecision as ApplicationPostTurnDecision,
    decide_parallel_official_completion_post_turn,
};
use crate::diagnostics::event_log;
use crate::domain::operator_alert::OperatorAlert;
use crate::domain::parallel_mode::ParallelModePostTurnQueueSignal;
use serde_json::json;

pub(crate) const POST_TURN_EVALUATION_TIMEOUT: Duration = Duration::from_secs(600);
#[path = "post_turn_evaluation/logging.rs"]
mod logging;
#[path = "post_turn_evaluation/official_completion.rs"]
mod official_completion;
#[path = "post_turn_evaluation/planning_worker_panel.rs"]
mod planning_worker_panel;
#[path = "post_turn_evaluation/repair.rs"]
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
#[derive(Clone)]
pub struct PostTurnEvaluationService {
    planning_feature: PlanningServices,
    parallel_mode_turn_service: ParallelModeTurnService,
}

impl PostTurnEvaluationService {
    pub fn new(
        planning_feature: PlanningServices,
        parallel_mode_turn_service: ParallelModeTurnService,
    ) -> Self {
        Self {
            planning_feature,
            parallel_mode_turn_service,
        }
    }

    pub fn worker_panel_start_state(
        &self,
        request: &PostTurnEvaluationRequest,
    ) -> PlanningPostTurnWorkerPanelStartState {
        self.planning_feature
            .runtime
            .post_turn_worker_panel_start_state(PlanningPostTurnWorkerPanelStartRequest {
                continuation_paused: request.context.continuation_paused,
                changed_planning_file_paths: &request.changed_planning_file_paths,
                current_runtime_projection: &request.context.current_runtime_projection,
            })
    }

    pub fn evaluate(&self, request: PostTurnEvaluationRequest) -> PostTurnEvaluationExecution {
        let executor = PostTurnEvaluationExecutor::new(
            self.planning_feature.clone(),
            self.parallel_mode_turn_service.clone(),
            request.planning_worker_panel_state.clone(),
        );
        executor.run(&request.context, &request)
    }

    pub fn evaluate_with_timeout(
        &self,
        request: PostTurnEvaluationRequest,
        timeout: Duration,
    ) -> PostTurnEvaluationExecution {
        let (execution_tx, execution_rx) = std::sync::mpsc::channel();
        let timeout_context = request.context.clone();
        let timeout_request = request.clone();
        let service = self.clone();
        std::thread::spawn(move || {
            let execution = service.evaluate(request);
            let _ = execution_tx.send(execution);
        });
        execution_rx.recv_timeout(timeout).unwrap_or_else(|_| {
            post_turn_evaluation_timeout_execution(&timeout_context, &timeout_request, timeout)
        })
    }
}

// Post-turn evaluation is the handoff between a completed Codex turn and the
// planning/parallel-mode continuation that may schedule the next prompt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PostTurnEvaluationRequest {
    pub context: PostTurnEvaluationContext,
    pub workspace_directory: String,
    pub completed_turn_id: String,
    pub changed_planning_file_paths: Vec<String>,
    pub execution_snapshot_capture: Option<PlanningTurnExecutionSnapshotCapture>,
    pub planning_worker_panel_state: PlanningWorkerPanelState,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PostTurnEvaluationContext {
    pub thread_id: String,
    pub planning_workspace_directory: String,
    pub latest_user_message: Option<String>,
    pub latest_main_reply: Option<String>,
    pub previous_handoff_task: Option<PlanningTaskHandoff>,
    pub current_runtime_projection: PlanningRuntimeProjection,
    pub continuation_paused: bool,
    pub can_queue_next: bool,
    pub stop_keyword: String,
    pub stop_keyword_matched: bool,
    pub no_file_changes_stop_matched: bool,
    pub mode_label: String,
}

impl PostTurnEvaluationContext {
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

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum PlanningWorkerStatus {
    #[default]
    Idle,
    RefreshRunning,
    RefreshSucceeded,
    RefreshFailed,
    RepairRunning,
    RepairSucceeded,
    RepairFailed,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PlanningWorkerPanelState {
    pub status: PlanningWorkerStatus,
    pub last_operation_label: Option<String>,
    pub last_summary: Option<String>,
    pub last_rejected_summary: Option<String>,
    pub last_queue_summary: Option<String>,
    pub last_notice_detail: Option<String>,
    pub last_prompt: Option<String>,
    pub last_response: Option<String>,
    pub last_host_detail: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PostTurnEvaluationOutcome {
    pub provenance: PostTurnEvaluationProvenance,
    pub runtime_projection: PlanningRuntimeProjection,
    pub planning_repair_state: Option<PostTurnPlanningRepairState>,
    pub runtime_notices: Vec<String>,
    pub action: PostTurnContinuationAction,
    pub operator_alerts: Vec<OperatorAlert>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PostTurnPlanningRepairState {
    pub attempts_used: usize,
    pub max_attempts: usize,
    pub latest_request: crate::application::service::planning::PlanningRepairRequest,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PostTurnEvaluationProvenance {
    pub completed_turn_id: String,
    pub handoff_task: Option<PlanningTaskHandoff>,
    pub parallel_queue_signal: Option<ParallelModePostTurnQueueSignal>,
}

impl PostTurnEvaluationProvenance {
    pub fn new(completed_turn_id: String) -> Self {
        Self {
            completed_turn_id,
            handoff_task: None,
            parallel_queue_signal: None,
        }
    }

    pub fn with_handoff_task(mut self, handoff_task: Option<PlanningTaskHandoff>) -> Self {
        self.handoff_task = handoff_task;
        self
    }

    pub fn with_parallel_queue_signal(
        mut self,
        parallel_queue_signal: Option<ParallelModePostTurnQueueSignal>,
    ) -> Self {
        self.parallel_queue_signal = parallel_queue_signal;
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PostTurnQueuedPrompt {
    pub prompt: String,
    pub mode_label: String,
    pub transcript_text: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PostTurnContinuationAction {
    QueueAutoPrompt(Box<PostTurnQueuedPrompt>),
    SkipAutoFollow {
        reason: PostTurnAutoFollowSkipReason,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PostTurnAutoFollowSkipReason {
    PostTurnContinuationPaused,
    LimitReached,
    NoAgentReply,
    StopKeywordMatched,
    NoFileChanges,
    PlanningBlocked,
    PlanningQueueIdlePolicyStop,
    PlanningQueueHeadRequired,
    PlanningQueueDrained,
    PlanningRepeatedQueueHead,
    ParallelSessionCompleted,
    PostTurnEvaluationTimedOut,
}

#[derive(Debug, Clone)]
struct HiddenPlanningRepairOutcome {
    runtime_projection: PlanningRuntimeProjection,
    resolved: bool,
}
#[derive(Debug, Clone)]
struct PlanningQueueRefreshOutcome {
    runtime_projection: PlanningRuntimeProjection,
}
#[derive(Debug, Clone)]
struct OfficialCompletionRefreshOutcome {
    runtime_projection: PlanningRuntimeProjection,
    runtime_notices: Vec<String>,
}
#[derive(Debug, Clone)]
struct PostTurnDecision {
    action: PostTurnContinuationAction,
    provenance: PostTurnEvaluationProvenance,
    operator_alerts: Vec<OperatorAlert>,
}
impl PostTurnDecision {
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
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(test, allow(dead_code))]
pub struct PostTurnEvaluationExecution {
    pub thread_id: String,
    pub completed_turn_id: String,
    pub evaluation: PostTurnEvaluationOutcome,
    pub planning_worker_panel_state: PlanningWorkerPanelState,
}
#[derive(Clone)]
struct PostTurnEvaluationExecutor {
    planning_feature: PlanningServices,
    parallel_mode_turn_service: ParallelModeTurnService,
    planning_worker_panel_state: PlanningWorkerPanelState,
}
impl PostTurnEvaluationExecutor {
    fn new(
        planning_feature: PlanningServices,
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
    // from the final runtime projection.
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
                Some(&context.current_runtime_projection),
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
        let reconciliation_outcome = self.planning_feature.runtime.reconcile_post_turn(
            PlanningPostTurnReconciliationRequest {
                workspace_directory: &request.workspace_directory,
                completed_turn_id: &request.completed_turn_id,
                changed_planning_file_paths: &request.changed_planning_file_paths,
                execution_snapshot_capture: request.execution_snapshot_capture.as_ref(),
                current_runtime_projection: &context.current_runtime_projection,
            },
        );
        let reconciliation_result = reconciliation_outcome.reconciliation_result;
        let mut runtime_notices = reconciliation_result.notices.clone();
        let mut runtime_projection = reconciliation_outcome.runtime_projection;
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
            runtime_projection = repair_outcome.runtime_projection;
        }
        let handled_parallel_completion =
            if let Some(completion_report) = official_completion_report {
                let official_completion_outcome = self.run_official_completion_refresh(
                    context,
                    request,
                    planning_workspace_directory,
                    &runtime_projection,
                    &completion_report,
                );
                runtime_notices.extend(official_completion_outcome.runtime_notices.clone());
                runtime_projection = official_completion_outcome.runtime_projection;
                true
            } else {
                false
            };
        if !handled_parallel_completion && continuation_enabled {
            let refresh_outcome =
                self.run_planning_queue_refresh(context, request, runtime_projection.clone());
            runtime_projection = refresh_outcome.runtime_projection;
        }
        let post_turn_decision = if handled_parallel_completion {
            PostTurnDecision::from_application_decision(
                request.completed_turn_id.clone(),
                decide_parallel_official_completion_post_turn(&runtime_projection),
            )
        } else {
            self.auto_follow_decision_from_projection(context, request, &runtime_projection)
        };
        event_log::emit_lazy("post_turn_evaluation_completed", || {
            post_turn_event_detail(
                context.log_context(request),
                "post_turn",
                "completed",
                Some(post_turn_action_decision(&post_turn_decision.action)),
                Some(&runtime_projection),
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
                runtime_projection,
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
        current_projection: PlanningRuntimeProjection,
    ) -> PlanningQueueRefreshOutcome {
        let preparation = self
            .planning_feature
            .worker
            .prepare_post_turn_queue_refresh(PlanningPostTurnQueueRefreshPreparationRequest {
                workspace_directory: &request.workspace_directory,
                parent_thread_id: Some(context.thread_id.as_str())
                    .filter(|thread_id| !thread_id.trim().is_empty()),
                completed_turn_id: &request.completed_turn_id,
                latest_user_message: context.latest_user_message.as_deref(),
                latest_main_reply: context.latest_main_reply.as_deref(),
                previous_handoff_task: context.previous_handoff_task(),
                current_runtime_projection: &current_projection,
            });
        let prepared = match preparation {
            PlanningPostTurnQueueRefreshPreparation::Skipped(skipped) => {
                event_log::emit_lazy("planning_worker_refresh_skipped", || {
                    planning_worker_refresh_skipped_detail(
                        context.log_context(request),
                        skipped.reason.log_label(),
                        &skipped.runtime_projection,
                    )
                });
                return PlanningQueueRefreshOutcome {
                    runtime_projection: skipped.runtime_projection,
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
                Some(&current_projection),
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
            .worker
            .refresh_prepared_queue_from_reply(prepared.as_ref());
        let outcome = match worker_outcome {
            Ok(outcome) => outcome,
            Err(error) => {
                let detail = if prepared.is_queue_idle_derivation() {
                    format!("planning worker queue-idle derivation failed: {error}")
                } else {
                    format!("planning worker refresh failed: {error}")
                };
                let invalid_projection = PlanningRuntimeProjection::invalid(
                    PLANNING_WORKER_REFRESH_FAILURE_BLOCK_REASON,
                );
                event_log::emit_lazy("planning_worker_refresh_failed", || {
                    post_turn_event_detail(
                        context.log_context(request),
                        "refresh",
                        "worker_failed",
                        Some("block_auto_follow"),
                        Some(&invalid_projection),
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
                    &invalid_projection,
                );
                return PlanningQueueRefreshOutcome {
                    runtime_projection: invalid_projection,
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
                Some(&outcome.runtime_projection),
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
        let mut runtime_projection = outcome.runtime_projection.clone();
        if let Some(repair_request) = outcome.repair_request.as_ref() {
            let repair_outcome = self.run_hidden_planning_repairs(
                context.thread_id.as_str(),
                &request.workspace_directory,
                &request.completed_turn_id,
                repair_request,
                context.previous_handoff_task(),
            );
            runtime_projection = if repair_outcome.resolved {
                repair_outcome.runtime_projection
            } else {
                event_log::emit_lazy("planning_worker_refresh_repair_unresolved", || {
                    post_turn_event_detail(
                        context.log_context(request),
                        "repair",
                        "unresolved_after_refresh",
                        Some("block_auto_follow"),
                        Some(&repair_outcome.runtime_projection),
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
                PlanningRuntimeProjection::invalid(PLANNING_WORKER_REFRESH_FAILURE_BLOCK_REASON)
            };
        }
        let finalization = self
            .planning_feature
            .worker
            .finalize_post_turn_queue_refresh(PlanningPostTurnQueueRefreshFinalizationRequest {
                workspace_directory: &request.workspace_directory,
                previous_handoff_task: context.previous_handoff_task(),
                previous_runtime_projection: &context.current_runtime_projection,
                refreshed_runtime_projection: &runtime_projection,
                queue_idle_derivation: prepared.is_queue_idle_derivation(),
            });
        runtime_projection = finalization.runtime_projection;
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
                            Some(&promotion_outcome.runtime_projection),
                            [(
                                "promoted_task_title",
                                json!(promotion_outcome.promoted_task_title.as_deref()),
                            )],
                        )
                    });
                    self.planning_worker_panel_state.last_queue_summary =
                        planning_worker_queue_summary(&promotion_outcome.runtime_projection);
                    self.planning_worker_panel_state.last_host_detail =
                        promotion_outcome.promoted_task_title.map(|title| {
                            format!(
                                "host promoted top follow-up proposal into the executable queue: {title}"
                            )
                        });
                }
                PlanningPostTurnQueueRefreshFinalizationEvent::ProposalPromotionFailed {
                    detail,
                    runtime_projection: invalid_projection,
                } => {
                    event_log::emit_lazy("planning_worker_proposal_promotion_failed", || {
                        post_turn_event_detail(
                            context.log_context(request),
                            "proposal_promotion",
                            "failed",
                            Some("block_auto_follow"),
                            Some(&invalid_projection),
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
                        &invalid_projection,
                    );
                    return PlanningQueueRefreshOutcome {
                        runtime_projection: invalid_projection,
                    };
                }
                PlanningPostTurnQueueRefreshFinalizationEvent::QueueIdleDerivationEmpty {
                    detail,
                } => {
                    self.planning_worker_panel_state.last_host_detail = Some(detail);
                }
                PlanningPostTurnQueueRefreshFinalizationEvent::RepeatedQueueHead {
                    detail,
                    runtime_projection: guard_projection,
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
                                Some(&guard_projection),
                                [("pause_reason", json!(detail.as_str()))],
                            )
                        },
                    );
                }
            }
        }

        PlanningQueueRefreshOutcome { runtime_projection }
    }

    // The final action is always derived from the latest runtime projection. Explicit
    // pause states and queue-idle stop policy win before the conversation model
    // is allowed to enqueue another prompt.
    #[tracing::instrument(level = "trace", skip(self, context))]
    fn auto_follow_decision_from_projection(
        &self,
        context: &PostTurnEvaluationContext,
        request: &PostTurnEvaluationRequest,
        runtime_projection: &PlanningRuntimeProjection,
    ) -> PostTurnDecision {
        match self.planning_feature.runtime.decide_post_turn_auto_follow(
            PlanningPostTurnAutoFollowRequest {
                continuation_paused: context.continuation_paused,
                can_queue_next: context.can_queue_next,
                latest_agent_message: context.latest_main_reply.as_deref(),
                stop_keyword: context.stop_keyword.as_str(),
                stop_keyword_matched: context.stop_keyword_matched,
                no_file_changes_stop_matched: context.no_file_changes_stop_matched,
                runtime_projection,
            },
        ) {
            PlanningPostTurnAutoFollowDecision::QueuePrompt(queued_prompt) => {
                event_log::emit_lazy("auto_follow_decision", || {
                    post_turn_event_detail(
                        context.log_context(request),
                        "auto_follow",
                        "decision",
                        Some("queue"),
                        Some(runtime_projection),
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
                PostTurnDecision::from_action_with_provenance(
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
                        Some(runtime_projection),
                        [("reason", json!(format!("{:?}", reason)))],
                    )
                });
                PostTurnDecision::from_action(
                    request.completed_turn_id.clone(),
                    PostTurnContinuationAction::SkipAutoFollow { reason },
                )
            }
        }
    }
}

fn auto_follow_skip_reason_from_post_turn(
    reason: PostTurnAutoFollowStopReason,
) -> PostTurnAutoFollowSkipReason {
    match reason {
        PostTurnAutoFollowStopReason::PlanningQueueDrained => {
            PostTurnAutoFollowSkipReason::PlanningQueueDrained
        }
        PostTurnAutoFollowStopReason::ParallelSessionCompleted => {
            PostTurnAutoFollowSkipReason::ParallelSessionCompleted
        }
    }
}

fn auto_follow_skip_reason_from_planning(
    reason: PlanningPostTurnAutoFollowSkipReason,
) -> PostTurnAutoFollowSkipReason {
    match reason {
        PlanningPostTurnAutoFollowSkipReason::PostTurnContinuationPaused => {
            PostTurnAutoFollowSkipReason::PostTurnContinuationPaused
        }
        PlanningPostTurnAutoFollowSkipReason::PlanningQueueDrained => {
            PostTurnAutoFollowSkipReason::PlanningQueueDrained
        }
        PlanningPostTurnAutoFollowSkipReason::PlanningQueueIdlePolicyStop => {
            PostTurnAutoFollowSkipReason::PlanningQueueIdlePolicyStop
        }
        PlanningPostTurnAutoFollowSkipReason::LimitReached => {
            PostTurnAutoFollowSkipReason::LimitReached
        }
        PlanningPostTurnAutoFollowSkipReason::NoAgentReply => {
            PostTurnAutoFollowSkipReason::NoAgentReply
        }
        PlanningPostTurnAutoFollowSkipReason::StopKeywordMatched => {
            PostTurnAutoFollowSkipReason::StopKeywordMatched
        }
        PlanningPostTurnAutoFollowSkipReason::NoFileChanges => {
            PostTurnAutoFollowSkipReason::NoFileChanges
        }
        PlanningPostTurnAutoFollowSkipReason::PlanningBlocked => {
            PostTurnAutoFollowSkipReason::PlanningBlocked
        }
        PlanningPostTurnAutoFollowSkipReason::PlanningQueueHeadRequired => {
            PostTurnAutoFollowSkipReason::PlanningQueueHeadRequired
        }
        PlanningPostTurnAutoFollowSkipReason::PlanningRepeatedQueueHead => {
            PostTurnAutoFollowSkipReason::PlanningRepeatedQueueHead
        }
    }
}

fn operator_alerts_for_action(action: &PostTurnContinuationAction) -> Vec<OperatorAlert> {
    match action {
        PostTurnContinuationAction::SkipAutoFollow {
            reason: PostTurnAutoFollowSkipReason::PlanningQueueDrained,
        } => vec![OperatorAlert::planning_queue_drained()],
        PostTurnContinuationAction::QueueAutoPrompt(_)
        | PostTurnContinuationAction::SkipAutoFollow { .. } => Vec::new(),
    }
}

// Timeout fallback reports a failed refresh while returning control to the main
// session. The background worker may still finish later, but the UI receives a
// deterministic blocked evaluation for the completed turn.
fn post_turn_evaluation_timeout_execution(
    context: &PostTurnEvaluationContext,
    request: &PostTurnEvaluationRequest,
    timeout: Duration,
) -> PostTurnEvaluationExecution {
    let message = format!(
        "post-turn planning worker evaluation timed out after {} seconds",
        timeout.as_secs()
    );
    PostTurnEvaluationExecution {
        thread_id: context.thread_id.clone(),
        completed_turn_id: request.completed_turn_id.clone(),
        evaluation: PostTurnEvaluationOutcome {
            provenance: PostTurnEvaluationProvenance::new(request.completed_turn_id.clone()),
            runtime_projection: PlanningRuntimeProjection::invalid(message.clone()),
            planning_repair_state: None,
            runtime_notices: vec![message.clone()],
            action: PostTurnContinuationAction::SkipAutoFollow {
                reason: PostTurnAutoFollowSkipReason::PostTurnEvaluationTimedOut,
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
#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapter::outbound::db::SqlitePlanningAuthorityAdapter;
    use crate::adapter::outbound::filesystem::FilesystemPlanningWorkspaceAdapter;
    use crate::adapter::outbound::git::parallel_mode_runtime::GitParallelModeRuntimeAdapter;
    use crate::adapter::outbound::github::GithubAutomationAdapter;
    use crate::application::port::outbound::planning_authority_port::NoopPlanningAuthorityPort;
    use crate::application::port::outbound::planning_task_repository_port::NoopPlanningTaskRepositoryPort;
    use crate::application::port::outbound::planning_worker_port::NoopPlanningWorkerPort;
    use crate::application::service::parallel_mode::ParallelModeService;
    use crate::domain::planning::{
        PriorityQueueProjection, PriorityQueueSkippedTask, PriorityQueueTask, TaskStatus,
    };
    use std::sync::Arc;

    #[test]
    fn post_turn_evaluator_boundary_uses_context_not_conversation_model() {
        let source = include_str!("post_turn_evaluation.rs");
        let official_completion_source =
            include_str!("post_turn_evaluation/official_completion.rs");
        let legacy_run_signature =
            ["fn run(\n        mut self,\n        conversation: &ConversationViewModel"].concat();
        let legacy_refresh_signature = [
            "fn run_planning_queue_refresh(\n        &mut self,\n        conversation: &ConversationViewModel",
        ]
        .concat();
        let legacy_fallback_name = ["fallback", "_conversation"].concat();

        assert!(source.contains("struct PostTurnEvaluationContext"));
        assert!(!source.contains(&legacy_run_signature));
        assert!(!source.contains(&legacy_refresh_signature));
        assert!(!source.contains(&legacy_fallback_name));
        assert!(!official_completion_source.contains("conversation: &ConversationViewModel"));
    }

    #[test]
    fn parallel_completion_reports_drained_queue_when_official_refresh_finishes_all_work() {
        let runtime_projection = PlanningRuntimeProjection::ready_with_queue_projection(
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

        let decision = PostTurnDecision::from_application_decision(
            "turn-1".to_string(),
            decide_parallel_official_completion_post_turn(&runtime_projection),
        );
        let PostTurnContinuationAction::SkipAutoFollow { reason } = decision.action else {
            panic!("parallel completion should skip auto-follow");
        };

        assert_eq!(reason, PostTurnAutoFollowSkipReason::PlanningQueueDrained);
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
        let runtime_projection = PlanningRuntimeProjection::invalid("planning still blocked");

        let decision = PostTurnDecision::from_application_decision(
            "turn-1".to_string(),
            decide_parallel_official_completion_post_turn(&runtime_projection),
        );
        let PostTurnContinuationAction::SkipAutoFollow { reason } = decision.action else {
            panic!("parallel completion should skip auto-follow");
        };

        assert_eq!(
            reason,
            PostTurnAutoFollowSkipReason::ParallelSessionCompleted
        );
        assert_eq!(
            decision.provenance.parallel_queue_signal,
            Some(ParallelModePostTurnQueueSignal::ParallelCompletionFinalized)
        );
        assert!(decision.operator_alerts.is_empty());
    }

    #[test]
    fn timeout_execution_returns_blocked_action_and_failed_panel_state() {
        let context = test_context(ready_projection(Some(queue_task())));
        let request = test_request(context.clone());

        let execution =
            post_turn_evaluation_timeout_execution(&context, &request, Duration::from_secs(7));

        assert_eq!(execution.thread_id, "thread-1");
        assert_eq!(execution.completed_turn_id, "turn-1");
        assert_eq!(
            execution.evaluation.action,
            PostTurnContinuationAction::SkipAutoFollow {
                reason: PostTurnAutoFollowSkipReason::PostTurnEvaluationTimedOut
            }
        );
        assert_eq!(
            execution.evaluation.runtime_projection.failure_reason(),
            Some("post-turn planning worker evaluation timed out after 7 seconds")
        );
        assert_eq!(
            execution.planning_worker_panel_state.status,
            PlanningWorkerStatus::RefreshFailed
        );
        assert_eq!(
            execution
                .planning_worker_panel_state
                .last_queue_summary
                .as_deref(),
            Some("planning refresh timed out")
        );
    }

    #[test]
    fn operator_alerts_only_surface_planning_queue_drained_skip() {
        assert_eq!(
            operator_alerts_for_action(&PostTurnContinuationAction::QueueAutoPrompt(Box::new(
                PostTurnQueuedPrompt {
                    prompt: "continue".to_string(),
                    mode_label: "auto".to_string(),
                    transcript_text: "queued".to_string(),
                },
            ))),
            Vec::<OperatorAlert>::new()
        );
        assert_eq!(
            operator_alerts_for_action(&PostTurnContinuationAction::SkipAutoFollow {
                reason: PostTurnAutoFollowSkipReason::NoAgentReply,
            }),
            Vec::<OperatorAlert>::new()
        );

        let alerts = operator_alerts_for_action(&PostTurnContinuationAction::SkipAutoFollow {
            reason: PostTurnAutoFollowSkipReason::PlanningQueueDrained,
        });

        assert_eq!(alerts.len(), 1);
        assert_eq!(alerts[0].title, "All planning tasks complete");
    }

    #[test]
    fn planning_skip_reason_mapping_covers_all_post_turn_action_variants() {
        let cases = [
            (
                PlanningPostTurnAutoFollowSkipReason::PostTurnContinuationPaused,
                PostTurnAutoFollowSkipReason::PostTurnContinuationPaused,
            ),
            (
                PlanningPostTurnAutoFollowSkipReason::PlanningQueueDrained,
                PostTurnAutoFollowSkipReason::PlanningQueueDrained,
            ),
            (
                PlanningPostTurnAutoFollowSkipReason::PlanningQueueIdlePolicyStop,
                PostTurnAutoFollowSkipReason::PlanningQueueIdlePolicyStop,
            ),
            (
                PlanningPostTurnAutoFollowSkipReason::LimitReached,
                PostTurnAutoFollowSkipReason::LimitReached,
            ),
            (
                PlanningPostTurnAutoFollowSkipReason::NoAgentReply,
                PostTurnAutoFollowSkipReason::NoAgentReply,
            ),
            (
                PlanningPostTurnAutoFollowSkipReason::StopKeywordMatched,
                PostTurnAutoFollowSkipReason::StopKeywordMatched,
            ),
            (
                PlanningPostTurnAutoFollowSkipReason::NoFileChanges,
                PostTurnAutoFollowSkipReason::NoFileChanges,
            ),
            (
                PlanningPostTurnAutoFollowSkipReason::PlanningBlocked,
                PostTurnAutoFollowSkipReason::PlanningBlocked,
            ),
            (
                PlanningPostTurnAutoFollowSkipReason::PlanningQueueHeadRequired,
                PostTurnAutoFollowSkipReason::PlanningQueueHeadRequired,
            ),
            (
                PlanningPostTurnAutoFollowSkipReason::PlanningRepeatedQueueHead,
                PostTurnAutoFollowSkipReason::PlanningRepeatedQueueHead,
            ),
        ];

        for (planning_reason, post_turn_reason) in cases {
            assert_eq!(
                auto_follow_skip_reason_from_planning(planning_reason),
                post_turn_reason
            );
        }
        assert_eq!(
            auto_follow_skip_reason_from_post_turn(
                PostTurnAutoFollowStopReason::PlanningQueueDrained,
            ),
            PostTurnAutoFollowSkipReason::PlanningQueueDrained
        );
        assert_eq!(
            auto_follow_skip_reason_from_post_turn(
                PostTurnAutoFollowStopReason::ParallelSessionCompleted,
            ),
            PostTurnAutoFollowSkipReason::ParallelSessionCompleted
        );
    }

    #[test]
    fn queue_refresh_skip_keeps_projection_and_preserves_existing_panel_state() {
        let mut executor = test_executor();
        executor.planning_worker_panel_state.status = PlanningWorkerStatus::RefreshSucceeded;
        executor.planning_worker_panel_state.last_summary = Some("previous summary".to_string());
        let mut context = test_context(PlanningRuntimeProjection::invalid("planning blocked"));
        context.latest_main_reply = Some("worker reply".to_string());
        let request = test_request(context.clone());

        let outcome = executor.run_planning_queue_refresh(
            &context,
            &request,
            context.current_runtime_projection.clone(),
        );

        assert_eq!(
            outcome.runtime_projection.failure_reason(),
            Some("planning blocked")
        );
        assert_eq!(
            executor.planning_worker_panel_state.status,
            PlanningWorkerStatus::RefreshSucceeded
        );
        assert_eq!(
            executor.planning_worker_panel_state.last_summary.as_deref(),
            Some("previous summary")
        );
    }

    #[test]
    fn auto_follow_decision_queues_prompt_with_handoff_provenance() {
        let executor = test_executor();
        let context = test_context(ready_projection(Some(queue_task())));
        let request = test_request(context.clone());

        let decision = executor.auto_follow_decision_from_projection(
            &context,
            &request,
            &context.current_runtime_projection,
        );

        let PostTurnContinuationAction::QueueAutoPrompt(prompt) = decision.action else {
            panic!("ready queue head should produce queued prompt action");
        };
        assert_eq!(prompt.mode_label, "auto-follow");
        assert!(prompt.prompt.contains("Queue head"));
        assert_eq!(
            decision
                .provenance
                .handoff_task
                .as_ref()
                .map(|task| task.task_id.as_str()),
            Some("task-1")
        );
        assert!(decision.operator_alerts.is_empty());
    }

    #[test]
    fn auto_follow_decision_maps_skip_to_post_turn_action() {
        let executor = test_executor();
        let mut context = test_context(ready_projection(Some(queue_task())));
        context.can_queue_next = false;
        let request = test_request(context.clone());

        let decision = executor.auto_follow_decision_from_projection(
            &context,
            &request,
            &context.current_runtime_projection,
        );

        assert_eq!(
            decision.action,
            PostTurnContinuationAction::SkipAutoFollow {
                reason: PostTurnAutoFollowSkipReason::LimitReached
            }
        );
        assert_eq!(decision.provenance.completed_turn_id, "turn-1");
        assert!(decision.operator_alerts.is_empty());
    }

    fn test_executor() -> PostTurnEvaluationExecutor {
        PostTurnEvaluationExecutor::new(
            PlanningServices::from_ports(
                Arc::new(FilesystemPlanningWorkspaceAdapter::new()),
                Arc::new(NoopPlanningAuthorityPort::default()),
                Arc::new(NoopPlanningTaskRepositoryPort),
                Arc::new(NoopPlanningWorkerPort),
            ),
            ParallelModeTurnService::new(ParallelModeService::new(
                Arc::new(SqlitePlanningAuthorityAdapter::new()),
                Arc::new(GithubAutomationAdapter::new()),
                Arc::new(GitParallelModeRuntimeAdapter::new()),
            )),
            PlanningWorkerPanelState::default(),
        )
    }

    fn test_context(
        current_runtime_projection: PlanningRuntimeProjection,
    ) -> PostTurnEvaluationContext {
        PostTurnEvaluationContext {
            thread_id: "thread-1".to_string(),
            planning_workspace_directory: "/tmp/workspace".to_string(),
            latest_user_message: Some("user request".to_string()),
            latest_main_reply: Some("assistant reply".to_string()),
            previous_handoff_task: None,
            current_runtime_projection,
            continuation_paused: false,
            can_queue_next: true,
            stop_keyword: "stop".to_string(),
            stop_keyword_matched: false,
            no_file_changes_stop_matched: false,
            mode_label: "auto-follow".to_string(),
        }
    }

    fn test_request(context: PostTurnEvaluationContext) -> PostTurnEvaluationRequest {
        PostTurnEvaluationRequest {
            context,
            workspace_directory: "/tmp/workspace".to_string(),
            completed_turn_id: "turn-1".to_string(),
            changed_planning_file_paths: Vec::new(),
            execution_snapshot_capture: None,
            planning_worker_panel_state: PlanningWorkerPanelState::default(),
        }
    }

    fn ready_projection(queue_head: Option<PriorityQueueTask>) -> PlanningRuntimeProjection {
        PlanningRuntimeProjection::ready(
            "Planning Context".to_string(),
            "queue summary".to_string(),
            queue_head,
        )
    }

    fn queue_task() -> PriorityQueueTask {
        PriorityQueueTask {
            rank: 1,
            task_id: "task-1".to_string(),
            direction_id: "general-workstream".to_string(),
            direction_title: "General".to_string(),
            task_title: "Queue head".to_string(),
            status: TaskStatus::Ready,
            combined_priority: 80,
            updated_at: "2026-05-12T00:00:00Z".to_string(),
            rank_reasons: vec!["ready".to_string()],
        }
    }
}
