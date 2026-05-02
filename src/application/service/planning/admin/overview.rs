// 학습 주석: admin overview는 workspace 진단, 방향 요약, runtime 상태를 한 화면에 모으는 읽기 전용 경로입니다. 중간 단계 중
// 실패할 수 있는 runtime summary를 그대로 호출자에게 전파해야 하므로 anyhow::Result를 반환 계약으로 사용합니다.
use anyhow::Result;

// 학습 주석: projection 함수들은 service/domain 결과를 admin 화면 DTO로 바꾸는 adapter 성격의 매핑입니다. overview service가
// 원본 service 구조를 화면에 직접 노출하지 않도록 이 파일에서는 매핑 함수만 호출합니다.
use super::projection::{map_directions_summary, map_doctor_report, map_runtime_snapshot};
// 학습 주석: PlanningAdminFacadeService는 admin 기능의 현재 workspace_dir과 PlanningFeature handle을 가진 상위 facade입니다.
// PlanningAdminOverview/RuntimeSummary는 admin API와 UI가 소비하는 읽기 모델입니다.
use super::{PlanningAdminFacadeService, PlanningAdminOverview, PlanningAdminRuntimeSummary};

// 학습 주석: 이 impl 블록은 admin facade의 overview 조회 책임을 담습니다. 쓰기 작업 없이 여러 하위 use case를 읽어 하나의
// 관리 화면 모델로 조립하는 경로라서, reset/crud 같은 mutation 파일과 분리되어 있습니다.
impl PlanningAdminFacadeService {
    // 학습 주석: load_overview는 admin 첫 화면에 필요한 전체 snapshot을 만듭니다. workspace doctor는 항상 진단 결과를 만들고,
    // directions summary는 planning workspace가 덜 준비된 상태에서도 화면을 띄울 수 있게 실패를 None으로 낮춥니다.
    pub fn load_overview(&self) -> Result<PlanningAdminOverview> {
        // 학습 주석: doctor는 파일 존재 여부, 구조 이상, authority 상태 같은 workspace 건강 상태를 검사합니다. admin overview의
        // 최상단 상태 표시가 이 값을 기반으로 하므로 directions/runtime보다 먼저 읽어도 부작용이 없습니다.
        let doctor = self
            // 학습 주석: planning facade 안에서 workspace use case 묶음을 선택합니다. admin facade는 내부 service가 아니라 공개
            // use case 표면을 통해 진단을 수행합니다.
            .planning
            .workspace
            // 학습 주석: workspace_dir은 이 admin facade 인스턴스가 관리하는 루트입니다. as_str()로 빌려 넘겨 경로 문자열 소유권을
            // 유지하면서 읽기 작업만 수행합니다.
            .inspect_workspace(self.workspace_dir.as_str());
        // 학습 주석: directions는 active planning 문서와 방향별 supporting file 상태를 요약합니다. 아직 초기화가 덜 된 workspace에서는
        // load_summary가 실패할 수 있으므로 overview 전체 실패가 아니라 directions 없음으로 표현합니다.
        let directions = self
            // 학습 주석: 같은 PlanningFeature에서 workspace use case를 다시 사용합니다. doctor와 summary가 같은 workspace_dir을
            // 보므로 admin 화면의 각 패널이 서로 다른 루트를 보여주지 않습니다.
            .planning
            .workspace
            // 학습 주석: load_summary는 planning 방향 목록과 queue idle review context를 읽어옵니다. 성공 결과만 admin DTO로 올립니다.
            .load_summary(self.workspace_dir.as_str())
            // 학습 주석: ok()는 Result를 Option으로 낮춥니다. directions 패널은 보조 정보라서 실패해도 overview 자체는 열 수 있게
            // 하는 정책입니다.
            .ok()
            // 학습 주석: map은 Some일 때만 projection을 적용합니다. 원본 workspace summary 타입을 admin API 계약으로 직접 내보내지
            // 않는 경계가 여기서 유지됩니다.
            .map(map_directions_summary);

        // 학습 주석: 최종 overview는 세 읽기 결과를 하나의 화면 모델로 모읍니다. runtime summary만 `?`로 실패를 전파하는 이유는
        // 실행 상태가 admin 화면의 핵심 상태이고, invalid snapshot도 load_runtime_summary 내부에서 명시적으로 만들기 때문입니다.
        Ok(PlanningAdminOverview {
            // 학습 주석: workspace_dir clone은 응답 DTO가 facade lifetime에 묶이지 않고 독립 문자열을 갖게 합니다.
            workspace_dir: self.workspace_dir.clone(),
            // 학습 주석: doctor는 domain/service 진단 모델에서 admin 표시 모델로 변환되어 들어갑니다.
            doctor: map_doctor_report(&doctor),
            // 학습 주석: runtime은 별도 함수로 분리해 overview 전체와 runtime-only API가 같은 projection 로직을 공유하게 합니다.
            runtime: self.load_runtime_summary()?,
            // 학습 주석: directions는 workspace summary를 읽을 수 있을 때만 채워집니다. None은 admin 화면에서 "요약 없음" 상태로
            // 처리될 수 있는 명시적인 선택입니다.
            directions,
        })
    }

    // 학습 주석: load_runtime_summary는 overview의 일부이면서 runtime 상태만 새로고침하는 API에서도 재사용할 수 있는 작은 읽기
    // 경로입니다. invalid workspace도 에러 대신 invalid snapshot으로 표현해 UI가 상태 패널을 안정적으로 그리게 합니다.
    pub fn load_runtime_summary(&self) -> Result<PlanningAdminRuntimeSummary> {
        // 학습 주석: runtime snapshot은 현재 planning workspace가 실행 가능/대기/무효/작업 보유 상태인지 판단하는 원천입니다.
        // admin facade는 PlanningFeature의 runtime use case를 통해 읽어 내부 runtime service 결합을 피합니다.
        let runtime = self
            .planning
            .runtime
            // 학습 주석: `_or_invalid` 변형은 파일이 없거나 읽기 실패가 있어도 UI가 다룰 수 있는 invalid snapshot으로 정규화합니다.
            .load_runtime_snapshot_or_invalid(self.workspace_dir.as_str());
        // 학습 주석: projection을 거치면 runtime 내부 snapshot의 세부 필드가 admin summary 계약으로 축약됩니다. 이 함수가 Result를
        // 유지하는 것은 호출자와 load_overview의 반환형을 일관되게 맞추기 위한 형태입니다.
        Ok(map_runtime_snapshot(&runtime))
    }
}
