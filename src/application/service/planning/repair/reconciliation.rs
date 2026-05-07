/*
 * post-turn reconciliation은 Codex 실행이 끝난 뒤 planning state를 보호하는 경계다.
 * direction/task authority의 source of truth가 DB로 이동했기 때문에, 현재 이 service는 turn이
 * 우연히 다시 쓸 수 없는 active planning support file 보호에 집중한다. runtime/facade.rs가 실행
 * 전에 `PlanningExecutionSnapshot`을 잡고, 실행 후 touched planning path와 함께
 * `reconcile_after_turn`을 호출하면 이 모듈이 두 사실을 비교해 필요한 보호 파일을 복구한다.
 */
use std::sync::Arc;

use anyhow::Result;

use crate::application::port::outbound::planning_task_repository_port::PlanningTaskRepositoryPort;
use crate::application::port::outbound::planning_workspace_port::PlanningWorkspaceLoadRecord;
use crate::application::port::outbound::planning_workspace_port::PlanningWorkspacePort;
use crate::application::service::planning::runtime::validation::PlanningValidationService;
use crate::application::service::planning::shared::contract::{
    RESULT_OUTPUT_FILE_PATH, canonical_active_planning_file_path,
};
use crate::domain::planning::PriorityQueueService;

pub use super::ledger_recovery::PlanningQueueProjectionAction;
pub use super::prompt::{
    PlanningRepairPromptHandoff, PlanningRepairRetryReason, build_planning_repair_prompt,
};
pub use super::protected_restore::PlanningProtectedFileRestoration;

#[derive(Clone)]
/*
 * 현재 DB-authority 모델에서 reconciliation은 의도적으로 작다. support file reload/restore에는
 * workspace port만 필요하다. 그래도 validation, queue, task repository dependency를 constructor
 * contract에 남겨 둔 이유는 기존 composition과 미래 authority repair flow가 facade wiring을 다시
 * 바꾸지 않고 같은 service boundary를 공유하게 하기 위해서다.
 */
pub struct PlanningReconciliationService {
    planning_workspace_port: Arc<dyn PlanningWorkspacePort>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
/*
 * user turn을 unchanged 상태로 지나가야 하는 planning file snapshot이다. task/direction authority가
 * DB port로 이동했기 때문에 현재는 result-output만 capture한다. `PlanningWorkspaceLoadRecord`와
 * 별도 타입으로 둔 이유는 generic load 결과가 아니라 execution guard라는 의미를 타입에 남기기 위해서다.
 */
pub struct PlanningExecutionSnapshot {
    // result-output.md는 completion copy contract를 정의하므로, turn 중 unexpected edit이 있으면 복구한다.
    pub result_output_markdown: Option<String>,
}

impl PlanningExecutionSnapshot {
    // TUI post-turn code는 service 호출 전에 이 cheap path check로 reconciliation 필요 여부를 거른다.
    pub fn captures_path(path: &str) -> bool {
        canonical_active_planning_file_path(path).is_some()
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
/*
 * post-turn reconciliation은 단순 success/failure가 아니라 operational report를 반환한다.
 * UI caller는 notices와 restored file list로 status copy를 만들고, future authority repair path는
 * invalid generated planning state를 발견했을 때 repair_request나 auto_follow_block_reason을
 * worker orchestration에 넘길 수 있다.
 */
pub struct PlanningReconciliationResult {
    // post-turn UI path에 표시되는 human-readable status line이다.
    pub notices: Vec<String>,
    // restore path가 per-file outcome을 기록할 때 쓰는 protected file 복구 상세다.
    pub restored_protected_files: Vec<PlanningProtectedFileRestoration>,
    // generated task authority candidate가 accepted되지 않고 rejected됐을 때 true다.
    pub rejected_task_authority: bool,
    // rejected candidate를 operator inspection용으로 저장했다면 그 archive path다.
    pub rejected_archive_path: Option<String>,
    // authority recovery 중 수행된 queue projection 조정 결과다.
    pub queue_projection_action: Option<PlanningQueueProjectionAction>,
    // automatic reconciliation이 candidate를 안전하게 accept할 수 없을 때 repair worker에게 넘길 prompt payload다.
    pub repair_request: Option<PlanningRepairRequest>,
    // reconciliation이 unsafe planning state를 발견한 뒤 auto-follow를 멈춰야 하는 이유다.
    pub auto_follow_block_reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
/*
 * repair request는 planning repair prompt generation에 넘기는 serialized context다.
 * accepted authority와 queue projection을 trusted baseline으로 담고, rejected candidate와 validation
 * message를 함께 실어 repair worker가 무엇이 실패했는지 추측하지 않고 task mutation command를 낼 수 있게 한다.
 */
pub struct PlanningRepairRequest {
    pub failure_summary: String,
    pub validation_errors: Vec<String>,
    pub direction_authority_json: String,
    pub accepted_task_authority_json: String,
    pub accepted_queue_projection_json: String,
    pub rejected_task_authority_json: Option<String>,
    pub rejected_archive_path: Option<String>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
// turn이 어떤 protected active planning file을 건드렸는지 나타내는 compact description이다.
pub(super) struct PlanningChangeSet {
    pub(super) result_output_changed: bool,
}

impl PlanningChangeSet {
    // reported path를 shared active-planning contract로 normalize한 뒤 관련성을 판단한다.
    fn from_paths(paths: &[String]) -> Self {
        let mut change_set = Self::default();
        for path in paths {
            if let Some(RESULT_OUTPUT_FILE_PATH) = canonical_active_planning_file_path(path) {
                change_set.result_output_changed = true;
            }
        }
        change_set
    }

    // protected file이 바뀌지 않았다면 reconciliation 전체를 건너뛸 수 있다.
    fn has_relevant_changes(self) -> bool {
        self.result_output_changed
    }
}

impl PlanningReconciliationService {
    /*
     * production constructor는 full reconciliation dependency set을 받는다. 현재 protected-file
     * restoration은 workspace port만 저장하지만, prefix가 붙은 인자들은 일시적으로 dormant한
     * authority repair collaborator를 composition 밖으로 노출하지 않기 위한 자리다.
     */
    pub fn with_task_repository(
        planning_workspace_port: Arc<dyn PlanningWorkspacePort>,
        _planning_validation_service: PlanningValidationService,
        _priority_queue_service: PriorityQueueService,
        _planning_task_repository_port: Arc<dyn PlanningTaskRepositoryPort>,
    ) -> Self {
        Self {
            planning_workspace_port,
        }
    }

    /*
     * Codex가 turn을 실행하기 전에 protected file content를 capture한다. 이 snapshot은 나중에
     * restore용 workspace record로 다시 변환되므로, 복구 작업도 일반 planning file commit과 같은
     * workspace-port write path를 사용한다.
     */
    pub fn load_execution_snapshot(
        &self,
        workspace_dir: &str,
    ) -> Result<PlanningExecutionSnapshot> {
        let workspace_record = self
            .planning_workspace_port
            .load_planning_workspace_files(workspace_dir)?;
        Ok(PlanningExecutionSnapshot {
            result_output_markdown: workspace_record.result_output_markdown,
        })
    }

    /*
     * turn이 capture된 active path를 실제로 건드렸을 때만 protected planning file을 복구한다.
     * changed path list가 cheap guard이고, result-output이 포함되면 service는 pre-turn snapshot을
     * commit하고 notice를 기록한다. TUI/auto-follow caller가 reconciliation이 보호 planning copy에
     * 대한 turn edit을 의도적으로 폐기했음을 설명할 수 있게 하기 위해서다.
     */
    pub fn reconcile_after_turn(
        &self,
        workspace_dir: &str,
        _turn_id: &str,
        changed_planning_file_paths: &[String],
        execution_snapshot: &PlanningExecutionSnapshot,
    ) -> Result<PlanningReconciliationResult> {
        let change_set = PlanningChangeSet::from_paths(changed_planning_file_paths);
        if !change_set.has_relevant_changes() {
            return Ok(PlanningReconciliationResult::default());
        }

        let mut result = PlanningReconciliationResult::default();
        self.planning_workspace_port
            .commit_planning_workspace_files(
                workspace_dir,
                &execution_snapshot_to_workspace_record(execution_snapshot),
            )?;
        result
            .notices
            .push("planning reconciliation restored protected planning files".to_string());
        Ok(result)
    }
}

// execution guard를 protected-file restore에 필요한 최소 workspace-port payload로 되돌린다.
pub(super) fn execution_snapshot_to_workspace_record(
    execution_snapshot: &PlanningExecutionSnapshot,
) -> PlanningWorkspaceLoadRecord {
    PlanningWorkspaceLoadRecord {
        result_output_markdown: execution_snapshot.result_output_markdown.clone(),
    }
}

#[cfg(test)]
mod tests;
