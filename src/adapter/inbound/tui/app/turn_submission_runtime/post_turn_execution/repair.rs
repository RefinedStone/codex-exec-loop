// 학습 주석: repair 루프는 application planning service의 worker 계약을 직접 호출합니다. 여기서 만드는
// `PlanningLedgerRepairRequest`가 숨은 planner worker prompt의 입력이고, worker 결과는 runtime snapshot으로 되돌아옵니다.
use crate::application::service::planning::{
    PlanningLedgerRepairRequest, PlanningRepairRequest, PlanningRepairRetryReason,
    PlanningTaskHandoff,
};

// 학습 주석: repair 진행 상태는 TUI의 planner worker panel에 남습니다. 사용자는 숨은 repair prompt를
// 직접 조작하지 않지만, panel status를 통해 repair 실행/성공/실패를 추적할 수 있습니다.
use super::super::super::PlannerWorkerStatus;
// 학습 주석: repair 모듈은 post-turn executor의 일부입니다. 최대 시도 횟수와 반환 DTO는 parent 모듈이
// 소유하고, 이 파일은 실제 retry loop만 분리해 post_turn_execution.rs의 큰 흐름을 줄입니다.
use super::{
    HiddenPlanningRepairOutcome, MAX_PLANNING_REPAIR_ATTEMPTS, PostTurnEvaluationExecutor,
};

// 학습 주석: 이 impl 확장은 post-turn evaluation 중 "planning state가 깨졌으나 사용자에게 별도 prompt를
// 띄우기 전에 내부 worker로 복구해 볼 수 있는" 경로를 담당합니다. 실패해도 마지막 runtime snapshot을 보존합니다.
impl PostTurnEvaluationExecutor {
    // 학습 주석: `run_hidden_planning_repairs`는 invalid planning runtime을 자동으로 고치기 위한 제한된
    // retry loop입니다. 성공하면 resolved=true와 최신 snapshot을 돌려 auto-follow 평가가 계속 진행되고,
    // 실패하면 resolved=false로 caller가 auto-follow를 멈추거나 block reason을 유지하게 합니다.
    pub(super) fn run_hidden_planning_repairs(
        &mut self,
        // 학습 주석: workspace_directory는 runtime snapshot 로드와 worker prompt의 기준 경로입니다.
        // post-turn request, official completion repair, builtin refresh repair가 모두 이 같은 경계를 공유합니다.
        workspace_directory: &str,
        // 학습 주석: root_turn_id는 repair가 어느 user/agent turn에서 파생됐는지 ledger와 prompt에 묶는 값입니다.
        // 숨은 worker가 만든 변경도 원래 턴의 후처리로 추적되어야 합니다.
        root_turn_id: &str,
        // 학습 주석: repair_request는 처음 발견된 planning 오류와 필요한 복구 목표를 담습니다. retry가
        // 필요하면 worker outcome의 새 request로 교체되어 다음 attempt prompt가 더 좁은 정보를 받습니다.
        repair_request: &PlanningRepairRequest,
        // 학습 주석: previous_handoff_task는 queue head나 이전 handoff와 repair 사이의 맥락을 이어 줍니다.
        // worker가 task authority를 고칠 때 "어떤 task 흐름을 보존해야 하는가"를 잃지 않게 합니다.
        previous_handoff_task: Option<&PlanningTaskHandoff>,
    ) -> HiddenPlanningRepairOutcome {
        // 학습 주석: 첫 snapshot은 repair 전 현재 runtime 상태입니다. worker 호출 자체가 실패하거나
        // attempt가 소진되면 이 값 또는 마지막 성공 worker가 낸 snapshot을 caller에게 돌려줍니다.
        let mut runtime_snapshot = self
            .planning
            .runtime
            .load_runtime_snapshot_or_invalid(workspace_directory);
        // 학습 주석: next_request는 retry마다 바뀔 수 있는 repair 지시입니다. 처음에는 caller가 준
        // request를 복제하고, worker가 "아직 더 고쳐야 함"을 반환하면 그 request로 다음 prompt를 만듭니다.
        let mut next_request = repair_request.clone();
        // 학습 주석: retry reason은 첫 시도에는 없습니다. 두 번째 시도부터는 task authority가 바뀌었는지
        // 여부를 알려 worker prompt가 "왜 다시 실행되는지"를 명시하게 합니다.
        let mut next_retry_reason = None;

        // 학습 주석: repair는 무한 루프가 아니라 parent constant로 제한합니다. planning 파일을 agent가
        // 계속 흔드는 상황에서도 post-turn evaluation thread가 오래 붙잡히지 않도록 작은 retry budget을 둡니다.
        for attempt_number in 1..=MAX_PLANNING_REPAIR_ATTEMPTS {
            // 학습 주석: worker_request는 이번 attempt의 완전한 prompt context입니다. attempt/max/retry reason을
            // 함께 싣는 이유는 worker output contract가 "이번이 몇 번째 복구이며 왜 반복되는지"를 알아야 하기 때문입니다.
            let worker_request = PlanningLedgerRepairRequest {
                workspace_directory,
                root_turn_id,
                // 학습 주석: request는 참조로 넘깁니다. worker prompt 렌더링과 실행이 같은 attempt 안에서만
                // 이 값을 읽고, 다음 attempt에서는 outcome이 준 소유 request로 교체합니다.
                repair_request: &next_request,
                previous_handoff_task,
                attempt_number,
                // 학습 주석: max_attempts를 request에 포함해 prompt가 남은 기회를 사용자/agent copy에 반영합니다.
                max_attempts: MAX_PLANNING_REPAIR_ATTEMPTS,
                // 학습 주석: retry_reason은 이전 attempt의 outcome 해석입니다. 첫 시도에는 None이고,
                // 이후에는 "바뀌었지만 여전히 invalid"와 "아예 바뀌지 않음"을 구분합니다.
                retry_reason: next_retry_reason,
            };
            // 학습 주석: prompt를 먼저 렌더링해 panel에 기록합니다. 실제 worker 실행이 실패해도 어떤
            // repair 지시를 보냈는지 TUI/debug state가 남아 있어 후처리 실패를 추적할 수 있습니다.
            let worker_prompt = self
                .planning
                .worker
                .render_repair_task_authority_prompt(&worker_request);
            self.record_planner_worker_running(
                // 학습 주석: RepairRunning은 hidden repair가 현재 panel을 차지한다는 신호입니다. label은
                // 짧게 `"repair"`로 고정해 다른 planner worker 작업과 구분합니다.
                PlannerWorkerStatus::RepairRunning,
                "repair",
                worker_prompt,
            );
            // 학습 주석: 실제 repair는 application service boundary를 넘어 worker orchestration으로 들어갑니다.
            // 이 함수는 prompt 계약/결과 해석만 담당하고, 파일 수정과 runtime 재검증은 service가 수행합니다.
            let worker_outcome = self.planning.worker.repair_task_authority(worker_request);

            // 학습 주석: worker 호출 오류는 retry 대상이 아닙니다. prompt 실행 자체가 실패했다는 뜻이라
            // 같은 request를 다시 보내기보다 panel에 실패를 기록하고 현재 snapshot으로 빠져나갑니다.
            let outcome = match worker_outcome {
                Ok(outcome) => outcome,
                Err(error) => {
                    // 학습 주석: detail에는 attempt 번호를 넣어 panel과 logs에서 어느 시점에 실패했는지
                    // 알 수 있게 합니다. failure record에는 마지막으로 알고 있던 runtime snapshot을 같이 실어 둡니다.
                    let detail = format!(
                        "planner repair attempt {attempt_number}/{} failed: {error}",
                        MAX_PLANNING_REPAIR_ATTEMPTS
                    );
                    self.record_planner_worker_failure(
                        PlannerWorkerStatus::RepairFailed,
                        &detail,
                        &runtime_snapshot,
                    );
                    // 학습 주석: resolved=false는 caller에게 "repair path가 planning runtime을 신뢰 가능한
                    // 상태로 만들지 못했다"는 신호입니다. snapshot은 UI가 현재 block reason을 계속 보여 주게 합니다.
                    return HiddenPlanningRepairOutcome {
                        runtime_snapshot,
                        resolved: false,
                    };
                }
            };

            // 학습 주석: worker가 정상 종료되면 panel에는 성공 outcome을 기록하고, 이후 판단은 outcome이
            // 반환한 runtime snapshot을 기준으로 합니다. repair가 일부만 성공해도 마지막 accepted state를 유지합니다.
            self.record_planner_worker_outcome(PlannerWorkerStatus::RepairSucceeded, &outcome);
            runtime_snapshot = outcome.runtime_snapshot.clone();

            // 학습 주석: repair_request가 None이면 service가 runtime을 다시 검증했고 더 고칠 task authority가
            // 없다는 뜻입니다. 이때는 resolved=true로 후속 auto-follow 평가가 최신 snapshot을 계속 사용할 수 있습니다.
            let Some(repair_request) = outcome.repair_request else {
                return HiddenPlanningRepairOutcome {
                    runtime_snapshot,
                    resolved: true,
                };
            };

            // 학습 주석: worker가 새 repair_request를 돌려줬다는 것은 아직 planning runtime이 invalid라는 뜻입니다.
            // 마지막 attempt라면 더 돌리지 않고 마지막 accepted snapshot을 남긴 채 실패로 마감합니다.
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
                // 학습 주석: exhausted failure는 "worker는 실행됐지만 제한 횟수 안에 valid 상태를 만들지
                // 못함"입니다. 호출자는 resolved=false를 보고 auto-follow를 보수적으로 멈춥니다.
                return HiddenPlanningRepairOutcome {
                    runtime_snapshot,
                    resolved: false,
                };
            }

            // 학습 주석: 다음 retry prompt에는 이전 attempt가 무엇을 바꿨는지 요약한 reason을 넣습니다.
            // 바뀌었는데 여전히 invalid인 경우와, worker output이 task authority를 바꾸지 못한 경우는 지시가 달라야 합니다.
            next_retry_reason = Some(if outcome.task_authority_changed {
                PlanningRepairRetryReason::TaskAuthorityStillInvalid
            } else {
                PlanningRepairRetryReason::TaskAuthorityUnchanged
            });
            // 학습 주석: service가 반환한 새 request를 소유값으로 저장해 다음 loop에서 참조로 넘깁니다. 이
            // handoff가 있어야 두 번째 prompt가 첫 번째 오류가 아니라 최신 validation 실패를 겨냥합니다.
            next_request = repair_request;
        }

        // 학습 주석: for 범위가 비정상적으로 비어 있거나 상수 변경으로 loop가 실행되지 않을 때의 보수적
        // fallback입니다. 현재 constant는 2라 도달하지 않지만, 반환 계약상 unresolved snapshot을 유지합니다.
        HiddenPlanningRepairOutcome {
            runtime_snapshot,
            resolved: false,
        }
    }
}
