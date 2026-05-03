// admin overview는 workspace 진단, 방향 요약, runtime 상태를 한 화면에 모으는 read-only 경로다. mutation은 하지
// 않지만 여러 use case의 결과를 결합하므로, 실패를 숨길 것과 전파할 것을 이 facade에서 명확히 구분한다.
use anyhow::Result;

// projection 함수들은 service/domain 결과를 admin 화면 DTO로 바꾸는 adapter 성격의 매핑이다. overview service가
// 원본 service 구조를 화면에 직접 노출하지 않도록 이 파일에서는 매핑 함수만 호출한다.
use super::projection::{map_directions_summary, map_doctor_report, map_runtime_snapshot};
// PlanningAdminFacadeService는 admin 기능의 현재 workspace_dir과 PlanningFeature handle을 가진 상위 facade다.
// PlanningAdminOverview/RuntimeSummary는 admin API와 UI가 소비하는 읽기 모델이다.
use super::{PlanningAdminFacadeService, PlanningAdminOverview, PlanningAdminRuntimeSummary};

// 이 impl 블록은 admin facade의 overview 조회 책임을 담는다. 쓰기 작업 없이 여러 하위 use case를 읽어 하나의
// 관리 화면 모델로 조립하는 경로라서, reset/crud 같은 mutation 파일과 분리되어 있다.
impl PlanningAdminFacadeService {
    // load_overview는 admin 첫 화면에 필요한 전체 snapshot을 만든다. workspace doctor는 항상 진단 결과를 만들고,
    // directions summary는 planning workspace가 덜 준비된 상태에서도 화면을 띄울 수 있게 실패를 None으로 낮춘다.
    pub fn load_overview(&self) -> Result<PlanningAdminOverview> {
        // doctor는 파일 존재 여부, 구조 이상, authority 상태 같은 workspace 건강 상태를 검사한다. inspect_workspace는
        // 실패를 report 안의 issue/note로 낮추는 진단 경로라 overview가 열리지 않는 hard error로 취급하지 않는다.
        let doctor = self
            // planning facade 안에서 workspace use case 묶음을 선택한다. admin facade는 내부 service가 아니라 공개
            // use case 표면을 통해 진단을 수행한다.
            .planning
            .workspace
            // workspace_dir은 이 admin facade 인스턴스가 관리하는 루트다. as_str()로 빌려 넘겨 경로 문자열 소유권을
            // 유지하면서 읽기 작업만 수행한다.
            .inspect_workspace(self.workspace_dir.as_str());
        // directions summary는 active planning 문서와 방향별 supporting file 상태를 요약한다. 아직 초기화가 덜 된
        // workspace에서는 load_summary가 실패할 수 있으므로 overview 전체 실패가 아니라 directions 없음으로 표현한다.
        // 이때 doctor가 이미 상단에 원인을 설명하므로 directions panel은 보조 패널로 남을 수 있다.
        let directions = self
            // 같은 PlanningFeature에서 workspace use case를 다시 사용한다. doctor와 summary가 같은 workspace_dir을
            // 보므로 admin 화면의 각 패널이 서로 다른 루트를 보여주지 않는다.
            .planning
            .workspace
            // load_summary는 planning 방향 목록과 queue idle review context를 읽어온다. 성공 결과만 admin DTO로 올린다.
            .load_summary(self.workspace_dir.as_str())
            // ok()는 Result를 Option으로 낮춘다. directions panel은 보조 정보라서 실패해도 overview 자체는 열 수 있게
            // 하는 정책이고, 실패 세부 원인은 doctor report에 맡긴다.
            .ok()
            // map은 Some일 때만 projection을 적용한다. 원본 workspace summary 타입을 admin API 계약으로 직접 내보내지
            // 않는 경계가 여기서 유지된다.
            .map(map_directions_summary);

        // 최종 overview는 세 읽기 결과를 하나의 화면 모델로 모은다. runtime summary만 `?`로 실패를 전파하는 이유는
        // 실행 상태가 admin 화면의 핵심이고, invalid workspace도 load_runtime_summary 내부에서 명시적 snapshot으로
        // 정규화되기 때문이다. 여기까지 온 runtime error는 단순 표시 누락보다 큰 adapter/service 실패다.
        Ok(PlanningAdminOverview {
            // workspace_dir clone은 응답 DTO가 facade lifetime에 묶이지 않고 독립 문자열을 갖게 한다.
            workspace_dir: self.workspace_dir.clone(),
            // doctor는 domain/service 진단 모델에서 admin 표시 모델로 변환되어 들어간다.
            doctor: map_doctor_report(&doctor),
            // runtime은 별도 함수로 분리해 overview 전체와 runtime-only API가 같은 projection 로직을 공유하게 한다.
            runtime: self.load_runtime_summary()?,
            // directions는 workspace summary를 읽을 수 있을 때만 채워진다. None은 admin 화면에서 "요약 없음" 상태로
            // 처리될 수 있는 명시적인 선택이다.
            directions,
        })
    }

    // load_runtime_summary는 overview의 일부이면서 runtime 상태만 새로고침하는 API에서도 재사용할 수 있는 작은 읽기
    // 경로다. invalid workspace도 에러 대신 invalid snapshot으로 표현해 UI가 상태 패널을 안정적으로 그리게 한다.
    pub fn load_runtime_summary(&self) -> Result<PlanningAdminRuntimeSummary> {
        // runtime snapshot은 현재 planning workspace가 실행 가능/대기/무효/작업 보유 상태인지 판단하는 원천이다.
        // admin facade는 PlanningFeature의 runtime use case를 통해 읽어 내부 runtime service 결합을 피하고, overview와
        // runtime-only refresh endpoint가 같은 snapshot/projection 정책을 공유하게 한다.
        let runtime = self
            .planning
            .runtime
            // `_or_invalid` 변형은 파일이 없거나 읽기 실패가 있어도 UI가 다룰 수 있는 invalid snapshot으로 정규화한다.
            .load_runtime_snapshot_or_invalid(self.workspace_dir.as_str());
        // projection을 거치면 runtime 내부 snapshot의 세부 필드가 admin summary 계약으로 축약된다. 이 함수가 Result를
        // 유지하는 것은 호출자와 load_overview의 반환형을 일관되게 맞추고, future runtime read error를 같은 API
        // shape로 전파할 여지를 남기기 위한 선택이다.
        Ok(map_runtime_snapshot(&runtime))
    }
}
