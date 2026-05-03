// reset facade는 filesystem rewrite/remove 작업을 감싼다. 실패 context를 잃으면 operator가 어떤 bootstrap
// artifact나 generated path에서 막혔는지 알 수 없으므로 anyhow::Result를 admin boundary까지 유지한다.
use anyhow::Result;

// PlanningResetTarget은 queue, directions, all 중 무엇을 초기화할지 나타내는 domain-facing 선택지다. admin
// facade는 문자열 form parsing을 이미 끝낸 typed target만 받아 application service로 넘긴다.
use crate::application::service::planning::PlanningResetTarget;

// reset 뒤에는 workspace 상태가 바뀌므로 doctor report를 admin 화면용 projection으로 다시 변환한다.
use super::projection::map_doctor_report;
// facade service는 shared planning dependency bundle을 들고 있고, outcome은 inbound adapter로 돌려줄 DTO다.
use super::{PlanningAdminFacadeService, PlanningAdminResetOutcome};

// 이 impl은 admin facade의 reset use case를 붙인다. inbound API/TUI는 reset 세부 절차를 모르고
// 이 method 하나를 호출해 workspace mutation과 화면 projection을 함께 받는다.
impl PlanningAdminFacadeService {
    // reset_workspace는 admin 명령을 application planning reset service로 전달한 뒤,
    // reset 직후의 doctor 상태까지 포함한 admin response를 구성한다.
    pub fn reset_workspace(
        &self,
        // target은 이미 parse/validation을 통과한 typed reset 범위다. 여기서는 문자열을 다시 해석하지 않는다.
        target: PlanningResetTarget,
    ) -> Result<PlanningAdminResetOutcome> {
        // planning.workspace.reset_workspace가 실제 rewrite/remove를 수행하는 authoritative path다. facade는
        // self.workspace_dir을 넘겨 현재 admin instance가 관리하는 workspace만 대상으로 삼고, reset 정책 자체는
        // workspace service에 남긴다.
        let result = self
            // planning bundle은 facade가 조립 시점에 받은 application use cases 묶음이다.
            .planning
            // workspace service group은 reset/doctor/init처럼 workspace artifact를 다루는 작업을 담당한다.
            .workspace
            // 실패하면 `?`가 즉시 return해, reset 실패 후 잘못된 성공 outcome을 만들지 않는다.
            .reset_workspace(self.workspace_dir.as_str(), target)?;
        // reset 후 doctor를 다시 실행해 "무엇을 고쳤는지"뿐 아니라 "현재 workspace가 유효한지"도
        // 같은 응답에 담는다. admin 화면은 이 snapshot으로 validation panel을 갱신하고, 별도 inspect 호출 없이
        // reset 직후 상태를 표시할 수 있다.
        let doctor = self
            // 같은 planning bundle을 사용하므로 reset과 inspect가 동일한 port 구성과 workspace 기준을 공유한다.
            .planning
            // workspace facade 아래의 inspect는 files를 읽고 validation/queue projection을 산출한다.
            .workspace
            // inspect 실패는 doctor report 내부의 issue로 표현되는 흐름이라 여기서는 `?`를 붙이지 않는다.
            .inspect_workspace(self.workspace_dir.as_str());
        // reset service 결과와 doctor projection을 admin DTO 하나로 합친다. 이 boundary에서 domain type을 직접
        // 노출하지 않기 때문에 web API와 page renderer가 같은 stable response shape을 사용할 수 있다.
        Ok(PlanningAdminResetOutcome {
            // label은 enum variant를 사용자/JSON 친화적인 문자열로 바꾼 값이다.
            target: result.target.label().to_string(),
            // rewritten_paths는 bootstrap content로 다시 쓴 파일 목록이다.
            rewritten_paths: result.rewritten_paths,
            // removed_paths는 reset 범위 때문에 삭제된 generated artifact 목록이다.
            removed_paths: result.removed_paths,
            // doctor는 reset 직후 상태를 화면/API 전용 view model로 낮춘 결과이다.
            doctor: map_doctor_report(&doctor),
        })
    }
}
