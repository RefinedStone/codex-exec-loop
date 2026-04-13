use std::sync::Arc;

use anyhow::Result;

use crate::application::port::outbound::planning_worker_port::{
    PlanningWorkerOperation, PlanningWorkerPort, PlanningWorkerRequest,
};
use crate::application::service::planning_prompt_service::PlanningRuntimeSnapshot;
use crate::application::service::planning_reconciliation_service::{
    PlanningRepairRequest, PlanningRepairRetryReason, build_planning_repair_prompt,
};
use crate::application::service::planning_runtime_facade_service::{
    PlanningRuntimeFacadeService, PlanningTaskHandoff,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanningQueueRefreshRequest<'a> {
    pub workspace_directory: &'a str,
    pub root_turn_id: &'a str,
    pub latest_main_reply: &'a str,
    pub previous_handoff_task: Option<&'a PlanningTaskHandoff>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanningLedgerRepairRequest<'a> {
    pub workspace_directory: &'a str,
    pub root_turn_id: &'a str,
    pub repair_request: &'a PlanningRepairRequest,
    pub attempt_number: usize,
    pub max_attempts: usize,
    pub retry_reason: Option<PlanningRepairRetryReason>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanningWorkerRunOutcome {
    pub runtime_snapshot: PlanningRuntimeSnapshot,
    pub notices: Vec<String>,
    pub repair_request: Option<PlanningRepairRequest>,
    pub worker_summary: Option<String>,
    pub rejected_summary: Option<String>,
    pub task_ledger_changed: bool,
}

#[derive(Clone)]
pub struct PlanningWorkerOrchestrationService {
    planning_worker_port: Arc<dyn PlanningWorkerPort>,
    runtime_facade: PlanningRuntimeFacadeService,
}

impl PlanningWorkerOrchestrationService {
    pub fn new(
        planning_worker_port: Arc<dyn PlanningWorkerPort>,
        runtime_facade: PlanningRuntimeFacadeService,
    ) -> Self {
        Self {
            planning_worker_port,
            runtime_facade,
        }
    }

    pub fn refresh_queue_from_reply(
        &self,
        request: PlanningQueueRefreshRequest<'_>,
    ) -> Result<PlanningWorkerRunOutcome> {
        self.run_worker_and_reconcile(
            request.workspace_directory,
            &format!("planner-refresh-{}", request.root_turn_id),
            PlanningWorkerOperation::RefreshQueue,
            build_planning_queue_refresh_prompt(
                request.latest_main_reply,
                request.previous_handoff_task,
            ),
        )
    }

    pub fn repair_task_ledger(
        &self,
        request: PlanningLedgerRepairRequest<'_>,
    ) -> Result<PlanningWorkerRunOutcome> {
        self.run_worker_and_reconcile(
            request.workspace_directory,
            &format!(
                "planner-repair-{}-{}",
                request.root_turn_id, request.attempt_number
            ),
            PlanningWorkerOperation::RepairTaskLedger,
            build_planning_repair_prompt(
                request.repair_request,
                request.attempt_number,
                request.max_attempts,
                request.retry_reason,
            ),
        )
    }

    fn run_worker_and_reconcile(
        &self,
        workspace_directory: &str,
        synthetic_turn_id: &str,
        operation: PlanningWorkerOperation,
        prompt: String,
    ) -> Result<PlanningWorkerRunOutcome> {
        let execution_snapshot = self
            .runtime_facade
            .load_execution_snapshot(workspace_directory)?;
        let worker_response =
            self.planning_worker_port
                .run_planning_session(PlanningWorkerRequest {
                    operation,
                    workspace_directory: workspace_directory.to_string(),
                    prompt,
                })?;
        let reconciliation_result = self.runtime_facade.reconcile_after_turn(
            workspace_directory,
            synthetic_turn_id,
            &worker_response.changed_planning_file_paths,
            &execution_snapshot,
        )?;
        let runtime_snapshot =
            if let Some(block_reason) = reconciliation_result.auto_followup_block_reason.clone() {
                PlanningRuntimeSnapshot::invalid(block_reason)
            } else {
                self.runtime_facade
                    .load_runtime_snapshot_or_invalid(workspace_directory)
            };
        let worker_summary = worker_response
            .final_agent_message
            .as_deref()
            .and_then(first_non_empty_line)
            .map(str::to_string);
        let rejected_summary = reconciliation_result
            .repair_request
            .as_ref()
            .map(|request| request.failure_summary.clone());
        let task_ledger_changed = worker_response
            .changed_planning_file_paths
            .iter()
            .any(|path| path == crate::domain::planning::TASK_LEDGER_FILE_PATH);
        let mut notices = reconciliation_result.notices;
        if let Some(worker_summary) = worker_summary.as_deref() {
            notices.push(format!(
                "planner {} summary: {}",
                operation_label(operation),
                worker_summary
            ));
        }

        Ok(PlanningWorkerRunOutcome {
            runtime_snapshot,
            notices,
            repair_request: reconciliation_result.repair_request,
            worker_summary,
            rejected_summary,
            task_ledger_changed,
        })
    }
}

fn build_planning_queue_refresh_prompt(
    latest_main_reply: &str,
    previous_handoff_task: Option<&PlanningTaskHandoff>,
) -> String {
    let previous_handoff_section = previous_handoff_task.map_or_else(String::new, |task| {
        format!(
            "\n직전에 main session으로 넘긴 task:\n- task_id: {}\n- title: {}\n- 이 task를 아무 변화 없이 그대로 `ready` queue head로 다시 선택하지 마세요.\n- 최신 답변 기준으로 끝났으면 `done`, 계속 진행 중이면 `in_progress`, 막혔으면 `blocked`, 후속 작업이 분리되면 기존 task 갱신 또는 새 task 추가로 반영하세요.\n",
            task.task_id, task.task_title
        )
    });

    format!(
        r#"대리인입니다.
planning worker refresh 입니다.

이번 세션은 planning 전용입니다. `.codex-exec-loop/planning/task-ledger.json` 중심으로 queue를 갱신하세요.
- planning control file 중 수정 가능한 대상은 `task-ledger.json` 하나뿐입니다.
- `directions.toml`, `task-ledger.schema.json`, `result-output.md`, `queue.snapshot.json` 은 수정하지 마세요.
- 현재 workspace의 planning 파일을 읽고, 아래 main session의 최신 답변에서 실행 가능한 후속 작업을 정리해 ledger에 반영하세요.
- 기존 task/proposal과 의미가 겹치면 새 항목을 남발하지 말고 기존 항목을 갱신하세요.
- 일반 queue에 올라가야 할 executable work만 `ready`/`blocked`/`in_progress`로 두고, 아직 operator 판단이 필요한 후보만 `proposed`로 남기세요.
- builtin next-task 자동 진행을 위해, `proposed`만 있고 바로 이어서 진행해야 할 후속 작업이 분명하면 최상위 proposal 1개를 `ready`로 승격하고 나머지 선택지는 `proposed`로 유지하세요.
- 마지막에는 이번 refresh에서 queue에 반영한 핵심 변경을 짧게 요약하세요.
{previous_handoff_section}

main session latest reply:
{latest_main_reply}"#
    )
}

fn first_non_empty_line(text: &str) -> Option<&str> {
    text.lines().map(str::trim).find(|line| !line.is_empty())
}

fn operation_label(operation: PlanningWorkerOperation) -> &'static str {
    match operation {
        PlanningWorkerOperation::RefreshQueue => "refresh",
        PlanningWorkerOperation::RepairTaskLedger => "repair",
    }
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::fs;
    use std::path::Path;
    use std::sync::{Arc, Mutex};
    use std::time::{SystemTime, UNIX_EPOCH};

    use anyhow::{Result, anyhow};

    use super::{PlanningQueueRefreshRequest, PlanningWorkerOrchestrationService};
    use crate::adapter::outbound::filesystem_planning_workspace_adapter::FilesystemPlanningWorkspaceAdapter;
    use crate::application::port::outbound::planning_worker_port::{
        PlanningWorkerPort, PlanningWorkerRequest, PlanningWorkerResponse,
    };
    use crate::application::port::outbound::planning_workspace_port::PlanningWorkspacePort;
    use crate::application::service::planning_bootstrap_service::PlanningBootstrapService;
    use crate::application::service::planning_prompt_service::PlanningPromptService;
    use crate::application::service::planning_reconciliation_service::PlanningReconciliationService;
    use crate::application::service::planning_runtime_facade_service::PlanningRuntimeFacadeService;
    use crate::application::service::planning_runtime_policy_service::PlanningRuntimePolicyService;
    use crate::application::service::planning_validation_service::PlanningValidationService;
    use crate::application::service::priority_queue_service::PriorityQueueService;
    use crate::application::service::turn_prompt_assembly_service::TurnPromptAssemblyService;
    use crate::domain::planning::{
        DIRECTIONS_FILE_PATH, TASK_LEDGER_FILE_PATH, TASK_LEDGER_SCHEMA_FILE_PATH,
    };

    #[derive(Clone)]
    struct ScriptedPlanningWorkerPort {
        workspace_port: Arc<dyn PlanningWorkspacePort>,
        replies: Arc<Mutex<VecDeque<String>>>,
    }

    impl ScriptedPlanningWorkerPort {
        fn new(workspace_port: Arc<dyn PlanningWorkspacePort>, replies: Vec<String>) -> Self {
            Self {
                workspace_port,
                replies: Arc::new(Mutex::new(replies.into())),
            }
        }
    }

    impl PlanningWorkerPort for ScriptedPlanningWorkerPort {
        fn run_planning_session(
            &self,
            request: PlanningWorkerRequest,
        ) -> Result<PlanningWorkerResponse> {
            let next_body = self
                .replies
                .lock()
                .expect("reply mutex poisoned")
                .pop_front()
                .ok_or_else(|| anyhow!("missing scripted task-ledger body"))?;
            self.workspace_port.replace_planning_workspace_file(
                &request.workspace_directory,
                TASK_LEDGER_FILE_PATH,
                Some(next_body.as_str()),
            )?;

            Ok(PlanningWorkerResponse {
                operation: request.operation,
                final_agent_message: Some("updated planning queue".to_string()),
                changed_planning_file_paths: vec![TASK_LEDGER_FILE_PATH.to_string()],
            })
        }
    }

    fn create_temp_workspace(prefix: &str) -> String {
        let unique_suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock should be valid")
            .as_nanos();
        let path = std::env::temp_dir().join(format!("{prefix}-{unique_suffix}"));
        fs::create_dir_all(&path).expect("temp workspace should be created");
        path.display().to_string()
    }

    fn write_bootstrap_workspace(workspace_dir: &str) {
        let planning_dir = Path::new(workspace_dir).join(".codex-exec-loop/planning");
        fs::create_dir_all(&planning_dir).expect("planning directory should be created");
        let artifacts = PlanningBootstrapService::new().build_artifacts();
        fs::write(
            planning_dir.join("directions.toml"),
            artifacts.directions_toml,
        )
        .expect("directions should write");
        fs::write(
            planning_dir.join("task-ledger.json"),
            artifacts.task_ledger_json,
        )
        .expect("task ledger should write");
        fs::write(
            planning_dir.join("task-ledger.schema.json"),
            artifacts.task_ledger_schema_json,
        )
        .expect("schema should write");
        fs::write(
            planning_dir.join("result-output.md"),
            artifacts.result_output_markdown,
        )
        .expect("result output should write");
    }

    fn service_with_worker(
        worker: Arc<dyn PlanningWorkerPort>,
    ) -> PlanningWorkerOrchestrationService {
        let workspace_port: Arc<dyn PlanningWorkspacePort> =
            Arc::new(FilesystemPlanningWorkspaceAdapter::new());
        let validation_service = PlanningValidationService::new();
        let priority_queue_service = PriorityQueueService::new();
        let planning_prompt_service = PlanningPromptService::new(
            workspace_port.clone(),
            validation_service.clone(),
            priority_queue_service.clone(),
        );
        let planning_reconciliation_service = PlanningReconciliationService::new(
            workspace_port,
            validation_service,
            priority_queue_service,
        );

        PlanningWorkerOrchestrationService::new(
            worker,
            PlanningRuntimeFacadeService::new(
                planning_prompt_service,
                planning_reconciliation_service,
                PlanningRuntimePolicyService::new(),
                TurnPromptAssemblyService::new(),
            ),
        )
    }

    #[test]
    fn refresh_queue_from_reply_accepts_valid_worker_candidate() {
        let workspace_dir = create_temp_workspace("planning-worker-refresh");
        write_bootstrap_workspace(&workspace_dir);
        let workspace_port: Arc<dyn PlanningWorkspacePort> =
            Arc::new(FilesystemPlanningWorkspaceAdapter::new());
        let valid_task_ledger = serde_json::to_string_pretty(&serde_json::json!({
                "version": 1,
                "tasks": [{
                    "id": "task-1",
                    "direction_id": "example-direction",
                "direction_relation_note": "latest answer completed the scaffold and left implementation work",
                "title": "Implement the highest-priority follow-up",
                "description": "Continue the next actionable item from the latest answer.",
                "status": "ready",
                "base_priority": 80,
                "dynamic_priority_delta": 0,
                "priority_reason": "latest answer exposed the next implementation slice",
                "depends_on": [],
                "blocked_by": [],
                "created_by": "llm",
                "last_updated_by": "llm",
                "source_turn_id": "turn-1",
                "updated_at": "2026-04-13T00:00:00Z"
            }]
        }))
        .expect("valid task ledger should serialize");
        let worker = Arc::new(ScriptedPlanningWorkerPort::new(
            workspace_port,
            vec![valid_task_ledger],
        ));
        let service = service_with_worker(worker);

        let result = service
            .refresh_queue_from_reply(PlanningQueueRefreshRequest {
                workspace_directory: &workspace_dir,
                root_turn_id: "turn-1",
                latest_main_reply: "Implemented the previous queue head and found one more task.",
                previous_handoff_task: None,
            })
            .expect("refresh should succeed");

        assert!(result.repair_request.is_none());
        assert!(result.runtime_snapshot.has_actionable_queue_head());
        assert_eq!(result.rejected_summary, None);
        assert_eq!(
            result.worker_summary.as_deref(),
            Some("updated planning queue")
        );
    }

    #[test]
    fn refresh_queue_from_reply_restores_invalid_worker_candidate() {
        let workspace_dir = create_temp_workspace("planning-worker-reject");
        write_bootstrap_workspace(&workspace_dir);
        let workspace_port: Arc<dyn PlanningWorkspacePort> =
            Arc::new(FilesystemPlanningWorkspaceAdapter::new());
        let invalid_task_ledger = r#"{
  "version": 1,
  "tasks": [{
    "id": "task-1",
    "direction_id": "example-direction",
    "title": "Broken task",
    "description": "Missing required fields"
  }]
}"#
        .to_string();
        let worker = Arc::new(ScriptedPlanningWorkerPort::new(
            workspace_port.clone(),
            vec![invalid_task_ledger],
        ));
        let service = service_with_worker(worker);

        let result = service
            .refresh_queue_from_reply(PlanningQueueRefreshRequest {
                workspace_directory: &workspace_dir,
                root_turn_id: "turn-2",
                latest_main_reply: "Need to continue after the broken planning update.",
                previous_handoff_task: None,
            })
            .expect("refresh should reconcile invalid worker candidate");

        assert!(result.repair_request.is_some());
        assert!(result.runtime_snapshot.preview_status_label() == "ready");
        assert!(result.rejected_summary.is_some());

        let restored = workspace_port
            .load_planning_workspace_files(&workspace_dir)
            .expect("workspace should load");
        assert_eq!(
            restored
                .directions_toml
                .as_deref()
                .map(str::trim)
                .expect("directions should remain"),
            fs::read_to_string(
                Path::new(&workspace_dir)
                    .join(".codex-exec-loop/planning")
                    .join("directions.toml")
            )
            .expect("directions should read")
            .trim()
        );
        assert!(restored.task_ledger_json.as_deref().is_some());
        assert!(
            restored
                .task_ledger_schema_json
                .as_deref()
                .map(|body| body.contains("\"type\""))
                .unwrap_or(false)
        );
        assert!(
            Path::new(&workspace_dir)
                .join(".codex-exec-loop/planning")
                .join("task-ledger.schema.json")
                .exists()
        );
        assert!(
            Path::new(&workspace_dir)
                .join(DIRECTIONS_FILE_PATH)
                .exists()
        );
        assert!(
            Path::new(&workspace_dir)
                .join(TASK_LEDGER_SCHEMA_FILE_PATH)
                .exists()
        );
    }
}
