// 학습 주석: planner worker panel은 application planning worker가 돌려준 snapshot/outcome을 TUI가 읽을 수
// 있는 마지막 관측 상태로 축약합니다. snapshot은 queue summary를 만들고, outcome은 worker prompt/response와
// repair 필요 여부를 전달합니다.
use crate::application::service::planning::{PlanningRuntimeSnapshot, PlanningWorkerRunOutcome};

// 학습 주석: `PlannerWorkerStatus`는 debug panel과 queue overlay가 같은 색/문구로 해석하는 UI status enum입니다.
// 여기서 성공/실패/실행 중 상태를 정하면 presentation layer는 별도 business rule 없이 표시만 합니다.
use super::super::super::PlannerWorkerStatus;
// 학습 주석: `PostTurnEvaluationExecutor`는 한 턴이 끝난 뒤 planning refresh, repair, proposal promotion,
// auto-follow decision을 수행합니다. 이 파일은 그 executor의 panel-state 기록 메서드만 따로 묶은 확장입니다.
use super::PostTurnEvaluationExecutor;

// 학습 주석: 이 impl 블록은 post-turn planning worker의 관측 가능성을 담당합니다. executor 내부의 실제
// planning 작업은 다른 메서드가 수행하고, 여기서는 "사용자에게 마지막으로 무엇이 일어났다고 보여 줄지"를 갱신합니다.
impl PostTurnEvaluationExecutor {
    // 학습 주석: worker 실행 시작을 기록합니다. refresh/repair worker를 호출하기 직전에 status와 prompt를 남겨
    // long-running post-turn evaluation 동안 debug panel이 "무슨 요청을 보내고 있는지"를 보여 줄 수 있게 합니다.
    pub(super) fn record_planner_worker_running(
        &mut self,
        // 학습 주석: status는 RefreshRunning 또는 RepairRunning처럼 호출자가 이미 결정한 실행 단계입니다.
        // panel state는 이 값을 그대로 사용해 refresh와 repair를 구분합니다.
        status: PlannerWorkerStatus,
        // 학습 주석: operation_label은 worker request의 종류를 짧게 설명하는 사람이 읽는 label입니다.
        // prompt/response가 길어도 패널 상단에서 실행 목적을 빠르게 확인하게 합니다.
        operation_label: &str,
        // 학습 주석: prompt는 planning worker로 보낸 실제 입력입니다. debug details가 켜졌을 때 worker 판단의
        // 근거를 추적할 수 있도록 마지막 요청 본문으로 보존합니다.
        prompt: String,
    ) {
        // 학습 주석: running 상태로 전환할 때 이전 결과성 필드는 모두 비웁니다. 그러지 않으면 새 실행 중인
        // worker 아래에 이전 response/rejection/error가 남아 사용자가 현재 실행 결과로 오해할 수 있습니다.
        self.planner_worker_panel_state.status = status;
        self.planner_worker_panel_state.last_operation_label = Some(operation_label.to_string());
        self.planner_worker_panel_state.last_summary = None;
        self.planner_worker_panel_state.last_rejected_summary = None;
        self.planner_worker_panel_state.last_notice_detail = None;
        self.planner_worker_panel_state.last_prompt = Some(prompt);
        self.planner_worker_panel_state.last_response = None;
        self.planner_worker_panel_state.last_host_detail = None;
    }

    // 학습 주석: worker가 정상적으로 응답한 뒤 panel state를 결과 중심으로 갱신합니다. "정상 응답"이어도
    // repair_request나 auto-follow block이 있으면 사용자가 개입해야 하므로 UI status를 실패 계열로 낮춥니다.
    pub(super) fn record_planner_worker_outcome(
        &mut self,
        // 학습 주석: success_status는 호출 경로가 기대한 긍정 상태입니다. refresh 경로는 RefreshSucceeded,
        // repair 경로는 RepairSucceeded를 넘기고, 아래 block rule이 필요하면 실패 status로 변환합니다.
        success_status: PlannerWorkerStatus,
        // 학습 주석: outcome은 worker summary, rejected summary, runtime snapshot, notices, raw response를
        // 모두 담는 application-layer 결과입니다. 여기서 UI panel의 각 last_* field로 펼칩니다.
        outcome: &PlanningWorkerRunOutcome,
    ) {
        // 학습 주석: repair_request가 있거나 snapshot이 auto-follow를 막으면 "worker call은 끝났지만 다음 턴을
        // 안전하게 자동 진행할 수 없음"입니다. 그래서 성공 status를 panel상 실패 status로 변환해 attention을 줍니다.
        self.planner_worker_panel_state.status = if outcome.repair_request.is_some()
            || outcome.runtime_snapshot.blocks_auto_followup()
        {
            // 학습 주석: refresh와 repair는 같은 outcome shape를 쓰지만 panel status enum은 작업 종류별
            // 성공/실패 variant를 갖습니다. match는 호출자가 넘긴 success_status의 작업 종류를 보존한 채 실패로 바꿉니다.
            match success_status {
                PlannerWorkerStatus::RefreshSucceeded => PlannerWorkerStatus::RefreshFailed,
                PlannerWorkerStatus::RepairSucceeded => PlannerWorkerStatus::RepairFailed,
                // 학습 주석: 호출자가 이미 실패/idle/running 같은 특수 status를 넘긴 경우에는 임의 변환하지 않습니다.
                // 이 함수의 책임은 refresh/repair success를 block-aware status로 보정하는 데 한정됩니다.
                _ => success_status,
            }
        } else {
            success_status
        };
        // 학습 주석: worker_summary는 planner가 채택한 판단의 compact 설명이고, rejected_summary는 버린 후보나
        // 실패 판단을 설명합니다. 둘을 나눠 두면 debug panel이 "채택된 것"과 "거절된 것"을 분리해서 보여 줍니다.
        self.planner_worker_panel_state.last_summary = outcome.worker_summary.clone();
        self.planner_worker_panel_state.last_rejected_summary = outcome.rejected_summary.clone();
        // 학습 주석: queue summary는 raw outcome text가 아니라 최신 runtime snapshot에서 다시 계산합니다.
        // worker가 response에서 무엇을 말했든 실제 queue head/summary가 UI의 기준이 되어야 합니다.
        self.planner_worker_panel_state.last_queue_summary =
            planner_queue_summary(&outcome.runtime_snapshot);
        // 학습 주석: notices 중 summary성 문구를 제거한 나머지를 detail로 보관합니다. summary는 위 field들이
        // 담당하고, notice_detail은 parse/repair/block 같은 부가 정보를 담는 보조 채널입니다.
        self.planner_worker_panel_state.last_notice_detail =
            planner_notice_detail(&outcome.notices);
        // 학습 주석: raw response는 debug details용입니다. 일반 status/queue copy와 달리 worker가 실제로
        // 반환한 내용을 나중에 분석할 수 있게 보존합니다.
        self.planner_worker_panel_state.last_response = outcome.worker_response.clone();
        // 학습 주석: host_detail은 worker가 아니라 TUI host가 promotion/repeated queue head 같은 후처리를
        // 했을 때 채워집니다. worker outcome 단계에서는 이전 host 후처리 흔적을 지웁니다.
        self.planner_worker_panel_state.last_host_detail = None;
    }

    // 학습 주석: worker 호출 자체가 실패했거나 host-side refresh/promotion이 실패했을 때 panel state를 실패
    // 결과로 기록합니다. 실패에도 runtime snapshot을 같이 받아 queue summary가 완전히 사라지지 않게 합니다.
    pub(super) fn record_planner_worker_failure(
        &mut self,
        // 학습 주석: 실패 status는 호출 경로가 refresh 실패인지 repair 실패인지 구분해 넘깁니다.
        status: PlannerWorkerStatus,
        // 학습 주석: detail은 worker error 또는 host promotion error를 사람이 읽는 한 줄 요약으로 만든 값입니다.
        // panel의 last_summary로 들어가므로 가장 직접적인 실패 원인을 담아야 합니다.
        detail: &str,
        // 학습 주석: runtime_snapshot은 실패 시점의 planning 상태입니다. invalid snapshot일 수도 있지만,
        // queue overlay/debug panel은 이 값으로 현재 auto-follow 가능성을 설명할 수 있습니다.
        runtime_snapshot: &PlanningRuntimeSnapshot,
    ) {
        // 학습 주석: 실패 기록은 결과성 state를 error 중심으로 재구성합니다. prompt는 running 단계에서 남긴
        // 마지막 요청을 유지하지만, response/notice/rejection은 성공 outcome이 아니므로 비웁니다.
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

// 학습 주석: queue summary는 planner worker panel과 queue overlay가 공유하는 compact planning state copy입니다.
// 우선 실행 가능한 queue head가 있으면 다음 task title을 보여 주고, 없으면 snapshot의 summary fallback을 사용합니다.
pub(super) fn planner_queue_summary(snapshot: &PlanningRuntimeSnapshot) -> Option<String> {
    snapshot
        // 학습 주석: queue_head는 auto-follow가 실제로 이어받을 다음 task입니다. 이 값이 있으면 단순 개수보다
        // "무엇이 다음인가"가 더 중요하므로 가장 우선해서 표시합니다.
        .queue_head()
        // 학습 주석: title은 planning task ledger에서 온 값이라 주변 공백을 제거해 compact status copy로 만듭니다.
        .map(|queue_head| format!("next task: {}", queue_head.task_title.trim()))
        // 학습 주석: queue head가 없을 때도 workspace가 invalid이거나 queue가 idle이면 snapshot summary가
        // 설명을 제공할 수 있습니다. None이면 panel은 queue summary line 자체를 생략합니다.
        .or_else(|| snapshot.queue_summary().map(str::to_string))
}

// 학습 주석: planner_notice_detail은 worker notices 중 이미 summary 필드로 승격된 문구를 제거하고, 남은
// diagnostic만 한 줄 detail로 접습니다. 이렇게 해야 panel이 같은 summary를 두 번 보여 주지 않습니다.
fn planner_notice_detail(notices: &[String]) -> Option<String> {
    // 학습 주석: notices는 worker/service 경계에서 여러 줄로 들어올 수 있지만 panel field는 Option<String>
    // 하나입니다. summary prefix를 걸러낸 뒤 남은 diagnostic을 ` | `로 이어 compact하게 만듭니다.
    let detail = notices
        .iter()
        // 학습 주석: refresh/repair summary는 `last_summary`나 `last_rejected_summary`와 겹치는 정보입니다.
        // detail 영역에는 중복 summary보다 추가 원인과 경고만 남깁니다.
        .filter(|notice| {
            !notice.starts_with("planner refresh summary:")
                && !notice.starts_with("planner repair summary:")
        })
        // 학습 주석: owned String notices를 borrowed str로 바꿔 join 준비를 합니다. 이 함수는 panel state에
        // 새 detail String만 만들고 원본 notices 소유권은 건드리지 않습니다.
        .map(String::as_str)
        .collect::<Vec<_>>()
        // 학습 주석: 여러 diagnostic은 한 panel line에서 읽히도록 separator로 접습니다. 긴 구조화 view가
        // 필요해지기 전까지는 debug panel을 compact하게 유지하는 선택입니다.
        .join(" | ");

    // 학습 주석: 걸러낸 뒤 아무 detail도 없으면 None을 반환합니다. presentation layer는 None을 보고
    // 빈 notice row를 그리지 않아 panel이 중복/공백 noise 없이 유지됩니다.
    (!detail.is_empty()).then_some(detail)
}
