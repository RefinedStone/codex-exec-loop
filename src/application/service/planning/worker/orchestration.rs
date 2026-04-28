use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::application::port::outbound::planning_authority_port::{
    PlanningAuthorityOfficialRefreshClaimStatus, PlanningAuthorityPort,
};
use crate::domain::planning::PlanningOfficialCompletionRefreshContract;
use anyhow::Result;

use crate::application::port::outbound::planning_worker_port::{
    PlanningWorkerOperation, PlanningWorkerPort, PlanningWorkerRequest,
};
use crate::application::service::planning::repair::reconciliation::{
    PlanningRepairPromptHandoff, PlanningRepairRequest, PlanningRepairRetryReason,
    build_planning_repair_prompt,
};
use crate::application::service::planning::runtime::facade::{
    PlanningRuntimeFacadeService, PlanningTaskHandoff,
};
use crate::application::service::planning::runtime::prompt::PlanningRuntimeSnapshot;
use crate::application::service::planning::shared::contract::TASK_LEDGER_FILE_PATH;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanningQueueRefreshRequest<'a> {
    pub workspace_directory: &'a str,
    pub root_turn_id: &'a str,
    pub latest_user_message: Option<&'a str>,
    pub latest_main_reply: &'a str,
    pub previous_handoff_task: Option<&'a PlanningTaskHandoff>,
    pub mode: PlanningQueueRefreshMode<'a>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanningOfficialCompletionRefreshRequest<'a> {
    pub workspace_directory: &'a str,
    pub latest_user_message: Option<&'a str>,
    pub latest_main_reply: &'a str,
    pub previous_handoff_task: Option<&'a PlanningTaskHandoff>,
    pub contract: &'a PlanningOfficialCompletionRefreshContract,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlanningQueueRefreshMode<'a> {
    FromLatestReply,
    DeriveNextTaskWhenQueueIdle { queue_idle_prompt_markdown: &'a str },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanningLedgerRepairRequest<'a> {
    pub workspace_directory: &'a str,
    pub root_turn_id: &'a str,
    pub repair_request: &'a PlanningRepairRequest,
    pub previous_handoff_task: Option<&'a PlanningTaskHandoff>,
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
    pub worker_response: Option<String>,
    pub rejected_summary: Option<String>,
    pub task_ledger_changed: bool,
}

#[derive(Clone)]
pub struct PlanningWorkerOrchestrationService {
    planning_worker_port: Arc<dyn PlanningWorkerPort>,
    runtime_facade: PlanningRuntimeFacadeService,
    planning_authority: Arc<dyn PlanningAuthorityPort>,
}

#[derive(Clone)]
struct OfficialCompletionRefreshPermit {
    planning_authority: Arc<dyn PlanningAuthorityPort>,
    workspace_directory: String,
    refresh_order: u64,
    owner_token: String,
}

impl OfficialCompletionRefreshPermit {
    fn new(
        planning_authority: Arc<dyn PlanningAuthorityPort>,
        workspace_directory: &str,
        refresh_order: u64,
        owner_token: String,
    ) -> Self {
        Self {
            planning_authority,
            workspace_directory: workspace_directory.to_string(),
            refresh_order,
            owner_token,
        }
    }
}

impl Drop for OfficialCompletionRefreshPermit {
    fn drop(&mut self) {
        let _ = self.planning_authority.release_official_refresh_claim(
            &self.workspace_directory,
            self.refresh_order,
            &self.owner_token,
        );
    }
}

impl PlanningWorkerOrchestrationService {
    pub fn new(
        planning_worker_port: Arc<dyn PlanningWorkerPort>,
        runtime_facade: PlanningRuntimeFacadeService,
        planning_authority: Arc<dyn PlanningAuthorityPort>,
    ) -> Self {
        Self {
            planning_worker_port,
            runtime_facade,
            planning_authority,
        }
    }

    pub fn refresh_queue_from_reply(
        &self,
        request: PlanningQueueRefreshRequest<'_>,
    ) -> Result<PlanningWorkerRunOutcome> {
        let prompt = self.render_refresh_queue_prompt(&request);
        self.run_worker_and_reconcile(
            request.workspace_directory,
            &format!("planner-refresh-{}", request.root_turn_id),
            PlanningWorkerOperation::RefreshQueue,
            prompt,
        )
    }

    pub fn refresh_queue_from_official_completion(
        &self,
        request: PlanningOfficialCompletionRefreshRequest<'_>,
    ) -> Result<PlanningWorkerRunOutcome> {
        let prompt = self.render_official_completion_refresh_prompt(&request);
        let _permit = self.acquire_official_refresh_permit(
            request.workspace_directory,
            request.contract.refresh_order,
        )?;
        self.run_worker_and_reconcile(
            request.workspace_directory,
            &format!("planner-refresh-{}", request.contract.root_turn_id),
            PlanningWorkerOperation::RefreshQueue,
            prompt,
        )
    }

    pub fn repair_task_ledger(
        &self,
        request: PlanningLedgerRepairRequest<'_>,
    ) -> Result<PlanningWorkerRunOutcome> {
        let prompt = self.render_repair_task_ledger_prompt(&request);
        self.run_worker_and_reconcile(
            request.workspace_directory,
            &format!(
                "planner-repair-{}-{}",
                request.root_turn_id, request.attempt_number
            ),
            PlanningWorkerOperation::RepairTaskLedger,
            prompt,
        )
    }

    pub fn render_refresh_queue_prompt(&self, request: &PlanningQueueRefreshRequest<'_>) -> String {
        match &request.mode {
            PlanningQueueRefreshMode::FromLatestReply => build_planning_queue_refresh_prompt(
                request.latest_user_message,
                request.latest_main_reply,
                request.previous_handoff_task,
            ),
            PlanningQueueRefreshMode::DeriveNextTaskWhenQueueIdle {
                queue_idle_prompt_markdown,
            } => build_planning_queue_idle_derive_prompt(
                request.latest_user_message,
                request.latest_main_reply,
                request.previous_handoff_task,
                queue_idle_prompt_markdown,
            ),
        }
    }

    pub fn render_official_completion_refresh_prompt(
        &self,
        request: &PlanningOfficialCompletionRefreshRequest<'_>,
    ) -> String {
        build_planning_official_completion_prompt(
            request.latest_user_message,
            request.latest_main_reply,
            request.previous_handoff_task,
            request.contract,
        )
    }

    pub fn render_repair_task_ledger_prompt(
        &self,
        request: &PlanningLedgerRepairRequest<'_>,
    ) -> String {
        build_planning_repair_prompt(
            request.repair_request,
            request
                .previous_handoff_task
                .map(|task| PlanningRepairPromptHandoff {
                    task_id: task.task_id.as_str(),
                    task_title: task.task_title.as_str(),
                    updated_at: task.updated_at.as_str(),
                    status_label: task.status_label.as_str(),
                }),
            request.attempt_number,
            request.max_attempts,
            request.retry_reason,
        )
    }

    fn acquire_official_refresh_permit(
        &self,
        workspace_directory: &str,
        refresh_order: u64,
    ) -> Result<OfficialCompletionRefreshPermit> {
        let owner_token = authority_claim_owner_token("official-refresh", refresh_order);
        loop {
            match self.planning_authority.acquire_official_refresh_claim(
                workspace_directory,
                refresh_order,
                &owner_token,
            )? {
                PlanningAuthorityOfficialRefreshClaimStatus::Acquired => {
                    return Ok(OfficialCompletionRefreshPermit::new(
                        self.planning_authority.clone(),
                        workspace_directory,
                        refresh_order,
                        owner_token,
                    ));
                }
                PlanningAuthorityOfficialRefreshClaimStatus::Waiting => {
                    std::thread::sleep(Duration::from_millis(10));
                }
                PlanningAuthorityOfficialRefreshClaimStatus::AlreadyCompleted => {
                    anyhow::bail!(
                        "official completion refresh order {refresh_order} already completed for `{workspace_directory}`"
                    );
                }
            }
        }
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
            .any(|path| path == TASK_LEDGER_FILE_PATH);
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
            worker_response: worker_response.final_agent_message,
            rejected_summary,
            task_ledger_changed,
        })
    }
}

fn build_planning_queue_refresh_prompt(
    latest_user_message: Option<&str>,
    latest_main_reply: &str,
    previous_handoff_task: Option<&PlanningTaskHandoff>,
) -> String {
    let latest_user_request_section = latest_user_request_section(latest_user_message);
    let previous_handoff_section = previous_handoff_section(previous_handoff_task);

    format!(
        r#"planning worker refresh 입니다.

이번 세션은 planning 전용입니다. `.codex-exec-loop/planning/task-ledger.json` 중심으로 queue를 갱신하세요.
- planning control file 중 수정 가능한 대상은 `task-ledger.json` 하나뿐입니다.
- `directions.toml`, `task-ledger.schema.json`, `result-output.md`, `queue.snapshot.json` 은 수정하지 마세요.
- 현재 workspace의 planning 파일을 읽고, 아래 최신 사용자 요청과 main session의 최신 답변을 함께 보고 실행 가능한 후속 작업을 정리해 ledger에 반영하세요.
- 최신 답변이 다음 순서, 이어서 할 일, 보완 항목, numbered checklist를 직접 제시하면 그것을 우선적인 후속 작업 근거로 사용하세요.
- 기존 task/proposal과 의미가 겹치면 새 항목을 남발하지 말고 기존 항목을 갱신하세요.
- 일반 queue에 올라가야 할 executable work만 `ready`/`blocked`/`in_progress`로 두고, 아직 operator 판단이 필요한 후보만 `proposed`로 남기세요.
- builtin next-task 자동 진행을 위해, `proposed`만 있고 바로 이어서 진행해야 할 후속 작업이 분명하면 최상위 proposal 1개를 `ready`로 승격하고 나머지 선택지는 `proposed`로 유지하세요.
- queue head를 유지하더라도 title, status, priority, updated_at 중 하나도 바뀌지 않은 채 그대로 반복하지 마세요.
- 같은 queue head를 유지해야 한다면 그 task 자체의 scope, description, priority_reason, updated_at 중 최소 하나는 최신 답변 기준으로 다시 써서 진전이 드러나게 하세요.
- 다른 blocked/proposed task만 추가한 것은 queue advancement로 간주되지 않습니다.
- 이미 일부가 끝났다면 기존 task를 더 좁은 남은 작업으로 갱신하거나, 완료된 slice와 새 follow-up task를 분리하세요.
- 마지막에는 이번 refresh에서 queue에 반영한 핵심 변경을 짧게 요약하세요.
{latest_user_request_section}
{previous_handoff_section}

main session latest reply:
{latest_main_reply}"#
    )
}

fn build_planning_queue_idle_derive_prompt(
    latest_user_message: Option<&str>,
    latest_main_reply: &str,
    previous_handoff_task: Option<&PlanningTaskHandoff>,
    queue_idle_prompt_markdown: &str,
) -> String {
    let latest_user_request_section = latest_user_request_section(latest_user_message);
    let previous_handoff_section = previous_handoff_section(previous_handoff_task);

    format!(
        r#"planning worker queue-idle active-derivation review 입니다.

이번 세션은 planning 전용입니다. `.codex-exec-loop/planning/task-ledger.json` 중심으로 queue를 재평가하고, 필요하면 다음 작업을 적극적으로 도출하세요.
- planning control file 중 수정 가능한 대상은 `task-ledger.json` 하나뿐입니다.
- `directions.toml`, direction detail docs, queue-idle review prompt, `task-ledger.schema.json`, `result-output.md`, `queue.snapshot.json` 은 수정하지 마세요.
- 현재 queue는 비어 있습니다. direction 목표와 success criteria, detail doc, 최신 사용자 요청, 최신 답변, 지금까지의 work list를 다시 비추어보고 다음 실행 가능한 작업이 실제로 남아 있는지 판단하세요.
- 최신 답변에 다음 순서, 이어서 만들 항목, 보완해야 할 목차, numbered checklist가 보이면 그것을 근거로 새 follow-up task를 만들어야 합니다.
- simple mode처럼 directions가 generic 하더라도, 최신 사용자 요청과 최신 답변이 분명한 다음 단계를 암시하면 queue를 비워 두지 마세요.
- 이미 done / in_progress / blocked / proposed 로 같은 의미가 관리되고 있으면 중복 생성 대신 기존 항목을 갱신하세요.
- 지금 바로 이어서 실행해야 할 항목만 `ready` 또는 `in_progress`로 두고, 나머지는 `proposed`로 남기세요.
- 최우선 follow-up이 명확하면 1개를 `ready`로 두고, 나머지 가능 작업은 `proposed`로 분리하세요.
- 정말 이어갈 작업이 없다면 queue를 비운 채 유지하고, 그 이유를 짧게 요약하세요.
- 마지막에는 이번 review에서 queue에 반영한 핵심 변경 또는 queue를 비운 판단 근거를 짧게 요약하세요.
{latest_user_request_section}
{previous_handoff_section}

queue-idle review prompt:
{queue_idle_prompt_markdown}

main session latest reply:
{latest_main_reply}"#
    )
}

fn build_planning_official_completion_prompt(
    latest_user_message: Option<&str>,
    latest_main_reply: &str,
    previous_handoff_task: Option<&PlanningTaskHandoff>,
    contract: &PlanningOfficialCompletionRefreshContract,
) -> String {
    let latest_user_request_section = latest_user_request_section(latest_user_message);
    let previous_handoff_section = previous_handoff_section(previous_handoff_task);
    let serialized_contract = serialize_official_completion_refresh_contract(contract);

    format!(
        r#"planning worker official completion refresh 입니다.

이번 세션은 planning 전용입니다. `.codex-exec-loop/planning/task-ledger.json` 기준으로 agent completion을 공식 상태로 반영하세요.
- planning control file 중 수정 가능한 대상은 `task-ledger.json` 하나뿐입니다.
- `directions.toml`, `task-ledger.schema.json`, `result-output.md`, `queue.snapshot.json` 은 수정하지 마세요.
- 아래 completion payload는 비공식 agent report입니다. ledger refresh가 성공하기 전까지 이 결과를 공식 완료로 간주하지 마세요.
- payload의 `task_id`와 `task_title`을 기준으로 기존 ledger task를 찾아 `done`, `blocked`, updated active task 중 무엇이 맞는지 판단하세요.
- follow-up work가 있으면 새 executable task를 queue에 반영하고, 없으면 queue를 비워 둘 수 있습니다.
- 같은 task를 다시 queue head로 유지해야 한다면 그 task 자체의 title, description, priority_reason, updated_at 중 최소 하나는 completion 결과 기준으로 갱신해 반복 assignment를 피하세요.
- 다른 blocked/proposed task만 추가한 것은 queue advancement로 간주되지 않습니다.
- 아래 JSON contract는 이번 refresh에서 처리할 단일 official ledger update input입니다. 여러 completion이 누적돼도 `refresh_order`가 더 작은 contract가 끝난 뒤 다음 contract를 반영하세요.
- `commit_sha`, `branch_name`, `worktree_path`는 provenance 용도입니다. ledger에는 작업 의미 중심으로 반영하되 repeat prevention 판단에 활용하세요.
- `validation_summary`가 실패 또는 미실행이면 후속 task를 `blocked` 또는 보완 task로 표현할지 신중히 결정하세요.
- 마지막에는 이번 official refresh에서 ledger에 반영한 핵심 판단을 짧게 요약하세요.
{latest_user_request_section}
{previous_handoff_section}

serialized completion refresh contract:
```json
{serialized_contract}
```

main session latest reply:
{latest_main_reply}"#,
        serialized_contract = serialized_contract,
    )
}

fn serialize_official_completion_refresh_contract(
    contract: &PlanningOfficialCompletionRefreshContract,
) -> String {
    serde_json::to_string_pretty(&contract)
        .expect("official completion refresh contract should serialize")
}

fn latest_user_request_section(latest_user_message: Option<&str>) -> String {
    latest_user_message
        .map(str::trim)
        .filter(|message| !message.is_empty())
        .map(|message| format!("\nlatest operator request:\n{message}\n"))
        .unwrap_or_default()
}

fn previous_handoff_section(previous_handoff_task: Option<&PlanningTaskHandoff>) -> String {
    previous_handoff_task.map_or_else(String::new, |task| {
        format!(
            "\n직전에 main session으로 넘긴 task:\n- task_id: {}\n- title: {}\n- updated_at: {}\n- status: {}\n- 이 task를 아무 변화 없이 그대로 `ready` queue head로 다시 선택하지 마세요.\n- 같은 task를 유지하려면 그 task 자체가 바뀌었다는 근거가 ledger에 있어야 합니다.\n- 최신 답변 기준으로 끝났으면 `done`, 계속 진행 중이지만 내용이 갱신되었으면 task를 업데이트하세요.\n- 후속 작업이 분리되면 기존 task 갱신 또는 새 task 추가로 반영하세요.\n",
            task.task_id,
            task.task_title,
            task.updated_at,
            task.status_label
        )
    })
}

fn authority_claim_owner_token(prefix: &str, nonce: u64) -> String {
    let unique_suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    format!("{prefix}-{}-{nonce}-{unique_suffix}", std::process::id())
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
    use std::thread;
    use std::time::Duration;
    use std::time::{SystemTime, UNIX_EPOCH};

    use anyhow::{Result, anyhow};

    use super::{
        PlanningOfficialCompletionRefreshRequest, PlanningQueueRefreshMode,
        PlanningQueueRefreshRequest, PlanningWorkerOrchestrationService,
    };
    use crate::adapter::outbound::db::SqlitePlanningAuthorityAdapter;
    use crate::adapter::outbound::filesystem::FilesystemPlanningWorkspaceAdapter;
    use crate::application::port::outbound::planning_authority_port::PlanningAuthorityPort;
    use crate::application::port::outbound::planning_worker_port::{
        PlanningWorkerPort, PlanningWorkerRequest, PlanningWorkerResponse,
    };
    use crate::application::port::outbound::planning_workspace_port::PlanningWorkspacePort;
    use crate::application::service::planning::authoring::bootstrap::{
        PlanningBootstrapMode, PlanningBootstrapService,
    };
    use crate::application::service::planning::repair::reconciliation::PlanningReconciliationService;
    use crate::application::service::planning::runtime::facade::{
        PlanningRuntimeFacadeService, PlanningTaskHandoff,
    };
    use crate::application::service::planning::runtime::policy::PlanningRuntimePolicyService;
    use crate::application::service::planning::runtime::prompt::PlanningPromptService;
    use crate::application::service::planning::runtime::validation::PlanningValidationService;
    use crate::application::service::planning::shared::contract::{
        DIRECTIONS_FILE_PATH, TASK_LEDGER_FILE_PATH, TASK_LEDGER_SCHEMA_FILE_PATH,
    };
    use crate::application::service::turn_prompt_assembly_service::TurnPromptAssemblyService;
    use crate::domain::planning::PriorityQueueService;
    use crate::domain::planning::{
        PlanningOfficialCompletionRefreshContract, PlanningOfficialCompletionRefreshPayload,
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

    #[derive(Clone)]
    struct RecordingPlanningWorkerPort {
        workspace_port: Arc<dyn PlanningWorkspacePort>,
        replies: Arc<Mutex<VecDeque<String>>>,
        refresh_orders: Arc<Mutex<Vec<u64>>>,
    }

    impl RecordingPlanningWorkerPort {
        fn new(
            workspace_port: Arc<dyn PlanningWorkspacePort>,
            replies: Vec<String>,
            refresh_orders: Arc<Mutex<Vec<u64>>>,
        ) -> Self {
            Self {
                workspace_port,
                replies: Arc::new(Mutex::new(replies.into())),
                refresh_orders,
            }
        }
    }

    impl PlanningWorkerPort for RecordingPlanningWorkerPort {
        fn run_planning_session(
            &self,
            request: PlanningWorkerRequest,
        ) -> Result<PlanningWorkerResponse> {
            self.refresh_orders
                .lock()
                .expect("refresh orders mutex poisoned")
                .push(prompt_refresh_order(&request.prompt));

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
        let artifacts =
            PlanningBootstrapService::new().build_artifacts_for_mode(PlanningBootstrapMode::Detail);
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

    fn ready_task_ledger(task_id: &str, source_turn_id: &str) -> String {
        serde_json::to_string_pretty(&serde_json::json!({
            "version": 1,
            "tasks": [{
                "id": task_id,
                "direction_id": "example-direction",
                "direction_relation_note": "latest answer completed the previous slice and exposed one more step",
                "title": format!("Continue {task_id}"),
                "description": "Continue the next actionable item from the latest answer.",
                "status": "ready",
                "base_priority": 80,
                "dynamic_priority_delta": 0,
                "priority_reason": "latest answer exposed the next implementation slice",
                "depends_on": [],
                "blocked_by": [],
                "created_by": "llm",
                "last_updated_by": "llm",
                "source_turn_id": source_turn_id,
                "updated_at": "2026-04-13T00:00:00Z"
            }]
        }))
        .expect("valid task ledger should serialize")
    }

    fn prompt_refresh_order(prompt: &str) -> u64 {
        let marker = "\"refresh_order\": ";
        let suffix = prompt
            .split(marker)
            .nth(1)
            .expect("official completion prompt should contain refresh_order");
        let digits = suffix
            .chars()
            .take_while(|ch| ch.is_ascii_digit())
            .collect::<String>();
        digits
            .parse::<u64>()
            .expect("refresh_order should parse as u64")
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

        let planning_authority: Arc<dyn PlanningAuthorityPort> =
            Arc::new(SqlitePlanningAuthorityAdapter::new());

        PlanningWorkerOrchestrationService::new(
            worker,
            PlanningRuntimeFacadeService::new(
                planning_prompt_service,
                planning_reconciliation_service,
                PlanningRuntimePolicyService::new(),
                TurnPromptAssemblyService::new(),
            ),
            planning_authority,
        )
    }

    #[test]
    fn refresh_queue_from_reply_accepts_valid_worker_candidate() {
        let workspace_dir = create_temp_workspace("planning-worker-refresh");
        write_bootstrap_workspace(&workspace_dir);
        let workspace_port: Arc<dyn PlanningWorkspacePort> =
            Arc::new(FilesystemPlanningWorkspaceAdapter::new());
        let valid_task_ledger = ready_task_ledger("task-1", "turn-1");
        let worker = Arc::new(ScriptedPlanningWorkerPort::new(
            workspace_port,
            vec![valid_task_ledger],
        ));
        let service = service_with_worker(worker);

        let result = service
            .refresh_queue_from_reply(PlanningQueueRefreshRequest {
                workspace_directory: &workspace_dir,
                root_turn_id: "turn-1",
                latest_user_message: Some("Continue the next implementation slice."),
                latest_main_reply: "Implemented the previous queue head and found one more task.",
                previous_handoff_task: None,
                mode: PlanningQueueRefreshMode::FromLatestReply,
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
    fn render_refresh_queue_prompt_includes_latest_user_request_for_idle_derivation() {
        let workspace_dir = create_temp_workspace("planning-worker-render-prompt");
        write_bootstrap_workspace(&workspace_dir);
        let workspace_port: Arc<dyn PlanningWorkspacePort> =
            Arc::new(FilesystemPlanningWorkspaceAdapter::new());
        let worker = Arc::new(ScriptedPlanningWorkerPort::new(workspace_port, Vec::new()));
        let service = service_with_worker(worker);

        let prompt = service.render_refresh_queue_prompt(&PlanningQueueRefreshRequest {
            workspace_directory: &workspace_dir,
            root_turn_id: "turn-3",
            latest_user_message: Some("강의 자료를 이어서 만들되 다음 단계도 계속 진행해줘."),
            latest_main_reply: "1. 중식 분류 체계\n2. 대표 메뉴 20선\n3. 웍 사용법",
            previous_handoff_task: None,
            mode: PlanningQueueRefreshMode::DeriveNextTaskWhenQueueIdle {
                queue_idle_prompt_markdown: "# Queue Idle Review Prompt",
            },
        });

        assert!(prompt.contains("latest operator request:"));
        assert!(prompt.contains("강의 자료를 이어서 만들되 다음 단계도 계속 진행해줘."));
        assert!(prompt.contains("최우선 follow-up이 명확하면 1개를 `ready`로"));
    }

    #[test]
    fn render_refresh_queue_prompt_includes_previous_handoff_context() {
        let workspace_dir = create_temp_workspace("planning-worker-render-handoff");
        write_bootstrap_workspace(&workspace_dir);
        let workspace_port: Arc<dyn PlanningWorkspacePort> =
            Arc::new(FilesystemPlanningWorkspaceAdapter::new());
        let worker = Arc::new(ScriptedPlanningWorkerPort::new(workspace_port, Vec::new()));
        let service = service_with_worker(worker);
        let previous_handoff = PlanningTaskHandoff {
            task_id: "task-7".to_string(),
            task_title: "정리된 강의 목차를 실제 슬라이드 초안으로 확장".to_string(),
            direction_id: "example-direction".to_string(),
            combined_priority: 90,
            updated_at: "2026-04-14T00:00:00Z".to_string(),
            status_label: "ready".to_string(),
        };

        let prompt = service.render_refresh_queue_prompt(&PlanningQueueRefreshRequest {
            workspace_directory: &workspace_dir,
            root_turn_id: "turn-4",
            latest_user_message: Some("다음 작업도 이어서 진행해줘."),
            latest_main_reply: "강의 목차를 정리했고 이제 슬라이드 초안을 만들 차례입니다.",
            previous_handoff_task: Some(&previous_handoff),
            mode: PlanningQueueRefreshMode::FromLatestReply,
        });

        assert!(prompt.contains("직전에 main session으로 넘긴 task:"));
        assert!(prompt.contains("- task_id: task-7"));
        assert!(prompt.contains("- updated_at: 2026-04-14T00:00:00Z"));
        assert!(prompt.contains("그대로 `ready` queue head로 다시 선택하지 마세요."));
    }

    #[test]
    fn render_official_completion_prompt_includes_serialized_contract() {
        let workspace_dir = create_temp_workspace("planning-worker-render-completion");
        write_bootstrap_workspace(&workspace_dir);
        let workspace_port: Arc<dyn PlanningWorkspacePort> =
            Arc::new(FilesystemPlanningWorkspaceAdapter::new());
        let worker = Arc::new(ScriptedPlanningWorkerPort::new(workspace_port, Vec::new()));
        let service = service_with_worker(worker);

        let contract = PlanningOfficialCompletionRefreshContract::new(
            "turn-9",
            3,
            PlanningOfficialCompletionRefreshPayload::new(
                "agent-2",
                "task-9",
                "Official completion pipeline 구현",
                "akra-agent/slot-1/official-completion",
                "/tmp/slot-1",
                "abc123def456",
                "cargo test passed",
                "official completion lifecycle wired",
                Some("Implemented official completion reporting.".to_string()),
                None,
                "2026-04-17T09:10:00Z",
            ),
        );
        let prompt = service.render_official_completion_refresh_prompt(
            &PlanningOfficialCompletionRefreshRequest {
                workspace_directory: &workspace_dir,
                latest_user_message: Some("이후 작업도 이어서 진행해."),
                latest_main_reply: "구현을 마쳤고 follow-up 하나가 남았습니다.",
                previous_handoff_task: Some(&PlanningTaskHandoff {
                    task_id: "task-9".to_string(),
                    task_title: "Official completion pipeline 구현".to_string(),
                    direction_id: "supersession-ledger-feedback-loop".to_string(),
                    combined_priority: 88,
                    updated_at: "2026-04-17T09:00:00Z".to_string(),
                    status_label: "ready".to_string(),
                }),
                contract: &contract,
            },
        );

        assert!(prompt.contains("serialized completion refresh contract:"));
        assert!(prompt.contains("\"version\": 2"));
        assert!(prompt.contains("\"refresh_kind\": \"official_completion\""));
        assert!(prompt.contains("\"root_turn_id\": \"turn-9\""));
        assert!(prompt.contains("\"refresh_order\": 3"));
        assert!(prompt.contains("\"agent_id\": \"agent-2\""));
        assert!(prompt.contains("\"task_id\": \"task-9\""));
        assert!(prompt.contains("\"commit_sha\": \"abc123def456\""));
        assert!(prompt.contains("\"validation_summary\": \"cargo test passed\""));
        assert!(
            prompt.contains("\"final_response_summary\": \"official completion lifecycle wired\"")
        );
        assert!(prompt.contains("Implemented official completion reporting."));
        assert!(
            prompt.contains("이번 refresh에서 처리할 단일 official ledger update input입니다.")
        );
        assert!(prompt.contains("latest operator request:"));
    }

    #[test]
    fn official_completion_refreshes_run_in_reserved_order() {
        let workspace_dir = create_temp_workspace("planning-worker-official-order");
        write_bootstrap_workspace(&workspace_dir);
        let workspace_port: Arc<dyn PlanningWorkspacePort> =
            Arc::new(FilesystemPlanningWorkspaceAdapter::new());
        let observed_orders = Arc::new(Mutex::new(Vec::new()));
        let worker = Arc::new(RecordingPlanningWorkerPort::new(
            workspace_port,
            vec![
                ready_task_ledger("task-1", "turn-1"),
                ready_task_ledger("task-2", "turn-2"),
            ],
            observed_orders.clone(),
        ));
        let earlier_service = service_with_worker(worker.clone());
        let later_service = service_with_worker(worker);
        let later_workspace_dir = workspace_dir.clone();
        let later_refresh = thread::spawn(move || {
            let contract = PlanningOfficialCompletionRefreshContract::new(
                "turn-2",
                2,
                PlanningOfficialCompletionRefreshPayload::new(
                    "agent-2",
                    "task-2",
                    "Task Two",
                    "akra-agent/slot-2/task-two",
                    "/tmp/slot-2",
                    "sha-2",
                    "cargo test passed",
                    "second completion finished",
                    None,
                    None,
                    "2026-04-17T10:01:00Z",
                ),
            );
            later_service.refresh_queue_from_official_completion(
                PlanningOfficialCompletionRefreshRequest {
                    workspace_directory: &later_workspace_dir,
                    latest_user_message: Some("다음 작업도 이어서 진행해."),
                    latest_main_reply: "두 번째 agent completion 입니다.",
                    previous_handoff_task: None,
                    contract: &contract,
                },
            )
        });

        thread::sleep(Duration::from_millis(50));

        let earlier_workspace_dir = workspace_dir.clone();
        let earlier_refresh = thread::spawn(move || {
            let contract = PlanningOfficialCompletionRefreshContract::new(
                "turn-1",
                1,
                PlanningOfficialCompletionRefreshPayload::new(
                    "agent-1",
                    "task-1",
                    "Task One",
                    "akra-agent/slot-1/task-one",
                    "/tmp/slot-1",
                    "sha-1",
                    "cargo test passed",
                    "first completion finished",
                    None,
                    None,
                    "2026-04-17T10:00:00Z",
                ),
            );
            earlier_service.refresh_queue_from_official_completion(
                PlanningOfficialCompletionRefreshRequest {
                    workspace_directory: &earlier_workspace_dir,
                    latest_user_message: Some("첫 번째 completion 을 공식 반영해."),
                    latest_main_reply: "첫 번째 agent completion 입니다.",
                    previous_handoff_task: None,
                    contract: &contract,
                },
            )
        });

        earlier_refresh
            .join()
            .expect("earlier refresh thread should not panic")
            .expect("earlier refresh should succeed");
        later_refresh
            .join()
            .expect("later refresh thread should not panic")
            .expect("later refresh should succeed");

        assert_eq!(
            observed_orders
                .lock()
                .expect("refresh order mutex poisoned")
                .as_slice(),
            &[1, 2]
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
                latest_user_message: Some("Need to continue after the broken planning update."),
                latest_main_reply: "Need to continue after the broken planning update.",
                previous_handoff_task: None,
                mode: PlanningQueueRefreshMode::FromLatestReply,
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
