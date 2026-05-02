// 학습 주석: reset facade는 filesystem/write 작업을 하므로 실패 가능성을 caller에게 전파해야 합니다.
// anyhow::Result를 쓰면 lower planning service의 context 있는 error를 admin layer까지 그대로 올릴 수 있습니다.
use anyhow::Result;

// 학습 주석: PlanningResetTarget은 queue, directions, all 중 무엇을 초기화할지 나타내는 domain-facing 선택지입니다.
// admin facade는 문자열 form parsing을 이미 끝낸 typed target만 받아 application service로 넘깁니다.
use crate::application::service::planning::PlanningResetTarget;

// 학습 주석: reset 뒤에는 workspace 상태가 바뀌므로 doctor report를 admin 화면용 projection으로 다시 변환합니다.
use super::projection::map_doctor_report;
// 학습 주석: facade service는 shared planning dependency bundle을 들고 있고, outcome은 inbound adapter로 돌려줄 DTO입니다.
use super::{PlanningAdminFacadeService, PlanningAdminResetOutcome};

// 학습 주석: 이 impl은 admin facade의 reset use case를 붙입니다. inbound API/TUI는 reset 세부 절차를 모르고
// 이 method 하나를 호출해 workspace mutation과 화면 projection을 함께 받습니다.
impl PlanningAdminFacadeService {
    // 학습 주석: reset_workspace는 admin 명령을 application planning reset service로 전달한 뒤,
    // reset 직후의 doctor 상태까지 포함한 admin response를 구성합니다.
    pub fn reset_workspace(
        &self,
        // 학습 주석: target은 이미 parse/validation을 통과한 typed reset 범위입니다. 여기서는 문자열을 다시 해석하지 않습니다.
        target: PlanningResetTarget,
    ) -> Result<PlanningAdminResetOutcome> {
        // 학습 주석: planning.workspace.reset_workspace가 실제 rewrite/remove를 수행하는 authoritative path입니다.
        // facade는 self.workspace_dir을 문자열로 넘겨 현재 admin instance가 관리하는 workspace만 대상으로 삼습니다.
        let result = self
            // 학습 주석: planning bundle은 facade가 조립 시점에 받은 application use cases 묶음입니다.
            .planning
            // 학습 주석: workspace service group은 reset/doctor/init처럼 workspace artifact를 다루는 작업을 담당합니다.
            .workspace
            // 학습 주석: 실패하면 `?`가 즉시 return해, reset 실패 후 잘못된 성공 outcome을 만들지 않습니다.
            .reset_workspace(self.workspace_dir.as_str(), target)?;
        // 학습 주석: reset 후 doctor를 다시 실행해 "무엇을 고쳤는지"뿐 아니라 "현재 workspace가 유효한지"도
        // 같은 응답에 담습니다. admin 화면은 이 snapshot으로 validation panel을 갱신합니다.
        let doctor = self
            // 학습 주석: 같은 planning bundle을 사용하므로 reset과 inspect가 동일한 port 구성과 workspace 기준을 공유합니다.
            .planning
            // 학습 주석: workspace facade 아래의 inspect는 files를 읽고 validation/queue projection을 산출합니다.
            .workspace
            // 학습 주석: inspect 실패는 doctor report 내부의 issue로 표현되는 흐름이라 여기서는 `?`를 붙이지 않습니다.
            .inspect_workspace(self.workspace_dir.as_str());
        // 학습 주석: reset service 결과와 doctor projection을 admin DTO에 합칩니다. 이 boundary에서 domain type을
        // 직접 노출하지 않기 때문에 web API와 page renderer가 같은 stable response shape을 사용할 수 있습니다.
        Ok(PlanningAdminResetOutcome {
            // 학습 주석: label은 enum variant를 사용자/JSON 친화적인 문자열로 바꾼 값입니다.
            target: result.target.label().to_string(),
            // 학습 주석: rewritten_paths는 bootstrap content로 다시 쓴 파일 목록입니다.
            rewritten_paths: result.rewritten_paths,
            // 학습 주석: removed_paths는 reset 범위 때문에 삭제된 generated artifact 목록입니다.
            removed_paths: result.removed_paths,
            // 학습 주석: doctor는 reset 직후 상태를 화면/API 전용 view model로 낮춘 결과입니다.
            doctor: map_doctor_report(&doctor),
        })
    }
}
