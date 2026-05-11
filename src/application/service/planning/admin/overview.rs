// admin overview는 workspace 진단, 방향 요약, runtime 상태를 한 화면에 모으는 read-only 경로다. mutation은 하지
// 않지만 여러 use case의 결과를 결합하므로, 실패를 숨길 것과 전파할 것을 이 facade에서 명확히 구분한다.
use anyhow::Result;

// projection 함수들은 service/domain 결과를 admin 화면 DTO로 바꾸는 adapter 성격의 매핑이다. overview service가
// 원본 service 구조를 화면에 직접 노출하지 않도록 이 파일에서는 매핑 함수만 호출한다.
use super::projection::{map_application_projection, map_directions_summary, map_doctor_report};
// PlanningAdminFacadeService는 admin 기능의 현재 workspace_dir과 PlanningFeature handle을 가진 상위 facade다.
// PlanningAdminOverview/RuntimeSummary는 admin API와 UI가 소비하는 읽기 모델이다.
use super::{PlanningAdminFacadeService, PlanningAdminOverview, PlanningAdminRuntimeSummary};
use crate::application::service::planning::{PlanningApplicationProjection, PlanningDoctorReport};

// 이 impl 블록은 admin facade의 overview 조회 책임을 담는다. 쓰기 작업 없이 여러 하위 use case를 읽어 하나의
// 관리 화면 모델로 조립하는 경로라서, reset/crud 같은 mutation 파일과 분리되어 있다.
impl PlanningAdminFacadeService {
    // load_overview는 admin 첫 화면에 필요한 전체 snapshot을 만든다. workspace doctor는 항상 진단 결과를 만들고,
    // directions summary는 planning workspace가 덜 준비된 상태에서도 화면을 띄울 수 있게 실패를 None으로 낮춘다.
    pub fn load_overview(&self) -> Result<PlanningAdminOverview> {
        // doctor는 파일 존재 여부, 구조 이상, authority 상태 같은 workspace 건강 상태를 검사한다. inspect_workspace는
        // 실패를 report 안의 issue/note로 낮추는 진단 경로라 overview가 열리지 않는 hard error로 취급하지 않는다.
        let doctor = self.load_doctor_report();
        // directions summary는 active planning 문서와 방향별 supporting file 상태를 요약한다. 아직 초기화가 덜 된
        // workspace에서는 load_summary가 실패할 수 있으므로 overview 전체 실패가 아니라 directions 없음으로 표현한다.
        // 이때 doctor가 이미 상단에 원인을 설명하므로 directions panel은 보조 패널로 남을 수 있다.
        let directions = self
            // 같은 PlanningFeature에서 workspace use case를 다시 사용한다. doctor와 summary가 같은 workspace_dir을
            // 보므로 admin 화면의 각 패널이 서로 다른 루트를 보여주지 않는다.
            .planning
            .workspace
            // load_summary는 planning 방향 목록과 queue-idle review context를 읽어온다. 성공 결과만 admin DTO로 올린다.
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

    // load_doctor_report는 workspace 건강 상태를 application facade 경유로 읽는다. overview와 compact control
    // surface가 같은 doctor source를 쓰게 해 adapter별로 health/issue 판단을 복제하지 않게 한다.
    pub fn load_doctor_report(&self) -> PlanningDoctorReport {
        self.planning
            .workspace
            .inspect_workspace(self.workspace_dir.as_str())
    }

    // load_runtime_application_projection은 planning runtime facts의 공통 read model을 돌려주는 compatibility
    // boundary다. admin overview, compact control surface, 이후 CLI/Telegram status path가 같은 projection source를
    // 공유할 수 있게 runtime projection 세부 구조를 여기서 한 번 감춘다.
    pub fn load_runtime_application_projection(&self) -> Result<PlanningApplicationProjection> {
        let runtime = self
            .planning
            .runtime
            // `_or_invalid` 변형은 파일이 없거나 읽기 실패가 있어도 UI가 다룰 수 있는 invalid projection으로 정규화한다.
            .load_runtime_projection_or_invalid(self.workspace_dir.as_str());
        Ok(PlanningApplicationProjection::from_runtime_projection(
            &runtime,
        ))
    }

    // load_runtime_summary는 overview의 일부이면서 runtime 상태만 새로고침하는 API에서도 재사용할 수 있는 작은 읽기
    // 경로다. invalid workspace도 에러 대신 invalid projection으로 표현해 UI가 상태 패널을 안정적으로 그리게 한다.
    pub fn load_runtime_summary(&self) -> Result<PlanningAdminRuntimeSummary> {
        // admin summary는 application projection을 표시용 admin DTO로 낮춘 값이다. queue/proposal facts를 surface마다
        // 다시 해석하지 않도록 projection source를 명시적으로 공유한다.
        Ok(map_application_projection(
            self.load_runtime_application_projection()?,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapter::outbound::filesystem::FilesystemPlanningWorkspaceAdapter;
    use crate::application::port::outbound::planning_authority_port::NoopPlanningAuthorityPort;
    use crate::application::port::outbound::planning_task_repository_port::NoopPlanningTaskRepositoryPort;
    use crate::application::port::outbound::planning_worker_port::NoopPlanningWorkerPort;
    use crate::application::port::outbound::planning_workspace_port::PlanningWorkspacePort;
    use crate::application::service::planning::PlanningServices;
    use std::fs;
    use std::sync::Arc;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn overview_seeds_default_authority_when_workspace_has_no_explicit_files() {
        let fixture = TestAdminFixture::new("admin-overview-empty");

        let overview = fixture
            .facade
            .load_overview()
            .expect("empty workspace should seed and render an overview");

        assert_eq!(overview.workspace_dir, fixture.workspace.path);
        assert_eq!(overview.doctor.planning_state, "ready_without_task");
        assert_eq!(
            overview.doctor.health.as_deref(),
            Some("planning workspace is healthy")
        );
        assert!(overview.directions.is_some());
        assert!(overview.runtime.workspace_present);
        assert_eq!(overview.runtime.preview_status_label, "ready");
        assert!(overview.runtime.queue_head.is_none());
    }

    #[test]
    fn overview_projects_seeded_workspace_directions_doctor_and_runtime() {
        let fixture = TestAdminFixture::new("admin-overview-seeded");
        fixture
            .facade
            .ensure_default_authority()
            .expect("default authority should seed active planning files");

        let overview = fixture
            .facade
            .load_overview()
            .expect("seeded workspace should render an overview");

        assert_eq!(overview.doctor.planning_state, "ready_without_task");
        assert_eq!(
            overview.doctor.health.as_deref(),
            Some("planning workspace is healthy")
        );
        assert!(overview.runtime.workspace_present);
        assert_eq!(overview.runtime.preview_status_label, "ready");
        assert!(overview.runtime.queue_head.is_none());
        let directions = overview
            .directions
            .expect("seeded workspace should expose direction summary");
        assert_eq!(directions.queue_idle_policy, "review_and_enqueue");
        assert_eq!(directions.queue_idle_prompt_status, "ready");
        assert!(
            directions
                .directions
                .iter()
                .any(|direction| direction.id == "general-workstream")
        );
    }

    struct TestAdminFixture {
        workspace: TempPlanningWorkspace,
        facade: PlanningAdminFacadeService,
    }

    impl TestAdminFixture {
        fn new(prefix: &str) -> Self {
            let workspace = TempPlanningWorkspace::new(prefix);
            let workspace_port: Arc<dyn PlanningWorkspacePort> =
                Arc::new(FilesystemPlanningWorkspaceAdapter::new());
            let authority_port = Arc::new(NoopPlanningAuthorityPort::default());
            let task_repository_port = Arc::new(NoopPlanningTaskRepositoryPort);
            let planning = PlanningServices::from_ports(
                workspace_port.clone(),
                authority_port.clone(),
                task_repository_port.clone(),
                Arc::new(NoopPlanningWorkerPort),
            );
            let facade = PlanningAdminFacadeService::from_planning_with_authority(
                workspace.path.clone(),
                planning,
                workspace_port,
                authority_port,
                task_repository_port,
            );
            Self { workspace, facade }
        }
    }

    struct TempPlanningWorkspace {
        path: String,
    }

    impl TempPlanningWorkspace {
        fn new(prefix: &str) -> Self {
            let unique_suffix = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("system clock should be valid")
                .as_nanos();
            let path = std::env::temp_dir().join(format!("{prefix}-{unique_suffix}"));
            fs::create_dir_all(&path).expect("temp planning workspace should be created");
            Self {
                path: path.display().to_string(),
            }
        }
    }

    impl Drop for TempPlanningWorkspace {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }
}
