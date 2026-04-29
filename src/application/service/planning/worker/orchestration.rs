use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::application::port::outbound::planning_authority_port::{
    PlanningAuthorityOfficialRefreshClaimStatus, PlanningAuthorityPort,
};
use crate::application::port::outbound::planning_task_repository_port::PlanningTaskRepositoryPort;
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
use crate::application::service::planning::shared::prompt_sections::{
    LEGACY_AUTHORITY_ARTIFACTS, PlanningPromptHandoff, PlanningWorkerAuthorityPromptContext,
    add_worker_authority_context_sections, worker_previous_handoff_lines, worker_role_lines,
    worker_task_authority_output_contract,
};
use crate::application::service::prompt_component::PromptDocument;

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
    planning_task_repository_port: Arc<dyn PlanningTaskRepositoryPort>,
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
        planning_task_repository_port: Arc<dyn PlanningTaskRepositoryPort>,
    ) -> Self {
        Self {
            planning_worker_port,
            runtime_facade,
            planning_authority,
            planning_task_repository_port,
        }
    }

    pub fn refresh_queue_from_reply(
        &self,
        request: PlanningQueueRefreshRequest<'_>,
    ) -> Result<PlanningWorkerRunOutcome> {
        let prompt = self.render_refresh_queue_prompt(&request);
        let previous_handoff = request.previous_handoff_task.cloned();
        self.run_worker_and_reconcile(
            request.workspace_directory,
            &format!("planner-refresh-{}", request.root_turn_id),
            PlanningWorkerOperation::RefreshQueue,
            prompt,
            previous_handoff.as_ref(),
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
            request.previous_handoff_task,
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
            request.previous_handoff_task,
        )
    }

    pub fn render_refresh_queue_prompt(&self, request: &PlanningQueueRefreshRequest<'_>) -> String {
        let authority_context = self.load_worker_authority_context(request.workspace_directory);
        match &request.mode {
            PlanningQueueRefreshMode::FromLatestReply => build_planning_queue_refresh_prompt(
                request.latest_user_message,
                request.latest_main_reply,
                request.previous_handoff_task,
                &authority_context,
            ),
            PlanningQueueRefreshMode::DeriveNextTaskWhenQueueIdle {
                queue_idle_prompt_markdown,
            } => build_planning_queue_idle_derive_prompt(
                request.latest_user_message,
                request.latest_main_reply,
                request.previous_handoff_task,
                queue_idle_prompt_markdown,
                &authority_context,
            ),
        }
    }

    pub fn render_official_completion_refresh_prompt(
        &self,
        request: &PlanningOfficialCompletionRefreshRequest<'_>,
    ) -> String {
        let authority_context = self.load_worker_authority_context(request.workspace_directory);
        build_planning_official_completion_prompt(
            request.latest_user_message,
            request.latest_main_reply,
            request.previous_handoff_task,
            request.contract,
            &authority_context,
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
        previous_handoff: Option<&PlanningTaskHandoff>,
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
                previous_handoff.map(|task| PlanningRepairPromptHandoff {
                    task_id: task.task_id.as_str(),
                    task_title: task.task_title.as_str(),
                    updated_at: task.updated_at.as_str(),
                    status_label: task.status_label.as_str(),
                }),
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

    fn load_worker_authority_context(
        &self,
        workspace_directory: &str,
    ) -> PlanningWorkerAuthorityPromptContext {
        match (
            self.planning_task_repository_port
                .load_direction_authority_snapshot(workspace_directory),
            self.planning_task_repository_port
                .load_task_authority_snapshot(workspace_directory),
        ) {
            (Ok(Some(direction_snapshot)), Ok(Some(task_snapshot))) => {
                PlanningWorkerAuthorityPromptContext {
                    status_lines: vec![
                        "source_of_truth=accepted DB direction authority, accepted DB task authority, and DB queue projection below".to_string(),
                        format!(
                            "direction_revision={}",
                            direction_snapshot.planning_revision
                        ),
                        format!("task_revision={}", task_snapshot.planning_revision),
                        format!("ignore_legacy_artifacts={LEGACY_AUTHORITY_ARTIFACTS}"),
                    ],
                    direction_authority_json: serde_json::to_string_pretty(
                        &direction_snapshot.directions,
                    )
                    .ok(),
                    task_authority_json: serde_json::to_string_pretty(
                        &task_snapshot.task_authority,
                    )
                    .ok(),
                    queue_projection_json: serde_json::to_string_pretty(
                        &task_snapshot.queue_projection,
                    )
                    .ok(),
                }
            }
            (direction_result, task_result) => {
                let direction_status = authority_load_status(direction_result);
                let task_status = authority_load_status(task_result);
                PlanningWorkerAuthorityPromptContext {
                    status_lines: vec![
                        "source_of_truth=accepted DB authority only".to_string(),
                        format!("direction_authority={direction_status}"),
                        format!("task_authority={task_status}"),
                        format!("ignore_legacy_artifacts={LEGACY_AUTHORITY_ARTIFACTS}"),
                    ],
                    direction_authority_json: None,
                    task_authority_json: None,
                    queue_projection_json: None,
                }
            }
        }
    }
}

fn build_planning_queue_refresh_prompt(
    latest_user_message: Option<&str>,
    latest_main_reply: &str,
    previous_handoff_task: Option<&PlanningTaskHandoff>,
    authority_context: &PlanningWorkerAuthorityPromptContext,
) -> String {
    add_worker_authority_context_sections(
        PromptDocument::builder("planning-worker-refresh").lines("role", worker_role_lines()),
        authority_context,
    )
    .bullets("output-contract", worker_task_authority_output_contract())
    .bullets("refresh-policy", queue_refresh_policy_rules())
    .bullets("queue-advancement", queue_advancement_rules())
    .optional_text("latest-operator-request", latest_user_message)
    .lines(
        "previous-handoff",
        worker_previous_handoff_lines(previous_handoff_task.map(worker_handoff)),
    )
    .text("main-session-latest-reply", latest_main_reply)
    .build()
    .render()
}

fn build_planning_queue_idle_derive_prompt(
    latest_user_message: Option<&str>,
    latest_main_reply: &str,
    previous_handoff_task: Option<&PlanningTaskHandoff>,
    queue_idle_prompt_markdown: &str,
    authority_context: &PlanningWorkerAuthorityPromptContext,
) -> String {
    add_worker_authority_context_sections(
        PromptDocument::builder("planning-worker-queue-idle-review")
            .lines("role", worker_role_lines()),
        authority_context,
    )
    .bullets("output-contract", worker_task_authority_output_contract())
    .bullets("idle-review-policy", queue_idle_review_policy_rules())
    .optional_text("latest-operator-request", latest_user_message)
    .lines(
        "previous-handoff",
        worker_previous_handoff_lines(previous_handoff_task.map(worker_handoff)),
    )
    .text("queue-idle-review-prompt", queue_idle_prompt_markdown)
    .text("main-session-latest-reply", latest_main_reply)
    .build()
    .render()
}

fn build_planning_official_completion_prompt(
    latest_user_message: Option<&str>,
    latest_main_reply: &str,
    previous_handoff_task: Option<&PlanningTaskHandoff>,
    contract: &PlanningOfficialCompletionRefreshContract,
    authority_context: &PlanningWorkerAuthorityPromptContext,
) -> String {
    let serialized_contract = serialize_official_completion_refresh_contract(contract);
    let contract_block = format!("```json\n{serialized_contract}\n```");

    add_worker_authority_context_sections(
        PromptDocument::builder("planning-worker-official-completion")
            .lines("role", worker_role_lines()),
        authority_context,
    )
    .bullets("output-contract", worker_task_authority_output_contract())
    .bullets("completion-policy", official_completion_policy_rules())
    .bullets("queue-advancement", queue_advancement_rules())
    .optional_text("latest-operator-request", latest_user_message)
    .lines(
        "previous-handoff",
        worker_previous_handoff_lines(previous_handoff_task.map(worker_handoff)),
    )
    .text("completion-refresh-contract", &contract_block)
    .text("main-session-latest-reply", latest_main_reply)
    .build()
    .render()
}

fn serialize_official_completion_refresh_contract(
    contract: &PlanningOfficialCompletionRefreshContract,
) -> String {
    serde_json::to_string_pretty(&contract)
        .expect("official completion refresh contract should serialize")
}

fn authority_load_status<T>(result: Result<Option<T>>) -> String {
    match result {
        Ok(Some(_)) => "loaded".to_string(),
        Ok(None) => "missing".to_string(),
        Err(error) => format!("error: {error}"),
    }
}

fn queue_refresh_policy_rules() -> Vec<String> {
    vec![
        "Use planning context, latest operator request, and latest main-session reply together."
            .to_string(),
        "If the latest reply names next steps, follow-up work, gaps, or a numbered checklist, treat that as the strongest follow-up signal."
            .to_string(),
        "Update existing matching tasks/proposals instead of creating duplicates.".to_string(),
        "Keep only executable work in `ready`, `blocked`, or `in_progress`; keep operator-choice candidates as `proposed`."
            .to_string(),
        "If proposals exist and the next executable step is clear, promote one top proposal to `ready` and keep the rest proposed."
            .to_string(),
        "If part of a task is complete, narrow the existing task to remaining work or split completed and follow-up slices."
            .to_string(),
    ]
}

fn queue_idle_review_policy_rules() -> Vec<String> {
    vec![
        "The queue is empty; re-check direction goals, success criteria, detail docs, latest request, latest reply, and work list."
            .to_string(),
        "If the latest reply implies next work, create or update follow-up tasks even when directions are generic."
            .to_string(),
        "Put only the single clearest immediate follow-up in `ready` or `in_progress`; keep alternatives as `proposed`."
            .to_string(),
        "If no useful work remains, keep the queue empty and summarize why.".to_string(),
    ]
}

fn official_completion_policy_rules() -> Vec<String> {
    vec![
        "Completion payload is an unofficial agent report until this ledger refresh succeeds."
            .to_string(),
        "Match by `task_id` and `task_title`; decide whether the ledger task is `done`, `blocked`, or still active with updates."
            .to_string(),
        "Process the supplied contract as the single official ledger update input for this refresh order."
            .to_string(),
        "`commit_sha`, `branch_name`, and `worktree_path` are provenance; reflect task meaning in the ledger."
            .to_string(),
        "If validation failed or did not run, decide whether to create a blocked or remediation task."
            .to_string(),
    ]
}

fn queue_advancement_rules() -> Vec<String> {
    vec![
        "Do not repeat the same queue head unchanged.".to_string(),
        "If the same task remains queue head, update scope, description, priority_reason, title, status, or updated_at from the latest evidence."
            .to_string(),
        "Adding only blocked/proposed tasks is not queue advancement.".to_string(),
    ]
}

fn worker_handoff(task: &PlanningTaskHandoff) -> PlanningPromptHandoff<'_> {
    PlanningPromptHandoff {
        task_id: task.task_id.as_str(),
        task_title: task.task_title.as_str(),
        updated_at: task.updated_at.as_str(),
        status_label: task.status_label.as_str(),
    }
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
    use super::build_planning_queue_refresh_prompt;
    use crate::application::service::planning::runtime::facade::PlanningTaskHandoff;
    use crate::application::service::planning::shared::prompt_sections::PlanningWorkerAuthorityPromptContext;

    #[test]
    fn test_module_compiles_after_task_authority_file_removal() {
        assert!(std::env::current_dir().is_ok());
    }

    #[test]
    fn refresh_prompt_embeds_db_authority_and_legacy_ignore_contract() {
        let authority_context = PlanningWorkerAuthorityPromptContext {
            status_lines: vec![
                "source_of_truth=accepted DB direction authority, accepted DB task authority, and DB queue projection below".to_string(),
                "direction_revision=7".to_string(),
                "task_revision=8".to_string(),
                "ignore_legacy_files=task-ledger.json,directions.toml,queue.snapshot.json,planning-snapshot.json,.codex-exec-loop/runtime/exports/*".to_string(),
            ],
            direction_authority_json: Some("{\"version\":1,\"directions\":[]}".to_string()),
            task_authority_json: Some("{\"version\":1,\"tasks\":[]}".to_string()),
            queue_projection_json: Some(
                "{\"next_task\":null,\"active_tasks\":[],\"proposed_tasks\":[],\"skipped_tasks\":[]}"
                    .to_string(),
            ),
        };

        let prompt = build_planning_queue_refresh_prompt(
            Some("latest user"),
            "latest reply",
            Some(&PlanningTaskHandoff {
                task_id: "task-1".to_string(),
                task_title: "Task 1".to_string(),
                direction_id: "direction-a".to_string(),
                combined_priority: 10,
                updated_at: "2026-04-29T00:00:00Z".to_string(),
                status_label: "ready".to_string(),
            }),
            &authority_context,
        );

        assert!(prompt.contains("[accepted-db-direction-authority]"));
        assert!(prompt.contains("{\"version\":1,\"directions\":[]}"));
        assert!(prompt.contains("[accepted-db-task-authority]"));
        assert!(prompt.contains("{\"version\":1,\"tasks\":[]}"));
        assert!(prompt.contains("[db-queue-projection]"));
        assert!(prompt.contains("task-ledger.json,directions.toml,queue.snapshot.json"));
        assert!(prompt.contains(
            "Do not read or infer planning authority from stale legacy/export artifacts."
        ));
    }
}
