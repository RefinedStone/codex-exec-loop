use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
#[path = "orchestration/logging.rs"]
mod logging;
mod prompts;
use self::logging::{operation_label, orchestration_event_detail};
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
use crate::application::service::planning::runtime::prompt::PlanningRuntimeProjection;
use crate::application::service::planning::shared::prompt_sections::PlanningWorkerAuthorityPromptContext;
use crate::application::service::planning::task_mutation::{
    PlanningTaskCommandExtraction, PlanningTaskMutationRequest, PlanningTaskMutationService,
    PlanningTaskMutationSource, extract_planning_task_commands,
};
use crate::diagnostics::event_log;
use crate::domain::planning::{
    OriginSessionKind, PlanningOfficialCompletionRefreshContract, TaskMutationProvenance,
};
use anyhow::Result;
use serde_json::json;

/*
 * worker orchestration은 free-form worker planning output과 accepted planning authority 사이의 bridge다.
 * DB authority context를 넣어 prompt를 만들고, planning worker를 실행한 뒤, structured task command만
 * repository mutation으로 바꾼다. 마지막으로 runtime facade가 protected file과 queue projection side effect를
 * reconcile하게 하여 worker 출력이 곧바로 authority 전체를 덮어쓰지 못하게 한다.
 */
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanningQueueRefreshRequest<'a> {
    // completed turn id는 hidden worker mutation을 유발한 visible turn을 provenance로 남길 때 쓴다.
    pub workspace_directory: &'a str,
    pub parent_thread_id: Option<&'a str>,
    pub completed_turn_id: &'a str,
    pub latest_user_message: Option<&'a str>,
    pub latest_main_reply: &'a str,
    pub previous_handoff_task: Option<&'a PlanningTaskHandoff>,
    pub mode: PlanningQueueRefreshMode<'a>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanningOfficialCompletionRefreshRequest<'a> {
    // official completion refresh는 monotonic refresh_order를 가진 contract를 싣는다. 여러 client가 같은 완료 turn을
    // 관찰해도 이 order가 중복 queue derivation을 막는 기준이 된다.
    pub workspace_directory: &'a str,
    pub parent_thread_id: Option<&'a str>,
    pub latest_user_message: Option<&'a str>,
    pub latest_main_reply: &'a str,
    pub previous_handoff_task: Option<&'a PlanningTaskHandoff>,
    pub contract: &'a PlanningOfficialCompletionRefreshContract,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlanningQueueRefreshMode<'a> {
    FromLatestMainReply,
    // queue-idle derivation은 이 service에 hard-code하지 않고 direction authority supporting file의 prompt로 조정한다.
    DeriveQueueHeadWhenQueueIdle { queue_idle_prompt_markdown: &'a str },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanningLedgerRepairRequest<'a> {
    // repair attempt도 worker call이지만, prompt는 latest user/main-turn exchange가 아니라 capture된 rejection packet에서 만든다.
    pub workspace_directory: &'a str,
    pub parent_thread_id: Option<&'a str>,
    pub completed_turn_id: &'a str,
    pub repair_request: &'a PlanningRepairRequest,
    pub previous_handoff_task: Option<&'a PlanningTaskHandoff>,
    pub attempt_number: usize,
    pub max_attempts: usize,
    pub retry_reason: Option<PlanningRepairRetryReason>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanningWorkerRunOutcome {
    // outcome은 worker response보다 일부러 넓다. caller는 refreshed runtime projection, reconciliation notice,
    // repair packet, accepted task authority가 실제로 바뀌었는지까지 함께 알아야 한다.
    pub runtime_projection: PlanningRuntimeProjection,
    pub notices: Vec<String>,
    pub repair_request: Option<PlanningRepairRequest>,
    pub worker_summary: Option<String>,
    pub worker_response: Option<String>,
    pub rejected_summary: Option<String>,
    pub task_authority_changed: bool,
}

#[derive(Clone)]
pub struct PlanningWorkerOrchestrationService {
    // port는 trust boundary별로 갈라진다. worker_port는 hidden worker를 실행하고, authority/task repository는 accepted state를
    // 저장하며, runtime_facade는 workspace-facing aftermath를 검증한다.
    planning_worker_port: Arc<dyn PlanningWorkerPort>,
    runtime_facade: PlanningRuntimeFacadeService,
    planning_authority: Arc<dyn PlanningAuthorityPort>,
    planning_task_repository_port: Arc<dyn PlanningTaskRepositoryPort>,
    task_mutation_service: PlanningTaskMutationService,
}

#[derive(Clone)]
struct OfficialCompletionRefreshPermit {
    // official completion refresh claim을 위한 RAII permit이다. worker execution이나 reconciliation이 실패해도
    // permit drop이 claim release를 시도한다.
    planning_authority: Arc<dyn PlanningAuthorityPort>,
    workspace_directory: String,
    refresh_order: u64,
    owner_token: String,
}

#[derive(Debug, Clone, Copy)]
struct WorkerParentProvenance<'a> {
    thread_id: Option<&'a str>,
    turn_id: Option<&'a str>,
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
            // permit을 만든 request 값이 scope 밖으로 나간 뒤에도 release call이 유효하도록 owned data를 보관한다.
            workspace_directory: workspace_directory.to_string(),
            refresh_order,
            owner_token,
        }
    }
}
impl Drop for OfficialCompletionRefreshPermit {
    fn drop(&mut self) {
        // Drop은 error를 반환할 수 없으므로 release는 best-effort다. stale claim은 worker orchestration panic이 아니라
        // authority-store cleanup 작업으로 다룬다.
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
        // mutation service는 worker-authored task command를 받아들이는 유일한 경로다.
        // task repository port와 queue projection service를 재사용해 worker output도 user edit와 같은 검증을 거치게 한다.
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
    #[tracing::instrument(level = "trace", skip(self))]
    pub fn refresh_queue_from_reply(
        &self,
        request: PlanningQueueRefreshRequest<'_>,
    ) -> Result<PlanningWorkerRunOutcome> {
        // normal queue refresh는 latest main reply를 evidence로 쓰고, previous handoff를 함께 넘겨 worker가 닫거나 갱신할 수 있게 한다.
        let prompt = self.render_refresh_queue_prompt(&request);
        let previous_handoff = request.previous_handoff_task.cloned();
        self.run_worker_and_reconcile(
            request.workspace_directory,
            &format!("planning-worker-refresh-{}", request.completed_turn_id),
            PlanningWorkerOperation::RefreshQueue,
            prompt,
            previous_handoff.as_ref(),
            WorkerParentProvenance {
                thread_id: request.parent_thread_id,
                turn_id: Some(request.completed_turn_id),
            },
        )
    }

    pub fn load_runtime_projection_or_invalid(
        &self,
        workspace_directory: &str,
    ) -> PlanningRuntimeProjection {
        self.runtime_facade
            .load_runtime_projection_or_invalid(workspace_directory)
    }

    #[tracing::instrument(level = "trace", skip(self))]
    pub fn refresh_queue_from_official_completion(
        &self,
        request: PlanningOfficialCompletionRefreshRequest<'_>,
    ) -> Result<PlanningWorkerRunOutcome> {
        let prompt = self.render_official_completion_refresh_prompt(&request);
        // permit은 worker/reconcile sequence 전체 동안 유지된다. 이 refresh가 진행 중일 때 다른 client가 같은 official
        // completion order로 task를 다시 derive하지 못하게 한다.
        let _permit = self.acquire_official_refresh_permit(
            request.workspace_directory,
            request.contract.refresh_order,
        )?;
        self.run_worker_and_reconcile(
            request.workspace_directory,
            &format!(
                "planning-worker-refresh-{}",
                request.contract.completed_turn_id
            ),
            PlanningWorkerOperation::RefreshQueue,
            prompt,
            request.previous_handoff_task,
            WorkerParentProvenance {
                thread_id: request.parent_thread_id,
                turn_id: Some(request.contract.completed_turn_id.as_str()),
            },
        )
    }
    #[tracing::instrument(level = "trace", skip(self))]
    pub fn repair_task_authority(
        &self,
        request: PlanningLedgerRepairRequest<'_>,
    ) -> Result<PlanningWorkerRunOutcome> {
        // repair mode는 accepted authority와 rejected payload context를 worker에게 주고, valid planning_task_commands만 내라고 요구한다.
        let prompt = self.render_repair_task_authority_prompt(&request);
        self.run_worker_and_reconcile(
            request.workspace_directory,
            &format!(
                "planning-worker-repair-{}-{}",
                request.completed_turn_id, request.attempt_number
            ),
            PlanningWorkerOperation::RepairTaskAuthority,
            prompt,
            request.previous_handoff_task,
            WorkerParentProvenance {
                thread_id: request.parent_thread_id,
                turn_id: Some(request.completed_turn_id),
            },
        )
    }
    #[tracing::instrument(level = "trace", skip(self))]
    pub fn render_refresh_queue_prompt(&self, request: &PlanningQueueRefreshRequest<'_>) -> String {
        // prompt rendering은 항상 가능한 최신 accepted authority snapshot을 포함하지만, rendering 자체는 state를 mutate하지 않는다.
        let authority_context = self.load_worker_authority_context(request.workspace_directory);
        match &request.mode {
            PlanningQueueRefreshMode::FromLatestMainReply => build_planning_queue_refresh_prompt(
                request.latest_user_message,
                request.latest_main_reply,
                request.previous_handoff_task,
                &authority_context,
            ),
            PlanningQueueRefreshMode::DeriveQueueHeadWhenQueueIdle {
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
    #[tracing::instrument(level = "trace", skip(self))]
    pub fn render_official_completion_refresh_prompt(
        &self,
        request: &PlanningOfficialCompletionRefreshRequest<'_>,
    ) -> String {
        // official completion prompt는 completion contract를 포함한다. worker가 latest visible text만 보지 않고
        // authoritative completion order를 기준으로 판단하게 하기 위해서다.
        let authority_context = self.load_worker_authority_context(request.workspace_directory);
        build_planning_official_completion_prompt(
            request.latest_user_message,
            request.latest_main_reply,
            request.previous_handoff_task,
            request.contract,
            &authority_context,
        )
    }
    #[tracing::instrument(level = "trace", skip(self))]
    pub fn render_repair_task_authority_prompt(
        &self,
        request: &PlanningLedgerRepairRequest<'_>,
    ) -> String {
        // repair prompt는 previous handoff를 작은 borrowed view로 변환한다. 전체 runtime handoff object를 clone할 필요를 없앤다.
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
    #[tracing::instrument(level = "trace", skip(self))]
    fn acquire_official_refresh_permit(
        &self,
        workspace_directory: &str,
        refresh_order: u64,
    ) -> Result<OfficialCompletionRefreshPermit> {
        // owner token에는 process/time entropy를 넣는다. 같은 order에 대한 반복 refresh loop도 authority store에서 구분된다.
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
                    // authority store가 refresh order별로 직렬화한다. caller는 이미 background planning refresh path에 있으므로
                    // 짧고 명시적인 wait을 수행한다.
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
    #[tracing::instrument(level = "trace", skip(self))]
    fn run_worker_and_reconcile(
        &self,
        workspace_directory: &str,
        orchestration_id: &str,
        operation: PlanningWorkerOperation,
        prompt: String,
        _previous_handoff: Option<&PlanningTaskHandoff>,
        parent_provenance: WorkerParentProvenance<'_>,
    ) -> Result<PlanningWorkerRunOutcome> {
        // worker execution 전에 execution snapshot을 capture한다. protected file reconciliation이 worker file change를
        // orchestration 시작 시점의 상태와 비교할 수 있게 하기 위해서다.
        event_log::emit_lazy("planning_worker_orchestration_started", || {
            orchestration_event_detail(
                workspace_directory,
                orchestration_id,
                operation,
                "started",
                Some("capture_execution_snapshot"),
                None,
                [
                    ("prompt_chars", json!(prompt.chars().count())),
                    ("has_previous_handoff", json!(_previous_handoff.is_some())),
                ],
            )
        });
        let execution_snapshot = match self
            .runtime_facade
            .load_execution_snapshot(workspace_directory)
        {
            Ok(snapshot) => snapshot,
            Err(error) => {
                event_log::emit_lazy("planning_worker_orchestration_failed", || {
                    orchestration_event_detail(
                        workspace_directory,
                        orchestration_id,
                        operation,
                        "load_execution_snapshot",
                        Some("abort"),
                        None,
                        [("error", json!(error.to_string()))],
                    )
                });
                return Err(error);
            }
        };
        // worker는 changed planning file과 final message를 모두 돌려줄 수 있다. accepted task authority를 mutate할 수 있는 것은
        // final message 안의 structured planning_task_commands뿐이다.
        let worker_response =
            match self
                .planning_worker_port
                .run_planning_session(PlanningWorkerRequest {
                    operation,
                    workspace_directory: workspace_directory.to_string(),
                    prompt,
                }) {
                Ok(response) => response,
                Err(error) => {
                    event_log::emit_lazy("planning_worker_orchestration_failed", || {
                        orchestration_event_detail(
                            workspace_directory,
                            orchestration_id,
                            operation,
                            "run_planning_session",
                            Some("abort"),
                            None,
                            [("error", json!(error.to_string()))],
                        )
                    });
                    return Err(error);
                }
            };
        let task_provenance = TaskMutationProvenance::new(OriginSessionKind::Planner)
            .with_thread_turn(
                worker_response.thread_id.clone(),
                worker_response.turn_id.clone(),
            )
            .with_parent(
                parent_provenance.thread_id.map(str::to_string),
                parent_provenance.turn_id.map(str::to_string),
            );
        let mut authority_result = PlanningReconciliationResult::default();
        let mut task_authority_changed = false;
        if let Some(final_message) = worker_response.final_agent_message.as_deref() {
            // accepted path는 command 기반이라 validation, conflict handling, queue projection rebuild가
            // PlanningTaskMutationService에 중앙화된다.
            match extract_planning_task_commands(final_message) {
                PlanningTaskCommandExtraction::Commands(commands) => {
                    match self
                        .task_mutation_service
                        .apply_commands(PlanningTaskMutationRequest {
                            workspace_directory: workspace_directory.to_string(),
                            source: PlanningTaskMutationSource::Worker,
                            legacy_source_turn_id: worker_response.turn_id.clone(),
                            provenance: task_provenance.clone(),
                            commands,
                        }) {
                        Ok(mutation_result) => {
                            task_authority_changed = mutation_result.task_authority_changed;
                            if mutation_result.task_authority_changed {
                                // mutation service가 projection을 이미 다시 만들었다. reconciliation result는 downstream notice를 위해 그 사실만 기록한다.
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
                PlanningTaskCommandExtraction::InvalidCommands {
                    error,
                    rejected_json,
                } => {
                    // invalid command JSON은 조용히 사라지지 않고 repair request가 된다. planning ledger drift가 operator와 retry loop에 보이게 한다.
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
        // command handling 뒤에도 file-level reconciliation은 실행된다. worker가 task command를 내지 않았어도 planning workspace file을
        // 건드렸을 수 있기 때문이다.
        let reconciliation_result = match self.runtime_facade.reconcile_after_turn(
            workspace_directory,
            orchestration_id,
            &worker_response.changed_planning_file_paths,
            &execution_snapshot,
        ) {
            Ok(result) => result,
            Err(error) => {
                event_log::emit_lazy("planning_worker_orchestration_failed", || {
                    orchestration_event_detail(
                        workspace_directory,
                        orchestration_id,
                        operation,
                        "reconcile_after_turn",
                        Some("abort"),
                        None,
                        [
                            (
                                "changed_planning_file_count",
                                json!(worker_response.changed_planning_file_paths.len()),
                            ),
                            ("error", json!(error.to_string())),
                        ],
                    )
                });
                return Err(error);
            }
        };
        let reconciliation_result =
            merge_reconciliation_results(authority_result, reconciliation_result);
        let runtime_projection =
            if let Some(block_reason) = reconciliation_result.auto_follow_block_reason.clone() {
                // reconciliation block은 reload로 가리지 않고 즉시 invalid runtime projection으로 표면화한다.
                PlanningRuntimeProjection::invalid(block_reason)
            } else {
                self.runtime_facade
                    .load_runtime_projection_or_invalid(workspace_directory)
            };
        let worker_summary = worker_response
            .final_agent_message
            .as_deref()
            .and_then(first_non_empty_line)
            .map(str::to_string);
        // UI caller는 full repair request를 풀지 않고도 짧은 줄이 필요하므로 rejected summary를 outcome에도 복제한다.
        let rejected_summary = reconciliation_result
            .repair_request
            .as_ref()
            .map(|request| request.failure_summary.clone());
        let mut notices = reconciliation_result.notices;
        if let Some(worker_summary) = worker_summary.as_deref() {
            notices.push(format!(
                "planning worker {} summary: {}",
                operation_label(operation),
                worker_summary
            ));
        }
        event_log::emit_lazy("planning_worker_orchestration_completed", || {
            orchestration_event_detail(
                workspace_directory,
                orchestration_id,
                operation,
                "completed",
                Some("return_outcome"),
                Some(&runtime_projection),
                [
                    (
                        "changed_planning_file_count",
                        json!(worker_response.changed_planning_file_paths.len()),
                    ),
                    ("task_authority_changed", json!(task_authority_changed)),
                    (
                        "repair_requested",
                        json!(reconciliation_result.repair_request.is_some()),
                    ),
                    (
                        "auto_followup_blocked",
                        json!(reconciliation_result.auto_follow_block_reason.is_some()),
                    ),
                    ("notices_count", json!(notices.len())),
                    ("has_worker_summary", json!(worker_summary.is_some())),
                ],
            )
        });
        Ok(PlanningWorkerRunOutcome {
            runtime_projection,
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
        // prompt authority context는 read-only이고 best-effort다. 두 DB snapshot이 모두 있으면 worker는 정확한 accepted
        // authority와 queue projection을 받고, 아니면 명시적인 load status를 받는다.
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
                // section을 생략하는 것보다 status-only context가 낫다. worker가 workspace file에서 authority를 추론하지 않게 알려 준다.
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
        // rejection packet은 accepted authority와 rejected payload를 함께 싣는다. repair worker가 둘을 비교해 ledger 전체
        // rewrite 대신 더 작은 valid command set을 낼 수 있게 한다.
        let mut result = PlanningReconciliationResult {
            rejected_task_authority: true,
            ..PlanningReconciliationResult::default()
        };
        // repair 품질은 현재 accepted authority에 의존하므로 이 load는 실패 가능성을 그대로 전파한다.
        // misleading empty context로 repair prompt를 만드는 것보다 명확하다.
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
    // compact status string은 prompt에 직접 들어간다. worker는 authority가 loaded/missing/unavailable 중 무엇인지 알아야 한다.
    match result {
        Ok(Some(_)) => "loaded".to_string(),
        Ok(None) => "missing".to_string(),
        Err(error) => format!("error: {error}"),
    }
}

fn authority_claim_owner_token(prefix: &str, nonce: u64) -> String {
    // token은 security-sensitive하지 않다. local concurrent refresh attempt 사이에서 claim/release bookkeeping을 위한
    // collision-resistant owner id다.
    let unique_suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    format!("{prefix}-{}-{nonce}-{unique_suffix}", std::process::id())
}

fn first_non_empty_line(text: &str) -> Option<&str> {
    // notice는 첫 non-empty worker line만 사용한다. full final agent message로 UI가 과도하게 길어지는 것을 막는다.
    text.lines().map(str::trim).find(|line| !line.is_empty())
}

fn merge_reconciliation_results(
    mut primary: PlanningReconciliationResult,
    secondary: PlanningReconciliationResult,
) -> PlanningReconciliationResult {
    // command reconciliation과 file reconciliation은 별도로 만들어진다. merge는 첫 non-empty repair/blocking decision을 보존하고,
    // 양쪽의 additive notice와 protected-file restoration detail은 함께 누적한다.
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
    primary.auto_follow_block_reason = primary
        .auto_follow_block_reason
        .or(secondary.auto_follow_block_reason);
    primary
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::{Arc, Mutex};

    use anyhow::{Result, anyhow};

    use super::*;
    use crate::application::port::outbound::planning_authority_port::NoopPlanningAuthorityPort;
    use crate::application::port::outbound::planning_task_repository_port::{
        NoopPlanningTaskRepositoryPort, PlanningDirectionAuthorityCommit,
        PlanningTaskAuthorityCommit, PlanningTaskRepositoryPort,
    };
    use crate::application::port::outbound::planning_worker_port::{
        PlanningWorkerRequest, PlanningWorkerResponse,
    };
    use crate::application::port::outbound::planning_workspace_port::{
        PlanningDraftFileRecord, PlanningDraftLoadRecord, PlanningDraftStageRecord,
        PlanningWorkspaceLoadRecord, PlanningWorkspacePort,
    };
    use crate::application::service::planning::repair::reconciliation::PlanningReconciliationService;
    use crate::application::service::planning::runtime::policy::PlanningRuntimePolicyService;
    use crate::application::service::planning::runtime::prompt::PlanningPromptService;
    use crate::application::service::planning::runtime::validation::PlanningValidationService;
    use crate::application::service::planning::shared::contract::RESULT_OUTPUT_FILE_PATH;
    use crate::application::service::turn_prompt_assembly_service::TurnPromptAssemblyService;
    use crate::domain::planning::{
        DirectionCatalogDocument, DirectionDefinition, DirectionState, OriginSessionKind,
        PLANNING_FORMAT_VERSION, PriorityQueueProjection, QueueIdleConfig, TaskActor,
        TaskAuthorityDocument,
    };

    static NEXT_WORKSPACE_ID: AtomicU64 = AtomicU64::new(1);

    #[derive(Default)]
    struct RecordingPlanningWorkerPort {
        response: Mutex<Option<PlanningWorkerResponse>>,
        requests: Mutex<Vec<PlanningWorkerRequest>>,
    }

    impl RecordingPlanningWorkerPort {
        fn new(response: PlanningWorkerResponse) -> Self {
            Self {
                response: Mutex::new(Some(response)),
                requests: Mutex::new(Vec::new()),
            }
        }

        fn requests(&self) -> Vec<PlanningWorkerRequest> {
            self.requests
                .lock()
                .expect("recorded worker requests should not be poisoned")
                .clone()
        }
    }

    impl PlanningWorkerPort for RecordingPlanningWorkerPort {
        fn run_planning_session(
            &self,
            request: PlanningWorkerRequest,
        ) -> Result<PlanningWorkerResponse> {
            self.requests
                .lock()
                .expect("recorded worker requests should not be poisoned")
                .push(request);
            self.response
                .lock()
                .expect("worker response should not be poisoned")
                .clone()
                .ok_or_else(|| anyhow!("test worker response was not configured"))
        }
    }

    #[derive(Default)]
    struct RecordingPlanningWorkspacePort {
        record: Mutex<PlanningWorkspaceLoadRecord>,
        commits: Mutex<Vec<PlanningWorkspaceLoadRecord>>,
        optional_files: Mutex<BTreeMap<String, String>>,
    }

    impl RecordingPlanningWorkspacePort {
        fn new(result_output_markdown: &str) -> Self {
            Self {
                record: Mutex::new(PlanningWorkspaceLoadRecord {
                    result_output_markdown: Some(result_output_markdown.to_string()),
                }),
                commits: Mutex::new(Vec::new()),
                optional_files: Mutex::new(BTreeMap::new()),
            }
        }

        fn commits(&self) -> Vec<PlanningWorkspaceLoadRecord> {
            self.commits
                .lock()
                .expect("recorded workspace commits should not be poisoned")
                .clone()
        }
    }

    impl PlanningWorkspacePort for RecordingPlanningWorkspacePort {
        fn stage_planning_draft_files(
            &self,
            _workspace_dir: &str,
            _draft_name: &str,
            _files: &[PlanningDraftFileRecord],
        ) -> Result<PlanningDraftStageRecord> {
            Err(anyhow!(
                "stage_planning_draft_files should not be used by orchestration tests"
            ))
        }

        fn load_planning_draft_files(
            &self,
            _workspace_dir: &str,
            _draft_name: &str,
        ) -> Result<PlanningDraftLoadRecord> {
            Err(anyhow!(
                "load_planning_draft_files should not be used by orchestration tests"
            ))
        }

        fn replace_planning_draft_file(
            &self,
            _workspace_dir: &str,
            _draft_name: &str,
            _active_path: &str,
            _body: &str,
        ) -> Result<String> {
            Err(anyhow!(
                "replace_planning_draft_file should not be used by orchestration tests"
            ))
        }

        fn load_planning_workspace_files(
            &self,
            _workspace_dir: &str,
        ) -> Result<PlanningWorkspaceLoadRecord> {
            Ok(self
                .record
                .lock()
                .expect("workspace record should not be poisoned")
                .clone())
        }

        fn load_planning_workspace_candidate_files(
            &self,
            _workspace_dir: &str,
        ) -> Result<PlanningWorkspaceLoadRecord> {
            Err(anyhow!(
                "load_planning_workspace_candidate_files should not be used by orchestration tests"
            ))
        }

        fn commit_planning_workspace_files(
            &self,
            _workspace_dir: &str,
            record: &PlanningWorkspaceLoadRecord,
        ) -> Result<()> {
            *self
                .record
                .lock()
                .expect("workspace record should not be poisoned") = record.clone();
            self.commits
                .lock()
                .expect("recorded workspace commits should not be poisoned")
                .push(record.clone());
            Ok(())
        }

        fn load_optional_planning_file(
            &self,
            _workspace_dir: &str,
            relative_path: &str,
        ) -> Result<Option<String>> {
            Ok(self
                .optional_files
                .lock()
                .expect("optional planning file map should not be poisoned")
                .get(relative_path)
                .cloned())
        }

        fn load_optional_planning_candidate_file(
            &self,
            _workspace_dir: &str,
            _relative_path: &str,
        ) -> Result<Option<String>> {
            Err(anyhow!(
                "load_optional_planning_candidate_file should not be used by orchestration tests"
            ))
        }

        fn replace_planning_workspace_file(
            &self,
            _workspace_dir: &str,
            _relative_path: &str,
            _body: Option<&str>,
        ) -> Result<()> {
            Err(anyhow!(
                "replace_planning_workspace_file should not be used by orchestration tests"
            ))
        }

        fn remove_planning_workspace_entry(
            &self,
            _workspace_dir: &str,
            _relative_path: &str,
        ) -> Result<()> {
            Err(anyhow!(
                "remove_planning_workspace_entry should not be used by orchestration tests"
            ))
        }

        fn archive_rejected_planning_file(
            &self,
            _workspace_dir: &str,
            _archive_name: &str,
            _active_path: &str,
            _body: &str,
        ) -> Result<String> {
            Err(anyhow!(
                "archive_rejected_planning_file should not be used by orchestration tests"
            ))
        }
    }

    #[test]
    fn refresh_worker_commits_task_commands_and_restores_protected_files() {
        let workspace = workspace("command-commit");
        let repo = Arc::new(NoopPlanningTaskRepositoryPort);
        seed_authority(repo.as_ref(), &workspace);
        let workspace_port = Arc::new(RecordingPlanningWorkspacePort::new(
            "# Result Output\n- Summarize completed work.",
        ));
        let worker_message = r#"Worker planned follow-up.

```json
{"planning_task_commands":{"version":1,"commands":[{"op":"create_task","title":"Cover worker orchestration","description":"Exercise orchestration command commit and reconciliation.","direction_relation_note":"keeps worker orchestration covered"}]}}
```"#;
        let worker = Arc::new(RecordingPlanningWorkerPort::new(PlanningWorkerResponse {
            operation: PlanningWorkerOperation::RefreshQueue,
            thread_id: Some("worker-thread-1".to_string()),
            turn_id: Some("worker-turn-1".to_string()),
            final_agent_message: Some(worker_message.to_string()),
            changed_planning_file_paths: vec![RESULT_OUTPUT_FILE_PATH.to_string()],
        }));
        let service = orchestration_service(worker.clone(), workspace_port.clone(), repo.clone());

        let outcome = service
            .refresh_queue_from_reply(PlanningQueueRefreshRequest {
                workspace_directory: &workspace,
                parent_thread_id: Some("parent-thread-1"),
                completed_turn_id: "parent-turn-1",
                latest_user_message: Some("please continue"),
                latest_main_reply: "done",
                previous_handoff_task: None,
                mode: PlanningQueueRefreshMode::FromLatestMainReply,
            })
            .expect("worker refresh should succeed");

        assert!(outcome.task_authority_changed);
        assert_eq!(
            outcome.worker_summary.as_deref(),
            Some("Worker planned follow-up.")
        );
        assert_eq!(outcome.worker_response.as_deref(), Some(worker_message));
        assert!(outcome.repair_request.is_none());
        assert!(outcome.rejected_summary.is_none());
        assert!(
            outcome
                .notices
                .iter()
                .any(|notice| notice == "planning worker committed 1 task command(s)")
        );
        assert!(
            outcome
                .notices
                .iter()
                .any(|notice| notice
                    == "planning reconciliation restored protected planning files")
        );
        assert!(
            outcome.notices.iter().any(
                |notice| notice == "planning worker refresh summary: Worker planned follow-up."
            )
        );
        assert_eq!(workspace_port.commits().len(), 1);
        assert_eq!(
            workspace_port.commits()[0]
                .result_output_markdown
                .as_deref(),
            Some("# Result Output\n- Summarize completed work.")
        );

        let committed = repo
            .load_task_authority_snapshot(&workspace)
            .expect("task snapshot should load")
            .expect("task snapshot should exist");
        assert_eq!(committed.task_authority.tasks.len(), 1);
        let task = &committed.task_authority.tasks[0];
        assert_eq!(task.title, "Cover worker orchestration");
        assert_eq!(task.created_by, TaskActor::Worker);
        assert_eq!(task.last_updated_by, TaskActor::Worker);
        assert_eq!(task.source_turn_id.as_deref(), Some("worker-turn-1"));
        assert_eq!(
            task.provenance.origin_session_kind,
            Some(OriginSessionKind::Planner)
        );
        assert_eq!(
            task.provenance.thread_id.as_deref(),
            Some("worker-thread-1")
        );
        assert_eq!(task.provenance.turn_id.as_deref(), Some("worker-turn-1"));
        assert_eq!(
            task.provenance.parent_thread_id.as_deref(),
            Some("parent-thread-1")
        );
        assert_eq!(
            task.provenance.parent_turn_id.as_deref(),
            Some("parent-turn-1")
        );

        let requests = worker.requests();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].operation, PlanningWorkerOperation::RefreshQueue);
        assert_eq!(requests[0].workspace_directory, workspace);
        assert!(requests[0].prompt.contains("please continue"));
        assert!(requests[0].prompt.contains("source_of_truth=accepted DB"));
    }

    #[test]
    fn invalid_worker_task_commands_build_repair_request_without_mutating_authority() {
        let workspace = workspace("invalid-command");
        let repo = Arc::new(NoopPlanningTaskRepositoryPort);
        seed_authority(repo.as_ref(), &workspace);
        let workspace_port = Arc::new(RecordingPlanningWorkspacePort::new(
            "# Result Output\n- Summarize completed work.",
        ));
        let worker_message = r#"The worker tried to update planning.

```json
{"planning_task_commands":{"version":1,"commands":[{"create_task":{"title":"Missing op"}}]}}
```"#;
        let worker = Arc::new(RecordingPlanningWorkerPort::new(PlanningWorkerResponse {
            operation: PlanningWorkerOperation::RefreshQueue,
            thread_id: Some("worker-thread-2".to_string()),
            turn_id: Some("worker-turn-2".to_string()),
            final_agent_message: Some(worker_message.to_string()),
            changed_planning_file_paths: Vec::new(),
        }));
        let service = orchestration_service(worker, workspace_port.clone(), repo.clone());

        let outcome = service
            .refresh_queue_from_reply(PlanningQueueRefreshRequest {
                workspace_directory: &workspace,
                parent_thread_id: Some("parent-thread-2"),
                completed_turn_id: "parent-turn-2",
                latest_user_message: None,
                latest_main_reply: "done",
                previous_handoff_task: None,
                mode: PlanningQueueRefreshMode::FromLatestMainReply,
            })
            .expect("invalid command payload should be converted into repair request");

        assert!(!outcome.task_authority_changed);
        assert_eq!(
            outcome.worker_summary.as_deref(),
            Some("The worker tried to update planning.")
        );
        assert!(outcome.rejected_summary.is_some_and(|summary| {
            summary.contains("planning worker returned invalid planning_task_commands")
                && summary.contains("missing field `op`")
        }));
        let repair_request = outcome
            .repair_request
            .expect("invalid commands should produce a repair request");
        assert!(
            repair_request
                .failure_summary
                .contains("missing field `op`")
        );
        assert!(
            repair_request
                .accepted_task_authority_json
                .contains("\"tasks\": []")
        );
        assert!(
            repair_request
                .accepted_queue_projection_json
                .contains("\"next_task\": null")
        );
        assert!(
            repair_request
                .rejected_task_authority_json
                .as_deref()
                .is_some_and(|payload| payload.contains("\"planning_task_commands\""))
        );
        assert!(
            outcome
                .notices
                .iter()
                .any(|notice| notice.contains("missing field `op`"))
        );
        assert!(workspace_port.commits().is_empty());
        assert!(
            repo.load_task_authority_snapshot(&workspace)
                .expect("task snapshot should load")
                .expect("task snapshot should exist")
                .task_authority
                .tasks
                .is_empty()
        );
    }

    #[test]
    fn authority_status_and_reconciliation_merge_keep_operational_details() {
        assert_eq!(authority_load_status::<()>(Ok(Some(()))), "loaded");
        assert_eq!(authority_load_status::<()>(Ok(None)), "missing");
        assert_eq!(
            authority_load_status::<()>(Err(anyhow!("db unavailable"))),
            "error: db unavailable"
        );

        let primary = PlanningReconciliationResult {
            notices: vec!["authority notice".to_string()],
            rejected_task_authority: true,
            queue_projection_action: Some(
                crate::application::service::planning::repair::reconciliation::PlanningQueueProjectionAction::RebuiltFromAcceptedPlanning,
            ),
            auto_follow_block_reason: Some("authority blocked".to_string()),
            ..PlanningReconciliationResult::default()
        };
        let secondary = PlanningReconciliationResult {
            notices: vec!["file notice".to_string()],
            rejected_task_authority: false,
            auto_follow_block_reason: Some("file blocked".to_string()),
            ..PlanningReconciliationResult::default()
        };

        let merged = merge_reconciliation_results(primary, secondary);

        assert_eq!(
            merged.notices,
            vec!["authority notice".to_string(), "file notice".to_string()]
        );
        assert!(merged.rejected_task_authority);
        assert_eq!(
            merged.auto_follow_block_reason.as_deref(),
            Some("authority blocked")
        );
        assert!(merged.queue_projection_action.is_some());
    }

    fn orchestration_service(
        worker: Arc<dyn PlanningWorkerPort>,
        workspace_port: Arc<dyn PlanningWorkspacePort>,
        repo: Arc<NoopPlanningTaskRepositoryPort>,
    ) -> PlanningWorkerOrchestrationService {
        let validation = PlanningValidationService::new();
        let priority_queue = crate::domain::planning::PriorityQueueService::new();
        let prompt = PlanningPromptService::with_task_repository(
            workspace_port.clone(),
            validation.clone(),
            priority_queue.clone(),
            repo.clone(),
        );
        let reconciliation = PlanningReconciliationService::with_task_repository(
            workspace_port,
            validation,
            priority_queue,
            repo.clone(),
        );
        let runtime_facade = PlanningRuntimeFacadeService::new(
            prompt,
            reconciliation,
            PlanningRuntimePolicyService::new(),
            TurnPromptAssemblyService::new(),
        );
        PlanningWorkerOrchestrationService::new(
            worker,
            runtime_facade,
            Arc::new(NoopPlanningAuthorityPort::default()),
            repo,
        )
    }

    fn workspace(label: &str) -> String {
        format!(
            "/tmp/akra-planning-worker-orchestration-{label}-{}-{}",
            std::process::id(),
            NEXT_WORKSPACE_ID.fetch_add(1, Ordering::Relaxed)
        )
    }

    fn seed_authority(repo: &NoopPlanningTaskRepositoryPort, workspace: &str) {
        repo.clear_direction_authority_snapshot(workspace)
            .expect("direction snapshot should clear");
        repo.clear_task_authority_snapshot(workspace)
            .expect("task snapshot should clear");
        repo.commit_direction_authority_snapshot(
            workspace,
            PlanningDirectionAuthorityCommit {
                observed_planning_revision: None,
                directions: &directions(),
            },
        )
        .expect("direction snapshot should commit");
        repo.commit_task_authority_snapshot(
            workspace,
            PlanningTaskAuthorityCommit {
                observed_planning_revision: None,
                task_authority: &TaskAuthorityDocument {
                    version: PLANNING_FORMAT_VERSION,
                    tasks: Vec::new(),
                },
                queue_projection: &PriorityQueueProjection {
                    next_task: None,
                    active_tasks: Vec::new(),
                    proposed_tasks: Vec::new(),
                    skipped_tasks: Vec::new(),
                },
            },
        )
        .expect("task snapshot should commit");
    }

    fn directions() -> DirectionCatalogDocument {
        DirectionCatalogDocument {
            version: PLANNING_FORMAT_VERSION,
            queue_idle: QueueIdleConfig::default(),
            directions: vec![DirectionDefinition {
                id: "general-workstream".to_string(),
                title: "General".to_string(),
                summary: "Handle general planning work.".to_string(),
                success_criteria: vec!["done".to_string()],
                scope_hints: Vec::new(),
                detail_doc_path: String::new(),
                state: DirectionState::Active,
            }],
        }
    }
}
