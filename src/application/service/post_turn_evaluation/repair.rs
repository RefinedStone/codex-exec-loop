// Repair loop는 post-turn adapter가 application planning facade로 넘기는 hidden repair 경계다.
// TUI는 attempt/result DTO를 panel과 event log로 렌더링하고, repair retry policy와 worker prompt 계약은
// planning application layer가 소유한다.
use crate::application::service::planning::{
    DEFAULT_POST_TURN_REPAIR_ATTEMPT_LIMIT, PlanningPostTurnRepairAttemptResult,
    PlanningPostTurnRepairOutcome, PlanningPostTurnRepairRequest, PlanningRepairRequest,
    PlanningTaskHandoff,
};
use crate::diagnostics::event_log;
use serde_json::json;

// Repair 진행 상태는 TUI의 planning worker panel에 남는다. 사용자는 hidden prompt를
// 직접 조작하지 않지만, panel status가 실행/성공/실패와 마지막 prompt를 추적한다.
// 이 파일은 post-turn executor의 repair branch만 분리한다. 최대 시도 횟수와 반환 DTO는
// planning application facade가 소유해 official completion과 normal post-turn path가 같은 repair contract를 쓴다.
use super::logging::{PostTurnWorkerLogContext, post_turn_worker_event_detail};
use super::{HiddenPlanningRepairOutcome, PlanningWorkerStatus, PostTurnEvaluationExecutor};

// Post-turn evaluation 중 planning state가 깨졌을 때 사용자 prompt를 띄우기 전에 내부
// worker로 복구해 보는 경로다. 실패해도 마지막 runtime projection을 보존해 caller가
// auto-follow pause와 panel copy를 같은 planning state 기준으로 만들 수 있게 한다.
impl PostTurnEvaluationExecutor {
    // `run_hidden_planning_repairs`는 invalid planning runtime을 자동으로 고치기 위한 제한된
    // retry loop다. 성공하면 resolved=true와 최신 snapshot을 돌려 auto-follow 평가가 계속
    // 진행되고, 실패하면 resolved=false로 caller가 block reason을 유지한다.
    pub(super) fn run_hidden_planning_repairs(
        &mut self,
        // Thread id ties hidden repair attempts back to the visible conversation without logging prompt text.
        thread_id: &str,
        // Runtime projection load와 worker prompt의 기준 workspace다. Post-turn, official
        // completion, planning queue refresh repair가 모두 이 같은 filesystem boundary를 공유한다.
        workspace_directory: &str,
        // Repair가 어느 user/agent turn에서 파생됐는지 ledger와 prompt에 묶는 trace id다.
        completed_turn_id: &str,
        // 처음 발견된 planning 오류와 복구 목표다. Retry가 필요하면 worker outcome의 새 request로 좁혀진다.
        repair_request: &PlanningRepairRequest,
        // 이전 handoff task는 repair가 queue-driven 흐름에서 어떤 task context를 보존해야 하는지 알려 준다.
        previous_handoff_task: Option<&PlanningTaskHandoff>,
    ) -> HiddenPlanningRepairOutcome {
        let log_context =
            PostTurnWorkerLogContext::new(thread_id, completed_turn_id, workspace_directory);
        let repair_outcome = self
            .planning_feature
            .worker
            .repair_post_turn_task_authority(PlanningPostTurnRepairRequest {
                workspace_directory,
                parent_thread_id: Some(thread_id).filter(|thread_id| !thread_id.trim().is_empty()),
                completed_turn_id,
                repair_request,
                previous_handoff_task,
                max_attempts: DEFAULT_POST_TURN_REPAIR_ATTEMPT_LIMIT,
            });
        self.apply_repair_outcome(
            log_context,
            previous_handoff_task.is_some(),
            &repair_outcome,
        );
        HiddenPlanningRepairOutcome {
            runtime_projection: repair_outcome.runtime_projection,
            resolved: repair_outcome.resolved,
        }
    }

    fn apply_repair_outcome(
        &mut self,
        log_context: PostTurnWorkerLogContext,
        has_previous_handoff: bool,
        repair_outcome: &PlanningPostTurnRepairOutcome,
    ) {
        for attempt in &repair_outcome.attempts {
            event_log::emit_lazy("planning_worker_repair_attempt_started", || {
                post_turn_worker_event_detail(
                    log_context,
                    "repair",
                    "attempt_started",
                    Some("run_worker"),
                    Some(&attempt.started_runtime_projection),
                    [
                        ("attempt_number", json!(attempt.attempt_number)),
                        ("max_attempts", json!(attempt.max_attempts)),
                        (
                            "retry_reason",
                            json!(attempt.retry_reason.map(|reason| format!("{:?}", reason))),
                        ),
                        ("has_previous_handoff", json!(has_previous_handoff)),
                        (
                            "worker_prompt_chars",
                            json!(attempt.worker_prompt.chars().count()),
                        ),
                    ],
                )
            });
            self.record_planning_worker_running(
                PlanningWorkerStatus::RepairRunning,
                "repair",
                attempt.worker_prompt.clone(),
            );

            match &attempt.result {
                PlanningPostTurnRepairAttemptResult::WorkerFailed { detail, error } => {
                    self.record_planning_worker_failure(
                        PlanningWorkerStatus::RepairFailed,
                        detail,
                        &attempt.started_runtime_projection,
                    );
                    event_log::emit_lazy("planning_worker_repair_attempt_failed", || {
                        post_turn_worker_event_detail(
                            log_context,
                            "repair",
                            "attempt_failed",
                            Some("abort"),
                            Some(&attempt.started_runtime_projection),
                            [
                                ("attempt_number", json!(attempt.attempt_number)),
                                ("max_attempts", json!(attempt.max_attempts)),
                                ("error", json!(error)),
                            ],
                        )
                    });
                }
                PlanningPostTurnRepairAttemptResult::WorkerSucceeded {
                    outcome,
                    next_repair_request,
                    next_retry_reason,
                    resolved,
                    exhausted,
                } => {
                    self.record_planning_worker_outcome(
                        PlanningWorkerStatus::RepairSucceeded,
                        outcome,
                    );
                    event_log::emit_lazy("planning_worker_repair_attempt_succeeded", || {
                        post_turn_worker_event_detail(
                            log_context,
                            "repair",
                            "attempt_succeeded",
                            if next_repair_request.is_some() {
                                Some("continue_repair")
                            } else {
                                Some("resolved")
                            },
                            Some(&outcome.runtime_projection),
                            [
                                ("attempt_number", json!(attempt.attempt_number)),
                                (
                                    "task_authority_changed",
                                    json!(outcome.task_authority_changed),
                                ),
                                ("repair_requested", json!(next_repair_request.is_some())),
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
                    if *resolved {
                        event_log::emit_lazy("planning_worker_repair_completed", || {
                            post_turn_worker_event_detail(
                                log_context,
                                "repair",
                                "completed",
                                Some("resolved"),
                                Some(&outcome.runtime_projection),
                                [("attempt_number", json!(attempt.attempt_number))],
                            )
                        });
                    } else if *exhausted {
                        let detail = format!(
                            "planning worker repair exhausted after {} attempts; the last accepted planning state was kept",
                            attempt.max_attempts
                        );
                        self.record_planning_worker_failure(
                            PlanningWorkerStatus::RepairFailed,
                            &detail,
                            &outcome.runtime_projection,
                        );
                        event_log::emit_lazy("planning_worker_repair_exhausted", || {
                            post_turn_worker_event_detail(
                                log_context,
                                "repair",
                                "exhausted",
                                Some("block_auto_follow"),
                                Some(&outcome.runtime_projection),
                                [
                                    ("attempt_number", json!(attempt.attempt_number)),
                                    ("max_attempts", json!(attempt.max_attempts)),
                                    (
                                        "repair_failure_summary",
                                        json!(
                                            next_repair_request
                                                .as_ref()
                                                .map(|request| request.failure_summary.as_str())
                                        ),
                                    ),
                                ],
                            )
                        });
                    } else {
                        event_log::emit_lazy("planning_worker_repair_retrying", || {
                            post_turn_worker_event_detail(
                                log_context,
                                "repair",
                                "retrying",
                                Some("retry"),
                                Some(&outcome.runtime_projection),
                                [
                                    ("attempt_number", json!(attempt.attempt_number)),
                                    (
                                        "retry_reason",
                                        json!(
                                            next_retry_reason.map(|reason| format!("{:?}", reason))
                                        ),
                                    ),
                                    (
                                        "repair_failure_summary",
                                        json!(
                                            next_repair_request
                                                .as_ref()
                                                .map(|request| request.failure_summary.as_str())
                                        ),
                                    ),
                                ],
                            )
                        });
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::adapter::outbound::db::SqlitePlanningAuthorityAdapter;
    use crate::adapter::outbound::filesystem::FilesystemPlanningWorkspaceAdapter;
    use crate::adapter::outbound::git::parallel_mode_runtime::GitParallelModeRuntimeAdapter;
    use crate::adapter::outbound::github::GithubAutomationAdapter;
    use crate::application::port::outbound::planning_authority_port::NoopPlanningAuthorityPort;
    use crate::application::port::outbound::planning_task_repository_port::NoopPlanningTaskRepositoryPort;
    use crate::application::port::outbound::planning_worker_port::NoopPlanningWorkerPort;
    use crate::application::service::parallel_mode::ParallelModeService;
    use crate::application::service::parallel_mode::turn::ParallelModeTurnService;
    use crate::application::service::planning::{
        PlanningPostTurnRepairAttempt, PlanningRepairRetryReason, PlanningRuntimeProjection,
        PlanningServices, PlanningWorkerRunOutcome,
    };
    use crate::application::service::post_turn_evaluation::PlanningWorkerPanelState;
    use crate::domain::planning::{PriorityQueueTask, TaskStatus};

    #[test]
    fn repair_worker_failure_records_failed_panel_state() {
        let mut executor = test_executor();
        let started_runtime_projection = PlanningRuntimeProjection::invalid("broken authority");
        let repair_outcome = PlanningPostTurnRepairOutcome {
            runtime_projection: started_runtime_projection.clone(),
            resolved: false,
            attempts: vec![PlanningPostTurnRepairAttempt {
                attempt_number: 1,
                max_attempts: 2,
                retry_reason: None,
                started_runtime_projection: started_runtime_projection.clone(),
                worker_prompt: "repair prompt".to_string(),
                result: PlanningPostTurnRepairAttemptResult::WorkerFailed {
                    detail: "repair worker failed before producing commands".to_string(),
                    error: "transport failed".to_string(),
                },
            }],
        };

        executor.apply_repair_outcome(log_context(), false, &repair_outcome);

        assert_eq!(
            executor.planning_worker_panel_state.status,
            PlanningWorkerStatus::RepairFailed
        );
        assert_eq!(
            executor
                .planning_worker_panel_state
                .last_operation_label
                .as_deref(),
            Some("repair")
        );
        assert_eq!(
            executor.planning_worker_panel_state.last_prompt.as_deref(),
            Some("repair prompt")
        );
        assert_eq!(
            executor.planning_worker_panel_state.last_summary.as_deref(),
            Some("repair worker failed before producing commands")
        );
        assert!(executor.planning_worker_panel_state.last_response.is_none());
        assert!(
            executor
                .planning_worker_panel_state
                .last_queue_summary
                .is_none()
        );
    }

    #[test]
    fn resolved_repair_success_keeps_accepted_summary_and_extra_notices() {
        let mut executor = test_executor();
        let runtime_projection = ready_projection("queue ready after repair");
        let repair_outcome = PlanningPostTurnRepairOutcome {
            runtime_projection: runtime_projection.clone(),
            resolved: true,
            attempts: vec![PlanningPostTurnRepairAttempt {
                attempt_number: 1,
                max_attempts: 2,
                retry_reason: None,
                started_runtime_projection: PlanningRuntimeProjection::invalid("stale ledger"),
                worker_prompt: "repair prompt".to_string(),
                result: PlanningPostTurnRepairAttemptResult::WorkerSucceeded {
                    outcome: Box::new(worker_outcome(
                        runtime_projection,
                        Some("accepted task authority repair"),
                        Some("raw worker response"),
                        Some("discarded stale candidate"),
                        vec![
                            "planning worker repair summary: accepted task authority repair",
                            "kept operator-edited task title",
                        ],
                        None,
                    )),
                    next_repair_request: None,
                    next_retry_reason: None,
                    resolved: true,
                    exhausted: false,
                },
            }],
        };

        executor.apply_repair_outcome(log_context(), true, &repair_outcome);

        assert_eq!(
            executor.planning_worker_panel_state.status,
            PlanningWorkerStatus::RepairSucceeded
        );
        assert_eq!(
            executor.planning_worker_panel_state.last_summary.as_deref(),
            Some("accepted task authority repair")
        );
        assert_eq!(
            executor
                .planning_worker_panel_state
                .last_rejected_summary
                .as_deref(),
            Some("discarded stale candidate")
        );
        assert_eq!(
            executor
                .planning_worker_panel_state
                .last_notice_detail
                .as_deref(),
            Some("kept operator-edited task title")
        );
        assert_eq!(
            executor
                .planning_worker_panel_state
                .last_response
                .as_deref(),
            Some("raw worker response")
        );
        assert_eq!(
            executor
                .planning_worker_panel_state
                .last_queue_summary
                .as_deref(),
            Some("queue head: Repair follow-up")
        );
    }

    #[test]
    fn retry_then_exhausted_repair_ends_with_blocking_failure_copy() {
        let mut executor = test_executor();
        let retry_projection = ready_projection("retry still has queue head");
        let exhausted_projection = ready_projection("last accepted state kept");
        let repair_outcome = PlanningPostTurnRepairOutcome {
            runtime_projection: exhausted_projection.clone(),
            resolved: false,
            attempts: vec![
                PlanningPostTurnRepairAttempt {
                    attempt_number: 1,
                    max_attempts: 2,
                    retry_reason: None,
                    started_runtime_projection: PlanningRuntimeProjection::invalid("first error"),
                    worker_prompt: "first repair prompt".to_string(),
                    result: PlanningPostTurnRepairAttemptResult::WorkerSucceeded {
                        outcome: Box::new(worker_outcome(
                            retry_projection,
                            Some("first repair accepted"),
                            Some("first raw response"),
                            None,
                            Vec::new(),
                            Some(repair_request("still invalid after first repair")),
                        )),
                        next_repair_request: Some(repair_request(
                            "still invalid after first repair",
                        )),
                        next_retry_reason: Some(
                            PlanningRepairRetryReason::TaskAuthorityStillInvalid,
                        ),
                        resolved: false,
                        exhausted: false,
                    },
                },
                PlanningPostTurnRepairAttempt {
                    attempt_number: 2,
                    max_attempts: 2,
                    retry_reason: Some(PlanningRepairRetryReason::TaskAuthorityStillInvalid),
                    started_runtime_projection: PlanningRuntimeProjection::invalid("second error"),
                    worker_prompt: "second repair prompt".to_string(),
                    result: PlanningPostTurnRepairAttemptResult::WorkerSucceeded {
                        outcome: Box::new(worker_outcome(
                            exhausted_projection,
                            Some("second repair accepted"),
                            Some("second raw response"),
                            None,
                            Vec::new(),
                            Some(repair_request("still invalid after second repair")),
                        )),
                        next_repair_request: Some(repair_request(
                            "still invalid after second repair",
                        )),
                        next_retry_reason: Some(
                            PlanningRepairRetryReason::TaskAuthorityStillInvalid,
                        ),
                        resolved: false,
                        exhausted: true,
                    },
                },
            ],
        };

        executor.apply_repair_outcome(log_context(), false, &repair_outcome);

        assert_eq!(
            executor.planning_worker_panel_state.status,
            PlanningWorkerStatus::RepairFailed
        );
        assert_eq!(
            executor.planning_worker_panel_state.last_prompt.as_deref(),
            Some("second repair prompt")
        );
        assert_eq!(
            executor.planning_worker_panel_state.last_summary.as_deref(),
            Some(
                "planning worker repair exhausted after 2 attempts; the last accepted planning state was kept"
            )
        );
        assert_eq!(
            executor
                .planning_worker_panel_state
                .last_queue_summary
                .as_deref(),
            Some("queue head: Repair follow-up")
        );
        assert!(executor.planning_worker_panel_state.last_response.is_none());
    }

    fn test_executor() -> PostTurnEvaluationExecutor {
        PostTurnEvaluationExecutor {
            planning_feature: PlanningServices::from_ports(
                Arc::new(FilesystemPlanningWorkspaceAdapter::new()),
                Arc::new(NoopPlanningAuthorityPort::default()),
                Arc::new(NoopPlanningTaskRepositoryPort),
                Arc::new(NoopPlanningWorkerPort),
            ),
            parallel_mode_turn_service: ParallelModeTurnService::new(ParallelModeService::new(
                Arc::new(SqlitePlanningAuthorityAdapter::new()),
                Arc::new(GithubAutomationAdapter::new()),
                Arc::new(GitParallelModeRuntimeAdapter::new()),
            )),
            planning_worker_panel_state: PlanningWorkerPanelState::default(),
        }
    }

    fn log_context() -> PostTurnWorkerLogContext<'static> {
        PostTurnWorkerLogContext::new("thread-1", "turn-1", "/tmp/workspace")
    }

    fn ready_projection(queue_summary: &str) -> PlanningRuntimeProjection {
        PlanningRuntimeProjection::ready(
            "Planning Context".to_string(),
            queue_summary.to_string(),
            Some(queue_task()),
        )
    }

    fn queue_task() -> PriorityQueueTask {
        PriorityQueueTask {
            rank: 1,
            task_id: "task-1".to_string(),
            direction_id: "general-workstream".to_string(),
            direction_title: "General workstream".to_string(),
            task_title: "Repair follow-up".to_string(),
            status: TaskStatus::Ready,
            combined_priority: 100,
            updated_at: "2026-05-12T00:00:00Z".to_string(),
            rank_reasons: vec!["status=ready".to_string()],
        }
    }

    fn worker_outcome(
        runtime_projection: PlanningRuntimeProjection,
        worker_summary: Option<&str>,
        worker_response: Option<&str>,
        rejected_summary: Option<&str>,
        notices: Vec<&str>,
        repair_request: Option<PlanningRepairRequest>,
    ) -> PlanningWorkerRunOutcome {
        PlanningWorkerRunOutcome {
            runtime_projection,
            notices: notices.into_iter().map(str::to_string).collect(),
            repair_request,
            worker_summary: worker_summary.map(str::to_string),
            worker_response: worker_response.map(str::to_string),
            rejected_summary: rejected_summary.map(str::to_string),
            task_authority_changed: true,
        }
    }

    fn repair_request(failure_summary: &str) -> PlanningRepairRequest {
        PlanningRepairRequest {
            failure_summary: failure_summary.to_string(),
            validation_errors: vec![failure_summary.to_string()],
            direction_authority_json: "{}".to_string(),
            accepted_task_authority_json: "{}".to_string(),
            accepted_queue_projection_json: "{}".to_string(),
            rejected_task_authority_json: Some("{}".to_string()),
            rejected_archive_path: Some("planning/rejected/result-output.md".to_string()),
        }
    }
}
