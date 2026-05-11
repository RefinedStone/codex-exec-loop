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
