// Repair loop는 application planning service의 worker 계약을 호출하는 post-turn adapter 경계다.
// 여기서 만드는 `PlanningLedgerRepairRequest`가 hidden planner worker prompt의 입력이고,
// worker 결과는 다시 TUI가 판단할 `PlanningRuntimeSnapshot`으로 돌아온다.
use crate::application::service::planning::{
    PlanningLedgerRepairRequest, PlanningRepairRequest, PlanningRepairRetryReason,
    PlanningTaskHandoff,
};
use crate::diagnostics::event_log;
use serde_json::json;

// Repair 진행 상태는 TUI의 planner worker panel에 남는다. 사용자는 hidden prompt를
// 직접 조작하지 않지만, panel status가 실행/성공/실패와 마지막 prompt를 추적한다.
use super::super::super::PlannerWorkerStatus;
// 이 파일은 post-turn executor의 retry loop만 분리한다. 최대 시도 횟수와 반환 DTO는
// parent module이 소유해 official completion과 normal post-turn path가 같은 repair contract를 쓴다.
use super::logging::{PostTurnWorkerLogContext, post_turn_worker_event_detail};
use super::{
    HiddenPlanningRepairOutcome, MAX_PLANNING_REPAIR_ATTEMPTS, PostTurnEvaluationExecutor,
};

// Post-turn evaluation 중 planning state가 깨졌을 때 사용자 prompt를 띄우기 전에 내부
// worker로 복구해 보는 경로다. 실패해도 마지막 runtime snapshot을 보존해 caller가
// auto-follow pause와 panel copy를 같은 planning state 기준으로 만들 수 있게 한다.
impl PostTurnEvaluationExecutor {
    // `run_hidden_planning_repairs`는 invalid planning runtime을 자동으로 고치기 위한 제한된
    // retry loop다. 성공하면 resolved=true와 최신 snapshot을 돌려 auto-follow 평가가 계속
    // 진행되고, 실패하면 resolved=false로 caller가 block reason을 유지한다.
    pub(super) fn run_hidden_planning_repairs(
        &mut self,
        // Thread id ties hidden repair attempts back to the visible conversation without logging prompt text.
        thread_id: &str,
        // Runtime snapshot load와 worker prompt의 기준 workspace다. Post-turn, official
        // completion, builtin refresh repair가 모두 이 같은 filesystem boundary를 공유한다.
        workspace_directory: &str,
        // Repair가 어느 user/agent turn에서 파생됐는지 ledger와 prompt에 묶는 trace id다.
        root_turn_id: &str,
        // 처음 발견된 planning 오류와 복구 목표다. Retry가 필요하면 worker outcome의 새 request로 좁혀진다.
        repair_request: &PlanningRepairRequest,
        // 이전 handoff task는 repair가 queue-driven 흐름에서 어떤 task context를 보존해야 하는지 알려 준다.
        previous_handoff_task: Option<&PlanningTaskHandoff>,
    ) -> HiddenPlanningRepairOutcome {
        // 첫 snapshot은 repair 전 현재 runtime state다. Worker 실패나 attempt exhaustion 때
        // 이 값 또는 마지막 worker outcome snapshot을 caller에게 돌려준다.
        let mut runtime_snapshot = self
            .planning
            .runtime
            .load_runtime_snapshot_or_invalid(workspace_directory);
        let log_context =
            PostTurnWorkerLogContext::new(thread_id, root_turn_id, workspace_directory);
        // next_request는 retry마다 바뀔 수 있는 repair instruction이다.
        let mut next_request = repair_request.clone();
        // 첫 시도에는 retry reason이 없다. 두 번째부터는 이전 attempt가 왜 충분하지 않았는지 prompt에 싣는다.
        let mut next_retry_reason = None;

        // Repair는 무한 루프가 아니다. Planning 파일을 worker가 계속 흔들어도 post-turn
        // evaluation thread가 오래 붙잡히지 않도록 작은 retry budget을 둔다.
        for attempt_number in 1..=MAX_PLANNING_REPAIR_ATTEMPTS {
            // worker_request는 이번 attempt의 완전한 prompt context다. attempt/max/retry
            // reason을 함께 싣는 이유는 worker가 "몇 번째 복구이며 왜 반복되는지" 알아야 하기 때문이다.
            let worker_request = PlanningLedgerRepairRequest {
                workspace_directory,
                root_thread_id: Some(thread_id).filter(|thread_id| !thread_id.trim().is_empty()),
                root_turn_id,
                // Request는 같은 attempt 안에서 prompt 렌더링과 worker 실행이 공유하는 immutable input이다.
                repair_request: &next_request,
                previous_handoff_task,
                attempt_number,
                // max_attempts는 prompt가 남은 기회를 operator/worker copy에 반영하게 한다.
                max_attempts: MAX_PLANNING_REPAIR_ATTEMPTS,
                // Retry reason은 "바뀌었지만 여전히 invalid"와 "아예 바뀌지 않음"을 구분한다.
                retry_reason: next_retry_reason,
            };
            // Prompt를 먼저 렌더링해 panel에 기록한다. Worker 실행이 실패해도 어떤 repair
            // 지시를 보냈는지 TUI/debug state가 남아 후처리 실패를 추적할 수 있다.
            let worker_prompt = self
                .planning
                .worker
                .render_repair_task_authority_prompt(&worker_request);
            event_log::emit_lazy("planner_repair_attempt_started", || {
                post_turn_worker_event_detail(
                    log_context,
                    "repair",
                    "attempt_started",
                    Some("run_worker"),
                    Some(&runtime_snapshot),
                    [
                        ("attempt_number", json!(attempt_number)),
                        ("max_attempts", json!(MAX_PLANNING_REPAIR_ATTEMPTS)),
                        (
                            "retry_reason",
                            json!(next_retry_reason.map(|reason| format!("{:?}", reason))),
                        ),
                        (
                            "has_previous_handoff",
                            json!(previous_handoff_task.is_some()),
                        ),
                        ("worker_prompt_chars", json!(worker_prompt.chars().count())),
                    ],
                )
            });
            self.record_planner_worker_running(
                // RepairRunning은 hidden repair가 planner worker panel을 차지한다는 신호다.
                PlannerWorkerStatus::RepairRunning,
                "repair",
                worker_prompt,
            );
            // 실제 repair는 application service boundary를 넘어 worker orchestration으로 들어간다.
            // 이 adapter는 prompt 계약과 결과 해석만 담당하고 파일 수정/재검증은 service가 수행한다.
            let worker_outcome = self.planning.worker.repair_task_authority(worker_request);

            // Worker 호출 오류는 retry 대상이 아니다. Prompt 실행 자체가 실패했으므로 같은
            // request를 반복하지 않고 panel에 실패를 기록한 뒤 현재 snapshot으로 빠져나간다.
            let outcome = match worker_outcome {
                Ok(outcome) => outcome,
                Err(error) => {
                    // Attempt 번호를 detail에 넣어 panel/log가 어느 시점의 실패인지 설명하게 한다.
                    let detail = format!(
                        "planner repair attempt {attempt_number}/{} failed: {error}",
                        MAX_PLANNING_REPAIR_ATTEMPTS
                    );
                    self.record_planner_worker_failure(
                        PlannerWorkerStatus::RepairFailed,
                        &detail,
                        &runtime_snapshot,
                    );
                    event_log::emit_lazy("planner_repair_attempt_failed", || {
                        post_turn_worker_event_detail(
                            log_context,
                            "repair",
                            "attempt_failed",
                            Some("abort"),
                            Some(&runtime_snapshot),
                            [
                                ("attempt_number", json!(attempt_number)),
                                ("max_attempts", json!(MAX_PLANNING_REPAIR_ATTEMPTS)),
                                ("error", json!(error.to_string())),
                            ],
                        )
                    });
                    // resolved=false는 repair path가 planning runtime을 신뢰 가능한 상태로 만들지 못했다는 신호다.
                    return HiddenPlanningRepairOutcome {
                        runtime_snapshot,
                        resolved: false,
                    };
                }
            };

            // Worker가 정상 종료되면 panel에는 성공 outcome을 기록하고 이후 판단은 outcome snapshot 기준으로 한다.
            self.record_planner_worker_outcome(PlannerWorkerStatus::RepairSucceeded, &outcome);
            runtime_snapshot = outcome.runtime_snapshot.clone();
            event_log::emit_lazy("planner_repair_attempt_succeeded", || {
                post_turn_worker_event_detail(
                    log_context,
                    "repair",
                    "attempt_succeeded",
                    if outcome.repair_request.is_some() {
                        Some("continue_repair")
                    } else {
                        Some("resolved")
                    },
                    Some(&runtime_snapshot),
                    [
                        ("attempt_number", json!(attempt_number)),
                        (
                            "task_authority_changed",
                            json!(outcome.task_authority_changed),
                        ),
                        ("repair_requested", json!(outcome.repair_request.is_some())),
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

            // repair_request가 None이면 service 재검증 결과 더 고칠 task authority가 없다는 뜻이다.
            let Some(repair_request) = outcome.repair_request else {
                event_log::emit_lazy("planner_repair_completed", || {
                    post_turn_worker_event_detail(
                        log_context,
                        "repair",
                        "completed",
                        Some("resolved"),
                        Some(&runtime_snapshot),
                        [("attempt_number", json!(attempt_number))],
                    )
                });
                return HiddenPlanningRepairOutcome {
                    runtime_snapshot,
                    resolved: true,
                };
            };

            // 새 repair_request가 왔다는 것은 runtime이 아직 invalid라는 뜻이다. 마지막 attempt면 실패로 마감한다.
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
                event_log::emit_lazy("planner_repair_exhausted", || {
                    post_turn_worker_event_detail(
                        log_context,
                        "repair",
                        "exhausted",
                        Some("block_auto_follow"),
                        Some(&runtime_snapshot),
                        [
                            ("attempt_number", json!(attempt_number)),
                            ("max_attempts", json!(MAX_PLANNING_REPAIR_ATTEMPTS)),
                            (
                                "repair_failure_summary",
                                json!(repair_request.failure_summary.as_str()),
                            ),
                        ],
                    )
                });
                // Exhausted failure는 worker가 실행됐지만 제한 횟수 안에 valid state를 만들지 못했다는 신호다.
                return HiddenPlanningRepairOutcome {
                    runtime_snapshot,
                    resolved: false,
                };
            }

            // 다음 retry prompt에는 이전 attempt가 무엇을 바꿨는지 요약한 reason을 넣는다.
            // "변경됐지만 여전히 invalid"와 "변경 없음"은 worker 지시가 달라야 한다.
            next_retry_reason = Some(if outcome.task_authority_changed {
                PlanningRepairRetryReason::TaskAuthorityStillInvalid
            } else {
                PlanningRepairRetryReason::TaskAuthorityUnchanged
            });
            event_log::emit_lazy("planner_repair_retrying", || {
                post_turn_worker_event_detail(
                    log_context,
                    "repair",
                    "retrying",
                    Some("retry"),
                    Some(&runtime_snapshot),
                    [
                        ("attempt_number", json!(attempt_number)),
                        (
                            "retry_reason",
                            json!(next_retry_reason.map(|reason| format!("{:?}", reason))),
                        ),
                        (
                            "repair_failure_summary",
                            json!(repair_request.failure_summary.as_str()),
                        ),
                    ],
                )
            });
            // Service가 반환한 새 request를 소유값으로 저장해 다음 prompt가 최신 validation 실패를 겨냥하게 한다.
            next_request = repair_request;
        }

        // 상수 변경으로 loop가 실행되지 않을 때의 보수적 fallback이다. 현재는 도달하지 않는다.
        HiddenPlanningRepairOutcome {
            runtime_snapshot,
            resolved: false,
        }
    }
}
