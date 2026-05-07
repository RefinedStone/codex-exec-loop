// Parallel slot completion report is the domain contract captured by the supervisor
// when a worker finishes its assigned task.
use crate::application::service::parallel_mode::ParallelModeOfficialCompletionReport;
// Official completion refresh turns that report back into planning ledger state and
// a runtime snapshot the TUI can use for follow-up gating.
use crate::application::service::planning::{
    PlanningOfficialCompletionRefreshRequest, PlanningRuntimeSnapshot,
    PlanningRuntimeWorkspaceStatus,
};
use crate::diagnostics::event_log;
use serde_json::json;

// The refresh path reads conversation context and reports progress through the same
// planner worker panel used by builtin queue refresh and hidden repair.
use super::super::super::{ConversationViewModel, PlannerWorkerStatus};
// Repeated-head detection is shared with builtin refresh so official completion
// cannot requeue a slot task that failed to advance planning.
use super::logging::post_turn_event_detail;
use super::queue_head_detail::repeated_queue_head_detail;
// The parent post-turn module owns the failure constant and DTOs; this file owns
// only the official-completion branch of the executor.
use super::{
    OFFICIAL_COMPLETION_REFRESH_FAILURE_BLOCK_REASON, OfficialCompletionRefreshOutcome,
    PostTurnEvaluationExecutor, PostTurnEvaluationRequest,
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
        // Conversation supplies the latest committed agent reply and current planning snapshot.
        conversation: &ConversationViewModel,
        // Request anchors the slot workspace and queued turn id used by the supervisor.
        request: &PostTurnEvaluationRequest,
    ) -> Option<ParallelModeOfficialCompletionReport> {
        // Prefer the committed transcript reply over the report fallback so the
        // supervisor sees exactly what the operator saw in the slot session.
        let latest_main_reply = conversation
            .latest_agent_message_text()
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
                // official-completion report; the current planning snapshot is
                // still the most truthful state for the panel.
                event_log::emit_lazy("official_completion_capture_failed", || {
                    post_turn_event_detail(
                        conversation,
                        request,
                        "official_completion",
                        "capture_failed",
                        Some("skip_official_refresh"),
                        Some(&conversation.planning_runtime_snapshot),
                        [
                            ("error", json!(error.to_string())),
                            (
                                "changed_planning_file_count",
                                json!(request.changed_planning_file_paths.len()),
                            ),
                        ],
                    )
                });
                self.record_planner_worker_failure(
                    PlannerWorkerStatus::RefreshFailed,
                    &format!("parallel completion capture failed: {error}"),
                    &conversation.planning_runtime_snapshot,
                );
                None
            }
        }
    }

    /*
     * Run the official completion ledger refresh. Unlike builtin next-task refresh,
     * this path is driven by a completed parallel slot contract and must update the
     * supervisor reservation state before returning a snapshot to post-turn action
     * selection.
     */
    pub(super) fn run_official_completion_refresh(
        &mut self,
        // Source conversation supplies transcript context and previous handoff data.
        conversation: &ConversationViewModel,
        // Original post-turn request identifies the slot workspace and root turn id.
        request: &PostTurnEvaluationRequest,
        // Planning workspace may differ from the slot workspace when supervisor state is external.
        planning_workspace_directory: &str,
        // Current snapshot is reused when the planning workspace is the active turn workspace.
        current_snapshot: &PlanningRuntimeSnapshot,
        // Completion report is the official contract captured before this refresh.
        completion_report: &ParallelModeOfficialCompletionReport,
    ) -> OfficialCompletionRefreshOutcome {
        // Load the planning authority that the refresh worker will mutate. Reuse the
        // in-memory snapshot when possible so reconciliation done earlier in this
        // post-turn pass is not discarded.
        let planning_workspace_snapshot =
            if planning_workspace_directory == request.workspace_directory {
                current_snapshot.clone()
            } else {
                self.planning
                    .runtime
                    .load_runtime_snapshot_or_invalid(planning_workspace_directory)
            };

        // Official refresh cannot proceed without an initialized planning workspace;
        // mark the supervisor reservation failed and return a snapshot that blocks
        // follow-up from reusing the completed slot.
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
            event_log::emit_lazy("official_completion_refresh_blocked", || {
                post_turn_event_detail(
                    conversation,
                    request,
                    "official_completion",
                    "planning_workspace_unavailable",
                    Some("block_slot_finalization"),
                    Some(&failure_snapshot),
                    [
                        (
                            "planning_workspace_directory",
                            json!(planning_workspace_directory),
                        ),
                        ("failure_detail", json!(failure_detail)),
                    ],
                )
            });
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

        // Supervisor may emit a runtime notice when the reservation moves into
        // refreshing; preserve that notice so shell status reflects the state change.
        let mut runtime_notices = Vec::new();
        if let Some(notice) = self
            .parallel_mode_turn_service
            .mark_official_completion_refreshing(&request.workspace_directory)
        {
            runtime_notices.push(notice);
        }
        // The worker prompt needs the latest user and agent context. If the transcript
        // lacks a committed agent reply, fall back to the captured completion summary.
        let latest_main_reply = conversation
            .latest_agent_message_text()
            .map(str::trim)
            .filter(|message| !message.is_empty())
            .unwrap_or(completion_report.completion.final_response_summary.as_str());
        // Request joins three contexts: planning workspace authority, transcript
        // context, and the parallel completion contract.
        let worker_request = PlanningOfficialCompletionRefreshRequest {
            workspace_directory: planning_workspace_directory,
            parent_thread_id: Some(conversation.thread_id.as_str())
                .filter(|thread_id| !thread_id.trim().is_empty()),
            latest_user_message: conversation.latest_user_message_text(),
            latest_main_reply,
            previous_handoff_task: conversation.last_planning_task_handoff(),
            contract: completion_report,
        };
        // Record the exact prompt before execution so a failed worker run still leaves
        // enough state in the planner panel for operator recovery.
        let worker_prompt = self
            .planning
            .worker
            .render_official_completion_refresh_prompt(&worker_request);
        event_log::emit_lazy("official_completion_refresh_started", || {
            post_turn_event_detail(
                conversation,
                request,
                "official_completion",
                "refresh_started",
                Some("run_worker"),
                Some(&planning_workspace_snapshot),
                [
                    (
                        "planning_workspace_directory",
                        json!(planning_workspace_directory),
                    ),
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
                    ("refresh_order", json!(completion_report.refresh_order)),
                    ("worker_prompt_chars", json!(worker_prompt.chars().count())),
                ],
            )
        });
        self.record_planner_worker_running(
            PlannerWorkerStatus::RefreshRunning,
            "official-refresh",
            worker_prompt,
        );

        // Worker orchestration owns filesystem mutation and validation; this adapter
        // only sends the contract and interprets the outcome for TUI state.
        let worker_outcome = self
            .planning
            .worker
            .refresh_queue_from_official_completion(worker_request);
        let outcome = match worker_outcome {
            Ok(outcome) => outcome,
            Err(error) => {
                // Execution failure leaves the official completion reservation blocked
                // until an operator or later recovery path repairs planning state.
                let detail = format!("official completion refresh failed: {error}");
                self.parallel_mode_turn_service
                    .mark_official_completion_failed(&request.workspace_directory, &detail);
                let failure_snapshot =
                    official_completion_failure_snapshot(&planning_workspace_snapshot, &detail);
                event_log::emit_lazy("official_completion_refresh_failed", || {
                    post_turn_event_detail(
                        conversation,
                        request,
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
        // Outcome snapshot may still carry a repair request; keep it mutable so hidden
        // repair and repeated-head checks can refine the final decision snapshot.
        let mut runtime_snapshot = outcome.runtime_snapshot.clone();
        event_log::emit_lazy("official_completion_refresh_succeeded", || {
            post_turn_event_detail(
                conversation,
                request,
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
                &conversation.thread_id,
                planning_workspace_directory,
                &request.completed_turn_id,
                repair_request,
                conversation.last_planning_task_handoff(),
            );
            runtime_snapshot = if repair_outcome.resolved {
                repair_outcome.runtime_snapshot
            } else {
                event_log::emit_lazy("official_completion_repair_unresolved", || {
                    post_turn_event_detail(
                        conversation,
                        request,
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
                            (
                                "invalid_reason",
                                json!(OFFICIAL_COMPLETION_REFRESH_FAILURE_BLOCK_REASON),
                            ),
                        ],
                    )
                });
                official_completion_failure_snapshot(
                    &repair_outcome.runtime_snapshot,
                    OFFICIAL_COMPLETION_REFRESH_FAILURE_BLOCK_REASON,
                )
            };
        }

        // A successful refresh must also advance beyond the handed-off task. If the
        // same queue head remains unchanged, pause follow-up and surface the detail.
        if let Some(detail) = repeated_queue_head_detail(
            conversation.last_planning_task_handoff(),
            &planning_workspace_snapshot,
            &runtime_snapshot,
        ) {
            event_log::emit_lazy("official_completion_paused_repeated_queue_head", || {
                post_turn_event_detail(
                    conversation,
                    request,
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
            runtime_snapshot = runtime_snapshot.with_auto_followup_pause_reason(detail);
        }

        // Any snapshot that blocks auto-follow also blocks slot finalization. Mark
        // the supervisor reservation failed before returning to the post-turn reducer.
        if runtime_snapshot.blocks_auto_followup() {
            let failure_detail = runtime_snapshot
                .preview_detail()
                .unwrap_or(OFFICIAL_COMPLETION_REFRESH_FAILURE_BLOCK_REASON);
            self.parallel_mode_turn_service
                .mark_official_completion_failed(&request.workspace_directory, failure_detail);
            let failure_snapshot =
                official_completion_failure_snapshot(&runtime_snapshot, failure_detail);
            event_log::emit_lazy("official_completion_refresh_blocked", || {
                post_turn_event_detail(
                    conversation,
                    request,
                    "official_completion",
                    "finalization_blocked",
                    Some("block_slot_finalization"),
                    Some(&failure_snapshot),
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

        // Success copy becomes the supervisor reservation finalization detail. Prefer
        // worker summary when available because it names the ledger refresh result.
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
        event_log::emit_lazy("official_completion_refresh_finalized", || {
            post_turn_event_detail(
                conversation,
                request,
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

// Convert the latest known planning snapshot into a blocking snapshot that explains
// why official completion could not safely finalize. Keeping the existing snapshot
// preserves queue/proposal context for panels while adding the pause reason.
fn official_completion_failure_snapshot(
    current_snapshot: &PlanningRuntimeSnapshot,
    failure_detail: &str,
) -> PlanningRuntimeSnapshot {
    // Empty details collapse to the shared official-completion failure copy so every
    // failure snapshot has an operator-facing pause reason.
    let detail = if failure_detail.trim().is_empty() {
        OFFICIAL_COMPLETION_REFRESH_FAILURE_BLOCK_REASON
    } else {
        failure_detail
    };
    current_snapshot.with_auto_followup_pause_reason(detail.to_string())
}
