// Parallel slot completion report is the domain contract captured by the supervisor
// when a worker finishes its assigned task.
use crate::application::service::parallel_mode::ParallelModeOfficialCompletionReport;
// Official completion refresh turns that report back into planning ledger state and
// a runtime snapshot the TUI can use for follow-up gating.
use crate::application::service::planning::{
    PlanningPostTurnOfficialCompletionFinalizationRequest,
    PlanningPostTurnOfficialCompletionPreparation,
    PlanningPostTurnOfficialCompletionPreparationRequest,
    PlanningPostTurnOfficialCompletionRepairBlockRequest, PlanningRuntimeSnapshot,
};
use crate::diagnostics::event_log;
use serde_json::json;

// The refresh path reads conversation context and reports progress through the same
// planning worker panel used by planning queue refresh and hidden repair.
use super::super::super::PlanningWorkerStatus;
// Repeated-head detection is shared with planning queue refresh so official completion
// cannot requeue a slot task that failed to advance planning.
use super::logging::post_turn_event_detail;
// The parent post-turn module owns the failure constant and DTOs; this file owns
// only the official-completion branch of the executor.
use super::{
    OfficialCompletionRefreshOutcome, PostTurnEvaluationContext, PostTurnEvaluationExecutor,
    PostTurnEvaluationRequest,
};

/*
 * Official completion is the parallel-mode exit path. A slot session does not keep
 * auto-following in-place after its assignment is complete; instead this branch
 * captures the completed work, asks the planning worker to refresh the authoritative
 * ledger, and then tells the supervisor whether the slot can be finalized.
 */
impl PostTurnEvaluationExecutor {
    /*
     * Capture the finished slot turn before any ledger refresh runs. The completion
     * report is the handoff contract between the worker session and supervisor:
     * it carries final response copy, changed planning-file context, and the task
     * identity later used by the official refresh worker.
     */
    pub(super) fn begin_official_completion_if_needed(
        &mut self,
        // Context supplies the latest committed agent reply and current runtime snapshot.
        context: &PostTurnEvaluationContext,
        // Request anchors the slot workspace and queued turn id used by the supervisor.
        request: &PostTurnEvaluationRequest,
    ) -> Option<ParallelModeOfficialCompletionReport> {
        // Prefer the committed transcript reply over the report fallback so the
        // supervisor sees exactly what the operator saw in the slot session.
        let latest_main_reply = context
            .latest_main_reply
            .as_deref()
            .map(str::trim)
            .filter(|message| !message.is_empty());
        // Validation summary tells the supervisor whether protected planning files
        // were already reconciled before official ledger refresh starts.
        let validation_summary = if request.changed_planning_file_paths.is_empty() {
            "turn completed without planning file changes"
        } else {
            "turn completed with planning file changes; protected planning files were reconciled before official refresh"
        };

        match self.parallel_mode_turn_service.begin_official_completion(
            &request.workspace_directory,
            &request.completed_turn_id,
            None,
            latest_main_reply,
            Some(validation_summary),
        ) {
            Ok(report) => report,
            Err(error) => {
                // Capture failure means the supervisor could not create the
                // official-completion report; the current runtime snapshot is
                // still the most truthful state for the panel.
                event_log::emit_lazy("official_completion_capture_failed", || {
                    post_turn_event_detail(
                        context.log_context(request),
                        "official_completion",
                        "capture_failed",
                        Some("skip_official_refresh"),
                        Some(&context.current_runtime_snapshot),
                        [
                            ("error", json!(error.to_string())),
                            (
                                "changed_planning_file_count",
                                json!(request.changed_planning_file_paths.len()),
                            ),
                        ],
                    )
                });
                self.record_planning_worker_failure(
                    PlanningWorkerStatus::RefreshFailed,
                    &format!("parallel completion capture failed: {error}"),
                    &context.current_runtime_snapshot,
                );
                None
            }
        }
    }

    /*
     * Run the official completion ledger refresh. Unlike planning queue refresh,
     * this path is driven by a completed parallel slot contract and must update the
     * supervisor reservation state before returning a snapshot to post-turn action
     * selection.
     */
    pub(super) fn run_official_completion_refresh(
        &mut self,
        // Source context supplies transcript context and previous handoff data.
        context: &PostTurnEvaluationContext,
        // Original post-turn request identifies the slot workspace and completed turn id.
        request: &PostTurnEvaluationRequest,
        // Planning workspace may differ from the slot workspace when supervisor state is external.
        planning_workspace_directory: &str,
        // Current snapshot is reused when the planning workspace is the active turn workspace.
        current_snapshot: &PlanningRuntimeSnapshot,
        // Completion report is the official contract captured before this refresh.
        completion_report: &ParallelModeOfficialCompletionReport,
    ) -> OfficialCompletionRefreshOutcome {
        let preparation = self
            .planning_feature
            .worker()
            .prepare_post_turn_official_completion_refresh(
                PlanningPostTurnOfficialCompletionPreparationRequest {
                    planning_workspace_directory,
                    turn_workspace_directory: &request.workspace_directory,
                    parent_thread_id: Some(context.thread_id.as_str())
                        .filter(|thread_id| !thread_id.trim().is_empty()),
                    latest_user_message: context.latest_user_message.as_deref(),
                    latest_main_reply: context.latest_main_reply.as_deref(),
                    previous_handoff_task: context.previous_handoff_task(),
                    current_runtime_snapshot: current_snapshot,
                    contract: completion_report,
                },
            );
        let prepared = match preparation {
            PlanningPostTurnOfficialCompletionPreparation::Blocked(blocked) => {
                self.parallel_mode_turn_service
                    .mark_official_completion_failed(
                        &request.workspace_directory,
                        &blocked.failure_detail,
                    );
                event_log::emit_lazy("official_completion_refresh_blocked", || {
                    post_turn_event_detail(
                        context.log_context(request),
                        "official_completion",
                        "planning_workspace_unavailable",
                        Some("block_slot_finalization"),
                        Some(&blocked.failure_snapshot),
                        [
                            (
                                "planning_workspace_directory",
                                json!(planning_workspace_directory),
                            ),
                            ("failure_detail", json!(blocked.failure_detail)),
                        ],
                    )
                });
                self.record_planning_worker_failure(
                    PlanningWorkerStatus::RefreshFailed,
                    &blocked.failure_detail,
                    &blocked.failure_snapshot,
                );
                return OfficialCompletionRefreshOutcome {
                    runtime_snapshot: blocked.failure_snapshot,
                    runtime_notices: Vec::new(),
                };
            }
            PlanningPostTurnOfficialCompletionPreparation::Ready(prepared) => prepared,
        };

        // Supervisor may emit a runtime notice when the reservation moves into
        // refreshing; preserve that notice so shell status reflects the state change.
        let mut runtime_notices = Vec::new();
        if let Some(notice) = self
            .parallel_mode_turn_service
            .mark_official_completion_refreshing(&request.workspace_directory)
        {
            runtime_notices.push(notice);
        }
        event_log::emit_lazy("official_completion_refresh_started", || {
            post_turn_event_detail(
                context.log_context(request),
                "official_completion",
                "refresh_started",
                Some("run_worker"),
                Some(prepared.planning_workspace_snapshot()),
                [
                    (
                        "planning_workspace_directory",
                        json!(planning_workspace_directory),
                    ),
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
                    ("refresh_order", json!(prepared.refresh_order())),
                    (
                        "worker_prompt_chars",
                        json!(prepared.worker_prompt().chars().count()),
                    ),
                ],
            )
        });
        self.record_planning_worker_running(
            PlanningWorkerStatus::RefreshRunning,
            "official-refresh",
            prepared.worker_prompt().to_string(),
        );

        // Worker orchestration owns filesystem mutation and validation; this adapter
        // only sends the contract and interprets the outcome for TUI state.
        let worker_outcome = self
            .planning_feature
            .worker()
            .refresh_prepared_official_completion(prepared.as_ref());
        let outcome = match worker_outcome {
            Ok(outcome) => outcome,
            Err(error) => {
                // Execution failure leaves the official completion reservation blocked
                // until an operator or later recovery path repairs planning state.
                let detail = format!("official completion refresh failed: {error}");
                self.parallel_mode_turn_service
                    .mark_official_completion_failed(&request.workspace_directory, &detail);
                let failure_snapshot = prepared
                    .planning_workspace_snapshot()
                    .with_auto_follow_pause_reason(detail.clone());
                event_log::emit_lazy("official_completion_refresh_failed", || {
                    post_turn_event_detail(
                        context.log_context(request),
                        "official_completion",
                        "worker_failed",
                        Some("block_slot_finalization"),
                        Some(&failure_snapshot),
                        [
                            (
                                "planning_workspace_directory",
                                json!(planning_workspace_directory),
                            ),
                            ("refresh_order", json!(completion_report.refresh_order)),
                            ("error", json!(error.to_string())),
                        ],
                    )
                });
                self.record_planning_worker_failure(
                    PlanningWorkerStatus::RefreshFailed,
                    &detail,
                    &failure_snapshot,
                );
                return OfficialCompletionRefreshOutcome {
                    runtime_snapshot: failure_snapshot,
                    runtime_notices,
                };
            }
        };

        self.record_planning_worker_outcome(PlanningWorkerStatus::RefreshSucceeded, &outcome);
        // Outcome snapshot may still carry a repair request; keep it mutable so hidden
        // repair and repeated-head checks can refine the final decision snapshot.
        let mut runtime_snapshot = outcome.runtime_snapshot.clone();
        event_log::emit_lazy("official_completion_refresh_succeeded", || {
            post_turn_event_detail(
                context.log_context(request),
                "official_completion",
                "worker_succeeded",
                Some("apply_outcome"),
                Some(&runtime_snapshot),
                [
                    (
                        "planning_workspace_directory",
                        json!(planning_workspace_directory),
                    ),
                    ("refresh_order", json!(completion_report.refresh_order)),
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

        // Official refresh can discover broken task authority. Reuse the hidden repair
        // loop, but if it cannot resolve the issue, convert the snapshot into the
        // official-completion block reason rather than letting the slot continue.
        if let Some(repair_request) = outcome.repair_request.as_ref() {
            let repair_outcome = self.run_hidden_planning_repairs(
                context.thread_id.as_str(),
                planning_workspace_directory,
                &request.completed_turn_id,
                repair_request,
                context.previous_handoff_task(),
            );
            runtime_snapshot = if repair_outcome.resolved {
                repair_outcome.runtime_snapshot
            } else {
                let repair_block = self
                    .planning_feature
                    .worker()
                    .block_unresolved_post_turn_official_completion_repair(
                        PlanningPostTurnOfficialCompletionRepairBlockRequest {
                            runtime_snapshot: &repair_outcome.runtime_snapshot,
                        },
                    );
                event_log::emit_lazy("official_completion_repair_unresolved", || {
                    post_turn_event_detail(
                        context.log_context(request),
                        "repair",
                        "unresolved_after_official_completion",
                        Some("block_slot_finalization"),
                        Some(&repair_outcome.runtime_snapshot),
                        [
                            (
                                "planning_workspace_directory",
                                json!(planning_workspace_directory),
                            ),
                            ("refresh_order", json!(completion_report.refresh_order)),
                            (
                                "repair_failure_summary",
                                json!(repair_request.failure_summary.as_str()),
                            ),
                            ("invalid_reason", json!(repair_block.failure_detail)),
                        ],
                    )
                });
                repair_block.runtime_snapshot
            };
        }

        let finalization = self
            .planning_feature
            .worker()
            .finalize_post_turn_official_completion_refresh(
                PlanningPostTurnOfficialCompletionFinalizationRequest {
                    planning_workspace_directory,
                    previous_handoff_task: context.previous_handoff_task(),
                    previous_runtime_snapshot: prepared.planning_workspace_snapshot(),
                    refreshed_runtime_snapshot: &runtime_snapshot,
                    worker_summary: outcome.worker_summary.as_deref(),
                },
            );
        runtime_snapshot = finalization.runtime_snapshot;
        if let Some(detail) = finalization.repeated_queue_head_detail.as_ref() {
            event_log::emit_lazy("official_completion_paused_repeated_queue_head", || {
                post_turn_event_detail(
                    context.log_context(request),
                    "official_completion",
                    "repeated_queue_head_guard",
                    Some("pause_auto_follow"),
                    Some(&runtime_snapshot),
                    [
                        (
                            "planning_workspace_directory",
                            json!(planning_workspace_directory),
                        ),
                        ("refresh_order", json!(completion_report.refresh_order)),
                        ("pause_reason", json!(detail.as_str())),
                    ],
                )
            });
        }
        if let Some(failure_detail) = finalization.blocked_failure_detail.as_ref() {
            self.parallel_mode_turn_service
                .mark_official_completion_failed(&request.workspace_directory, failure_detail);
            event_log::emit_lazy("official_completion_refresh_blocked", || {
                post_turn_event_detail(
                    context.log_context(request),
                    "official_completion",
                    "finalization_blocked",
                    Some("block_slot_finalization"),
                    Some(&runtime_snapshot),
                    [
                        (
                            "planning_workspace_directory",
                            json!(planning_workspace_directory),
                        ),
                        ("refresh_order", json!(completion_report.refresh_order)),
                        ("failure_detail", json!(failure_detail)),
                    ],
                )
            });
            self.record_planning_worker_failure(
                PlanningWorkerStatus::RefreshFailed,
                failure_detail,
                &runtime_snapshot,
            );
            return OfficialCompletionRefreshOutcome {
                runtime_snapshot,
                runtime_notices,
            };
        }

        // Success copy becomes the supervisor reservation finalization detail. Prefer
        // worker summary when available because it names the ledger refresh result.
        let authority_refresh_outcome = finalization
            .authority_refresh_outcome
            .expect("successful official completion finalization should carry success copy");
        runtime_notices.extend(
            self.parallel_mode_turn_service
                .finalize_official_completion_success(
                    &request.workspace_directory,
                    &authority_refresh_outcome,
                ),
        );
        event_log::emit_lazy("official_completion_refresh_finalized", || {
            post_turn_event_detail(
                context.log_context(request),
                "official_completion",
                "finalized",
                Some("finalize_slot"),
                Some(&runtime_snapshot),
                [
                    (
                        "planning_workspace_directory",
                        json!(planning_workspace_directory),
                    ),
                    ("refresh_order", json!(completion_report.refresh_order)),
                    (
                        "authority_refresh_outcome_chars",
                        json!(authority_refresh_outcome.chars().count()),
                    ),
                    ("runtime_notices_count", json!(runtime_notices.len())),
                ],
            )
        });

        OfficialCompletionRefreshOutcome {
            runtime_snapshot,
            runtime_notices,
        }
    }
}
