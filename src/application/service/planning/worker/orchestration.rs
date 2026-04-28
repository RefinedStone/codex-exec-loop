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
    PlanningReconciliationResult, PlanningRepairPromptHandoff, PlanningRepairRequest,
    PlanningRepairRetryReason, build_planning_repair_prompt,
};
use crate::application::service::planning::runtime::facade::{
    PlanningRuntimeFacadeService, PlanningTaskHandoff,
};
use crate::application::service::planning::runtime::prompt::PlanningRuntimeSnapshot;

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
    pub task_authority_changed: bool,
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

    pub fn repair_task_authority(
        &self,
        request: PlanningLedgerRepairRequest<'_>,
    ) -> Result<PlanningWorkerRunOutcome> {
        let prompt = self.render_repair_task_authority_prompt(&request);
        self.run_worker_and_reconcile(
            request.workspace_directory,
            &format!(
                "planner-repair-{}-{}",
                request.root_turn_id, request.attempt_number
            ),
            PlanningWorkerOperation::RepairTaskAuthority,
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

    pub fn render_repair_task_authority_prompt(
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
        let task_authority_update = worker_response
            .final_agent_message
            .as_deref()
            .and_then(extract_task_authority_update);
        let mut authority_result = PlanningReconciliationResult::default();
        if let Some(candidate_task_authority) = task_authority_update.as_ref() {
            authority_result = self.runtime_facade.commit_task_authority_candidate(
                workspace_directory,
                candidate_task_authority,
                &execution_snapshot,
            )?;
        }
        let reconciliation_result = self.runtime_facade.reconcile_after_turn(
            workspace_directory,
            synthetic_turn_id,
            &worker_response.changed_planning_file_paths,
            &execution_snapshot,
        )?;
        let reconciliation_result =
            merge_reconciliation_results(authority_result, reconciliation_result);
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
        let task_authority_changed = task_authority_update.is_some();
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
            task_authority_changed,
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

이번 세션은 planning 전용입니다. DB task authority를 기준으로 queue를 갱신하세요.
- `result-output.md`는 수정하지 마세요.
- 현재 planning context와 아래 최신 사용자 요청, main session의 최신 답변을 함께 보고 실행 가능한 후속 작업을 task authority에 반영하세요.
- 마지막 답변에는 fenced JSON 하나를 포함하세요: `{{"task_authority": {{...}}}}`. `task_authority` 값은 갱신된 전체 task ledger 문서입니다.
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

이번 세션은 planning 전용입니다. DB task authority를 기준으로 queue를 재평가하고, 필요하면 다음 작업을 적극적으로 도출하세요.
- direction detail docs, queue-idle review prompt, `result-output.md` 는 수정하지 마세요.
- 현재 queue는 비어 있습니다. direction 목표와 success criteria, detail doc, 최신 사용자 요청, 최신 답변, 지금까지의 work list를 다시 비추어보고 다음 실행 가능한 작업이 실제로 남아 있는지 판단하세요.
- 마지막 답변에는 fenced JSON 하나를 포함하세요: `{{"task_authority": {{...}}}}`. `task_authority` 값은 갱신된 전체 task ledger 문서입니다.
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

이번 세션은 planning 전용입니다. DB task authority 기준으로 agent completion을 공식 상태로 반영하세요.
- `result-output.md`는 수정하지 마세요.
- 아래 completion payload는 비공식 agent report입니다. ledger refresh가 성공하기 전까지 이 결과를 공식 완료로 간주하지 마세요.
- payload의 `task_id`와 `task_title`을 기준으로 기존 ledger task를 찾아 `done`, `blocked`, updated active task 중 무엇이 맞는지 판단하세요.
- 마지막 답변에는 fenced JSON 하나를 포함하세요: `{{"task_authority": {{...}}}}`. `task_authority` 값은 갱신된 전체 task ledger 문서입니다.
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
        PlanningWorkerOperation::RepairTaskAuthority => "repair",
    }
}

fn extract_task_authority_update(message: &str) -> Option<String> {
    candidate_json_sections(message)
        .into_iter()
        .find_map(parse_task_authority_update)
}

fn candidate_json_sections(message: &str) -> Vec<&str> {
    let mut sections = Vec::new();
    let mut remainder = message;
    while let Some(start) = remainder.find("```") {
        remainder = &remainder[start + 3..];
        let body_start = remainder.find('\n').map(|index| index + 1).unwrap_or(0);
        let after_header = &remainder[body_start..];
        let Some(end) = after_header.find("```") else {
            break;
        };
        sections.push(after_header[..end].trim());
        remainder = &after_header[end + 3..];
    }
    sections.push(message.trim());
    sections
}

fn parse_task_authority_update(candidate: &str) -> Option<String> {
    if candidate.trim().is_empty() {
        return None;
    }
    let value = serde_json::from_str::<serde_json::Value>(candidate).ok()?;
    if let Some(task_authority) = value.get("task_authority") {
        return serde_json::to_string_pretty(task_authority).ok();
    }
    if value.get("version").is_some() && value.get("tasks").is_some() {
        return serde_json::to_string_pretty(&value).ok();
    }
    None
}

fn merge_reconciliation_results(
    mut primary: PlanningReconciliationResult,
    secondary: PlanningReconciliationResult,
) -> PlanningReconciliationResult {
    primary.notices.extend(secondary.notices);
    primary
        .restored_protected_files
        .extend(secondary.restored_protected_files);
    primary.rejected_task_authority |= secondary.rejected_task_authority;
    primary.rejected_archive_path = primary
        .rejected_archive_path
        .or(secondary.rejected_archive_path);
    primary.queue_projection_action = primary
        .queue_projection_action
        .or(secondary.queue_projection_action);
    primary.repair_request = primary.repair_request.or(secondary.repair_request);
    primary.auto_followup_block_reason = primary
        .auto_followup_block_reason
        .or(secondary.auto_followup_block_reason);
    primary
}

#[cfg(test)]
mod tests {
    #[test]
    fn test_module_compiles_after_task_authority_file_removal() {
        assert!(std::env::current_dir().is_ok());
    }
}
