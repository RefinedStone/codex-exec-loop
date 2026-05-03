use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
mod prompts;
use self::prompts::{
    build_planning_official_completion_prompt, build_planning_queue_idle_derive_prompt,
    build_planning_queue_refresh_prompt,
};
use crate::application::port::outbound::planning_authority_port::{
    PlanningAuthorityOfficialRefreshClaimStatus, PlanningAuthorityPort,
};
use crate::application::port::outbound::planning_task_repository_port::PlanningTaskRepositoryPort;
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
use crate::application::service::planning::shared::prompt_sections::PlanningWorkerAuthorityPromptContext;
use crate::application::service::planning::task_mutation::{
    PlanningTaskCommandExtraction, PlanningTaskMutationRequest, PlanningTaskMutationService,
    PlanningTaskMutationSource, extract_planning_task_commands,
};
use crate::domain::planning::PlanningOfficialCompletionRefreshContract;
use anyhow::Result;

/*
 * Worker orchestration is the bridge between free-form LLM planning output and
 * accepted planning authority. It renders prompts with DB authority context,
 * runs the planning worker, converts structured task commands into repository
 * mutations, then lets the runtime facade reconcile protected files and queue
 * projection side effects.
 */
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanningQueueRefreshRequest<'a> {
    // Root turn ids are used only to create synthetic worker turn ids; the
    // accepted task authority still records command provenance through the
    // mutation service.
    pub workspace_directory: &'a str,
    pub root_turn_id: &'a str,
    pub latest_user_message: Option<&'a str>,
    pub latest_main_reply: &'a str,
    pub previous_handoff_task: Option<&'a PlanningTaskHandoff>,
    pub mode: PlanningQueueRefreshMode<'a>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanningOfficialCompletionRefreshRequest<'a> {
    // Official completion refreshes carry a contract with a monotonic
    // refresh_order. That order is what prevents duplicate queue derivation
    // when multiple clients observe the same completed official turn.
    pub workspace_directory: &'a str,
    pub latest_user_message: Option<&'a str>,
    pub latest_main_reply: &'a str,
    pub previous_handoff_task: Option<&'a PlanningTaskHandoff>,
    pub contract: &'a PlanningOfficialCompletionRefreshContract,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlanningQueueRefreshMode<'a> {
    FromLatestReply,
    // Queue-idle derivation is intentionally prompt-configurable through
    // direction authority supporting files, not hard-coded in this service.
    DeriveNextTaskWhenQueueIdle { queue_idle_prompt_markdown: &'a str },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanningLedgerRepairRequest<'a> {
    // Repair attempts are worker calls too, but their prompt is built from a
    // captured rejection packet instead of the latest user/main-turn exchange.
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
    // The outcome is deliberately broader than the worker response: callers need
    // the refreshed runtime snapshot, reconciliation notices, any repair packet,
    // and whether accepted task authority actually changed.
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
    // Ports are split by trust boundary: worker_port runs the LLM, authority and
    // task repositories persist accepted state, and runtime_facade validates the
    // workspace-facing aftermath.
    planning_worker_port: Arc<dyn PlanningWorkerPort>,
    runtime_facade: PlanningRuntimeFacadeService,
    planning_authority: Arc<dyn PlanningAuthorityPort>,
    planning_task_repository_port: Arc<dyn PlanningTaskRepositoryPort>,
    task_mutation_service: PlanningTaskMutationService,
}

#[derive(Clone)]
struct OfficialCompletionRefreshPermit {
    // RAII permit for an official completion refresh claim. Dropping the permit
    // releases the claim even if worker execution or reconciliation fails.
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
            // Store owned data so the release call remains valid even after the
            // request value that created the permit has gone out of scope.
            workspace_directory: workspace_directory.to_string(),
            refresh_order,
            owner_token,
        }
    }
}
impl Drop for OfficialCompletionRefreshPermit {
    fn drop(&mut self) {
        // Release is best-effort because Drop cannot return errors. A stale
        // claim should be treated as authority-store cleanup work, not as a
        // panic in the worker orchestration path.
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
        // The mutation service is the only path that accepts worker-authored
        // task commands. It reuses the task repository port and queue projection
        // service so worker output follows the same validation as user edits.
        let task_mutation_service = PlanningTaskMutationService::new(
            planning_task_repository_port.clone(),
            crate::domain::planning::PriorityQueueService::new(),
        );
        Self {
            planning_worker_port,
            runtime_facade,
            planning_authority,
            planning_task_repository_port,
            task_mutation_service,
        }
    }
    pub fn refresh_queue_from_reply(
        &self,
        request: PlanningQueueRefreshRequest<'_>,
    ) -> Result<PlanningWorkerRunOutcome> {
        // Normal queue refresh uses the latest main reply as evidence and may
        // include the previous handoff so the worker can close or update it.
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
        // The permit is held across the whole worker/reconcile sequence. That
        // prevents another client from deriving tasks for the same official
        // completion order while this one is still in flight.
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
        // Repair mode feeds the worker accepted authority plus rejected payload
        // context and asks it to emit valid planning_task_commands only.
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
        // Prompt rendering always includes the freshest accepted authority
        // snapshot available, but rendering itself does not mutate state.
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
        // Official completion prompts include the completion contract so the
        // worker can reason about the authoritative completion order rather
        // than just the latest visible text.
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
        // Repair prompts translate a previous handoff into a small borrowed
        // view, avoiding any need to clone the entire runtime handoff object.
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
        // Owner tokens include process/time entropy so repeated refresh loops
        // for the same order are distinguishable in the authority store.
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
                    // The authority store serializes by refresh order. Waiting
                    // here is short and explicit because the caller is already
                    // in a background planning refresh path.
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
        _previous_handoff: Option<&PlanningTaskHandoff>,
    ) -> Result<PlanningWorkerRunOutcome> {
        // Capture the execution snapshot before worker execution so protected
        // file reconciliation can compare the worker's file changes against the
        // state that existed when the synthetic planning turn began.
        let execution_snapshot = self
            .runtime_facade
            .load_execution_snapshot(workspace_directory)?;
        // The worker can return both changed planning files and a final message.
        // Only structured planning_task_commands from that final message may
        // mutate accepted task authority.
        let worker_response =
            self.planning_worker_port
                .run_planning_session(PlanningWorkerRequest {
                    operation,
                    workspace_directory: workspace_directory.to_string(),
                    prompt,
                })?;
        let mut authority_result = PlanningReconciliationResult::default();
        let mut task_authority_changed = false;
        if let Some(final_message) = worker_response.final_agent_message.as_deref() {
            // Legacy full task_authority output is rejected. The accepted path
            // is command-based so validation, conflict handling, and queue
            // projection rebuilds stay centralized in PlanningTaskMutationService.
            match extract_planning_task_commands(final_message) {
                PlanningTaskCommandExtraction::Commands(commands) => {
                    match self
                        .task_mutation_service
                        .apply_commands(PlanningTaskMutationRequest {
                            workspace_directory: workspace_directory.to_string(),
                            source: PlanningTaskMutationSource::Llm,
                            source_turn_id: Some(synthetic_turn_id.to_string()),
                            commands,
                        }) {
                        Ok(mutation_result) => {
                            task_authority_changed = mutation_result.task_authority_changed;
                            if mutation_result.task_authority_changed {
                                // The mutation service has already rebuilt the
                                // projection; the reconciliation result records
                                // that fact for downstream notices.
                                authority_result.queue_projection_action =
                                    Some(crate::application::service::planning::repair::reconciliation::PlanningQueueProjectionAction::RebuiltFromAcceptedPlanning);
                                authority_result.notices.push(format!(
                                    "planning worker committed {} task command(s)",
                                    mutation_result.applied_command_count
                                ));
                            }
                        }
                        Err(error) => {
                            authority_result = self.build_rejected_command_result(
                                workspace_directory,
                                &format!(
                                    "planning worker task commands failed validation: {error}"
                                ),
                                None,
                            )?;
                        }
                    }
                }
                PlanningTaskCommandExtraction::LegacyTaskAuthorityRejected(rejected_json) => {
                    // Preserve the rejected payload for the repair prompt so a
                    // follow-up worker can translate it into valid commands.
                    authority_result = self.build_rejected_command_result(
                        workspace_directory,
                        "planning worker returned legacy task_authority; expected planning_task_commands",
                        Some(rejected_json),
                    )?;
                }
                PlanningTaskCommandExtraction::InvalidCommands {
                    error,
                    rejected_json,
                } => {
                    // Invalid command JSON becomes a repair request instead of
                    // silently disappearing; this keeps planning ledger drift
                    // visible to the operator and retry loop.
                    authority_result = self.build_rejected_command_result(
                        workspace_directory,
                        &format!(
                            "planning worker returned invalid planning_task_commands: {error}"
                        ),
                        rejected_json,
                    )?;
                }
                PlanningTaskCommandExtraction::None => {}
            }
        }
        // File-level reconciliation still runs after command handling because a
        // worker may have touched planning workspace files even when no task
        // command was emitted.
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
                // A reconciliation block should be surfaced immediately as an
                // invalid runtime snapshot instead of masking it with a reload.
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
        // Rejected summaries are duplicated onto the outcome because UI callers
        // often need a concise line without unpacking the full repair request.
        let rejected_summary = reconciliation_result
            .repair_request
            .as_ref()
            .map(|request| request.failure_summary.clone());
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
        // Prompt authority context is read-only and best-effort. When both DB
        // snapshots are present, the worker receives exact accepted authority
        // and queue projection; otherwise it receives explicit load statuses.
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
                // Status-only context is better than omitting the section; it
                // tells the worker not to infer authority from workspace files.
                let direction_status = authority_load_status(direction_result);
                let task_status = authority_load_status(task_result);
                PlanningWorkerAuthorityPromptContext {
                    status_lines: vec![
                        "source_of_truth=accepted DB authority only".to_string(),
                        format!("direction_authority={direction_status}"),
                        format!("task_authority={task_status}"),
                    ],
                    direction_authority_json: None,
                    task_authority_json: None,
                    queue_projection_json: None,
                }
            }
        }
    }
    fn build_rejected_command_result(
        &self,
        workspace_directory: &str,
        failure_summary: &str,
        rejected_payload: Option<String>,
    ) -> Result<PlanningReconciliationResult> {
        // Rejection packets carry accepted authority plus the rejected payload.
        // The repair worker can compare them and emit a smaller valid command
        // set instead of rewriting the whole ledger.
        let mut result = PlanningReconciliationResult {
            rejected_task_authority: true,
            ..PlanningReconciliationResult::default()
        };
        // These loads are fallible because repair quality depends on the
        // current accepted authority. Propagating the error is clearer than
        // producing a repair prompt with misleading empty context.
        let direction_snapshot = self
            .planning_task_repository_port
            .load_direction_authority_snapshot(workspace_directory)?;
        let task_snapshot = self
            .planning_task_repository_port
            .load_task_authority_snapshot(workspace_directory)?;
        let direction_authority_json = direction_snapshot
            .as_ref()
            .map(|snapshot| serde_json::to_string_pretty(&snapshot.directions))
            .transpose()?
            .unwrap_or_default();
        let accepted_task_authority_json = task_snapshot
            .as_ref()
            .map(|snapshot| serde_json::to_string_pretty(&snapshot.task_authority))
            .transpose()?
            .unwrap_or_default();
        let accepted_queue_projection_json = task_snapshot
            .as_ref()
            .map(|snapshot| serde_json::to_string_pretty(&snapshot.queue_projection))
            .transpose()?
            .unwrap_or_default();
        result.repair_request = Some(PlanningRepairRequest {
            failure_summary: failure_summary.to_string(),
            validation_errors: vec![failure_summary.to_string()],
            direction_authority_json,
            accepted_task_authority_json,
            accepted_queue_projection_json,
            rejected_task_authority_json: rejected_payload,
            rejected_archive_path: None,
        });
        result.notices.push(failure_summary.to_string());
        Ok(result)
    }
}

fn authority_load_status<T>(result: Result<Option<T>>) -> String {
    // Compact status strings are embedded directly into prompts, where the LLM
    // needs to know whether authority was loaded, missing, or unavailable.
    match result {
        Ok(Some(_)) => "loaded".to_string(),
        Ok(None) => "missing".to_string(),
        Err(error) => format!("error: {error}"),
    }
}

fn authority_claim_owner_token(prefix: &str, nonce: u64) -> String {
    // The token is not security-sensitive; it is a collision-resistant owner id
    // for claim/release bookkeeping across local concurrent refresh attempts.
    let unique_suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    format!("{prefix}-{}-{nonce}-{unique_suffix}", std::process::id())
}

fn first_non_empty_line(text: &str) -> Option<&str> {
    // Notices use only the first non-empty worker line to avoid flooding the UI
    // with the full final agent message.
    text.lines().map(str::trim).find(|line| !line.is_empty())
}

fn operation_label(operation: PlanningWorkerOperation) -> &'static str {
    match operation {
        PlanningWorkerOperation::RefreshQueue => "refresh",
        PlanningWorkerOperation::RepairTaskAuthority => "repair",
    }
}

fn merge_reconciliation_results(
    mut primary: PlanningReconciliationResult,
    secondary: PlanningReconciliationResult,
) -> PlanningReconciliationResult {
    // Command reconciliation and file reconciliation are produced separately.
    // Merge keeps the first non-empty repair/blocking decision while preserving
    // additive notices and protected-file restoration details from both sides.
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
