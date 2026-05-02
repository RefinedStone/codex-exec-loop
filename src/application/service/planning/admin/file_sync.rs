// File sync는 admin facade가 accepted planning 문서를 workspace의 편집 가능한 파일로 내보내고 다시 읽어오는 경로다.
// 실제 쓰기는 표준 fs API로 수행하고, planning 저장소 갱신은 facade helper에 맡긴다.
use std::fs;
// Workspace root와 planning-relative path를 안전하게 결합하기 위해 `Path`를 사용한다.
use std::path::Path;

// Admin API는 실패 원인을 operator에게 그대로 보여 주므로 `Context`로 어느 파일/디렉터리 작업이 실패했는지 붙이고,
// parallel busy guard는 `bail!`로 즉시 중단한다.
use anyhow::{Context, Result, bail};

// 이 파일은 `PlanningAdminFacadeService`에 export/apply 동작을 붙인다. 반환 outcome은 admin page/API가
// notice와 영향을 받은 path 목록을 표시하는 데 쓰는 얇은 DTO다.
use super::{PlanningAdminFacadeService, PlanningAdminFileSyncOutcome};
// Runtime projection snapshot에는 parallel slot lease와 distributor queue 상태가 함께 들어 있다. file sync는
// accepted 파일을 직접 덮어쓸 수 있으므로 이 snapshot으로 병렬 작업 중 여부를 먼저 검사한다.
use crate::application::port::outbound::planning_authority_port::PlanningAuthorityRuntimeProjectionSnapshot;
// 현재 admin file sync의 대상은 operator planning documents 중 result-output markdown이다. 상수를 써서 export
// 경로, apply 경로, notice path가 planning service 전체의 canonical path와 일치하게 한다.
use crate::application::service::planning::RESULT_OUTPUT_FILE_PATH;

// 이 impl은 admin facade의 파일 기반 편집 워크플로우를 담당한다. draft staging API와 달리
// "현재 accepted support file을 workspace에 꺼내 고친 뒤 다시 accepted documents에 반영"하는 운영자용 우회로다.
impl PlanningAdminFacadeService {
    // Accepted planning support file을 workspace 파일로 export한다. parallel worker가 같은 planning authority를
    // 수정 중이면 stale file을 내보낼 수 있으므로 guard를 먼저 통과해야 한다.
    pub fn export_active_files_for_edit(&self) -> Result<PlanningAdminFileSyncOutcome> {
        self.ensure_no_parallel_working("export planning support files")?;
        // Operator documents는 DB/authority가 현재 accepted로 보는 planning support 문서 묶음이다. export는
        // 이 accepted state를 source of truth로 삼고 workspace 파일은 단순 편집 사본으로 만든다.
        let documents = self.load_operator_planning_documents()?;
        // paths는 실제로 쓴 planning-relative path를 caller에게 알려 주는 기록이다. 대상 파일이 늘어나도
        // notice count와 UI 표시가 helper 호출 수와 함께 맞춰지도록 Vec으로 누적한다.
        let mut paths = Vec::new();
        write_candidate_file(
            &self.workspace_dir,
            RESULT_OUTPUT_FILE_PATH,
            &documents.result_output_markdown,
            &mut paths,
        )?;
        // outcome notice는 admin page flash/status copy의 원천이다. paths는 사용자가 어떤 workspace 파일을
        // 열어 편집하면 되는지 보여 주는 machine-readable 목록이다.
        Ok(PlanningAdminFileSyncOutcome {
            notice: format!(
                "exported {} planning support files for editing",
                paths.len()
            ),
            paths,
        })
    }

    // workspace에 export된 파일을 다시 accepted operator documents로 적용한다. 이 경로는 draft validation/promotion이
    // 아니라 admin이 직접 support file을 동기화하는 명령이므로 missing file을 오류로 본다.
    pub fn apply_exported_files(&self) -> Result<PlanningAdminFileSyncOutcome> {
        self.ensure_no_parallel_working("apply exported planning support files")?;
        // 기존 operator documents를 먼저 읽고 대상 필드만 workspace 파일 내용으로 교체한다. 다른 planning support
        // document가 생겨도 이 함수가 의도치 않게 나머지 필드를 초기화하지 않게 하기 위해서다.
        let mut documents = self.load_operator_planning_documents()?;
        documents.result_output_markdown = self
            .planning_workspace_port
            // Workspace port를 통해 planning-relative file을 읽는다. 직접 fs::read_to_string을 쓰지 않아
            // repo-scoped workspace 구현과 파일 시스템 구현의 차이를 port 아래에 남긴다.
            .load_optional_planning_file(self.workspace_dir.as_str(), RESULT_OUTPUT_FILE_PATH)?
            // apply는 export된 파일이 있어야 의미가 있다. None을 빈 문서로 처리하면 실수로 accepted
            // result-output을 지울 수 있으므로 명시적 missing error로 중단한다.
            .ok_or_else(|| anyhow::anyhow!("missing exported file: {RESULT_OUTPUT_FILE_PATH}"))?;
        self.commit_operator_planning_documents(documents)?;
        // 현재 적용 대상은 result-output 하나다. export와 같은 path list shape를 유지해 admin caller가 두 작업의
        // 결과를 같은 UI contract로 표시할 수 있다.
        let paths = vec![RESULT_OUTPUT_FILE_PATH.to_string()];
        Ok(PlanningAdminFileSyncOutcome {
            notice: format!("applied {} exported planning paths", paths.len()),
            paths,
        })
    }

    // File sync는 accepted planning state를 workspace file과 왕복시키므로 parallel worker가 lease를 들고 있거나
    // distributor queue item을 처리 중이면 막는다. action 문자열은 오류 문구의 동사로 쓴다.
    fn ensure_no_parallel_working(&self, action: &str) -> Result<()> {
        // Authority projection은 slot leases와 distributor queue records를 한 번에 읽는 snapshot이다. service 계층에서
        // 이 guard를 두면 admin API, pages, telegram 같은 모든 inbound가 같은 안전 규칙을 공유한다.
        let runtime = self
            .planning_authority_port
            .load_runtime_projections(self.workspace_dir.as_str())?;
        // busy reason이 있으면 구체적인 slot/item 정보를 포함해 실패한다. 단순 "busy"보다 어떤 task가
        // 파일 동기화를 막고 있는지 operator가 바로 알 수 있다.
        if let Some(reason) = describe_parallel_busy(&runtime) {
            bail!("{action} is blocked while parallel work is active: {reason}");
        }
        Ok(())
    }
}

// Parallel busy 설명은 guard의 정책을 문자열로 낮추는 helper다. lease를 먼저 검사하는 이유는 이미 실행/정리
// 중인 slot이 queue record보다 accepted 파일 충돌 위험을 더 직접적으로 나타내기 때문이다.
fn describe_parallel_busy(runtime: &PlanningAuthorityRuntimeProjectionSnapshot) -> Option<String> {
    // Leased/Running/CleanupPending은 모두 file sync가 끼어들면 안 되는 상태다. cleanup도 아직 authority state를
    // 정리하는 중일 수 있어 완료된 슬롯으로 취급하지 않는다.
    if let Some(lease) = runtime.slot_leases.values().find(|lease| {
        matches!(
            lease.state,
            crate::domain::parallel_mode::ParallelModeSlotLeaseState::Leased
                | crate::domain::parallel_mode::ParallelModeSlotLeaseState::Running
                | crate::domain::parallel_mode::ParallelModeSlotLeaseState::CleanupPending
        )
    }) {
        // slot id, state label, task id를 모두 넣어 operator가 어떤 병렬 lane을 기다리거나 정리해야 하는지 알 수 있게 한다.
        return Some(format!(
            "slot {} is {} for task {}",
            lease.slot_id,
            lease.state.label(),
            lease.task_id
        ));
    }
    // lease가 없더라도 distributor queue에 active record가 있으면 곧 slot 작업으로 이어질 수 있다. 이 경우에도
    // export/apply가 stale authority state를 기준으로 움직일 수 있어 차단한다.
    if let Some(record) = runtime
        .distributor_queue_records
        .iter()
        .find(|record| record.queue_state.is_active())
    {
        // queue item id와 task id를 노출해 아직 lease로 승격되지 않은 작업도 operator가 추적할 수 있게 한다.
        return Some(format!(
            "distributor item {} is {} for task {}",
            record.queue_item_id,
            record.queue_state.label(),
            record.task_id
        ));
    }
    None
}

// Accepted document body를 workspace의 planning-relative 파일로 쓴다. helper로 분리해 나중에 export 대상 파일이
// 늘어나도 directory creation, context, path recording 규칙을 한곳에서 공유한다.
fn write_candidate_file(
    // workspace_dir은 admin facade가 바라보는 repo/root다. relative_path와 결합해 실제 파일 시스템 경로를
    // 만들지만, caller에게는 relative path만 결과로 돌려준다.
    workspace_dir: &str,
    relative_path: &str,
    // body는 accepted operator document의 현재 내용이다. export는 변환이나 validation을 하지 않고 그대로 파일로
    // 써서 operator가 실제 accepted markdown을 편집하게 한다.
    body: &str,
    // written_paths는 caller의 outcome에 들어갈 누적 목록이다. 파일 쓰기가 성공한 뒤에만 push해 notice가
    // 실패한 파일까지 포함하지 않게 한다.
    written_paths: &mut Vec<String>,
) -> Result<()> {
    // absolute-ish workspace path는 내부 fs 작업에만 사용한다. 결과 DTO에는 repo 안에서 사용자가 인식하는
    // planning-relative path를 남긴다.
    let path = Path::new(workspace_dir).join(relative_path);
    // result-output path처럼 하위 디렉터리를 포함하는 파일을 export할 수 있으므로 parent를 먼저 만든다.
    // parent가 없을 수 있는 단일 파일 경로도 helper가 처리한다.
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    // write 실패에는 실제 filesystem path를 붙인다. admin caller가 권한/경로 문제를 해결해야 하므로
    // planning-relative path만으로는 진단이 부족하다.
    fs::write(&path, body).with_context(|| format!("failed to write {}", path.display()))?;
    written_paths.push(relative_path.to_string());
    Ok(())
}
