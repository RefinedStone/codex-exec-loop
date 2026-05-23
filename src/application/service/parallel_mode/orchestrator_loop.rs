use crate::application::port::outbound::parallel_agent_worker_port::{
    ParallelAgentWorkerPort, ParallelAgentWorkerStreamRequest,
};
use crate::application::service::conversation_runtime_event::ConversationStreamEvent;
use crate::application::service::parallel_agent_profile::load_parallel_agent_profile_config;
use crate::application::service::parallel_mode::turn::ParallelModeTurnService;
use crate::application::service::planning::{
    PlanningOfficialCompletionRefreshRequest, PlanningRuntimeProjection,
    PlanningRuntimeWorkspaceStatus, PlanningServices, PlanningTaskHandoff,
};
use crate::diagnostics::event_log;
use crate::domain::parallel_mode::{
    ParallelModeAutomationTrigger, ParallelModeControlPlaneWorkerEvent,
    ParallelModeControlPlaneWorkerEventKind, ParallelModeDispatchCommandSnapshot,
    ParallelModeDispatchOutcome, ParallelModeReadinessSnapshot, ParallelModeRuntimeEvent,
    ParallelModeSlotLeaseRequest, ParallelModeSupervisorSnapshot,
};
use chrono::Utc;
use std::collections::BTreeSet;
use std::sync::Arc;
use std::sync::mpsc::{self, Sender};
use std::thread;

use super::ParallelModeService;
use super::pool::load_pool_runtime_context;

pub struct ParallelModeDispatchOrchestratorTickRequest {
    pub workspace_directory: String,
    pub trigger: ParallelModeAutomationTrigger,
    pub epoch_id: u64,
    pub enqueue_trigger: Option<ParallelModeAutomationTrigger>,
    pub planning: PlanningServices,
    pub worker_port: Arc<dyn ParallelAgentWorkerPort>,
    pub turn_service: ParallelModeTurnService,
    pub event_sender: Sender<ParallelModeOrchestratorLoopEvent>,
}

#[derive(Debug, Clone)]
pub struct ParallelModeDispatchOrchestratorTickResult {
    pub workspace_directory: String,
    pub readiness_snapshot: ParallelModeReadinessSnapshot,
    pub supervisor_snapshot: ParallelModeSupervisorSnapshot,
    pub outcome: ParallelModeDispatchOutcome,
}

#[derive(Debug, Clone)]
pub enum ParallelModeOrchestratorLoopEvent {
    ConversationRuntimeNotice(String),
    WorkerEvent(ParallelModeControlPlaneWorkerEvent),
}

impl ParallelModeService {
    pub fn enqueue_dispatch_commands_for_trigger(
        &self,
        workspace_dir: &str,
        trigger: ParallelModeAutomationTrigger,
        planning_projection: &PlanningRuntimeProjection,
        epoch_id: Option<u64>,
    ) -> Result<usize, String> {
        self.enqueue_dispatch_commands_for_event(
            workspace_dir,
            parallel_runtime_event_for_dispatch_trigger(trigger),
            planning_projection,
            epoch_id,
        )
    }

    pub fn run_dispatch_orchestrator_tick(
        &self,
        request: ParallelModeDispatchOrchestratorTickRequest,
    ) -> ParallelModeDispatchOrchestratorTickResult {
        let workspace_directory = request.workspace_directory;
        let planning_projection = request
            .planning
            .runtime
            .load_runtime_projection_or_invalid(&workspace_directory);
        let readiness_snapshot = self.inspect_readiness(&workspace_directory, &planning_projection);

        let (supervisor_snapshot, outcome) = if readiness_snapshot.allows_parallel_mode() {
            if let Some(enqueue_trigger) = request.enqueue_trigger {
                let runtime_event = parallel_runtime_event_for_dispatch_trigger(enqueue_trigger);
                if let Err(error) = self.enqueue_dispatch_commands_for_event(
                    &workspace_directory,
                    runtime_event,
                    &planning_projection,
                    Some(request.epoch_id),
                ) {
                    event_log::emit_lazy("parallel_dispatch_command_enqueue_failed", || {
                        serde_json::json!({
                            "trigger": enqueue_trigger.label(),
                            "workspace": &workspace_directory,
                            "epoch_id": request.epoch_id,
                            "error": error,
                        })
                    });
                }
            }
            let outcome = match self.claim_next_dispatch_command(&workspace_directory) {
                Ok(Some(mut command)) => {
                    let outcome = dispatch_parallel_queue_pool(
                        self,
                        ParallelModeDispatchExecutionContext {
                            workspace_directory: &workspace_directory,
                            planning_projection: &planning_projection,
                            worker_port: request.worker_port,
                            turn_service: request.turn_service,
                            planning: request.planning,
                            event_sender: request.event_sender.clone(),
                            trigger: command.trigger,
                            epoch_id: request.epoch_id,
                        },
                    );
                    persist_dispatch_command_outcome(
                        self,
                        &workspace_directory,
                        &mut command,
                        &outcome,
                    );
                    outcome
                }
                Ok(None) => {
                    let mut outcome = ParallelModeDispatchOutcome::new(
                        request.trigger,
                        workspace_directory.clone(),
                        request.epoch_id,
                    );
                    outcome.blocked_reason =
                        Some("no pending durable dispatch command".to_string());
                    outcome.status_copy_input = outcome.status_detail();
                    outcome
                }
                Err(error) => {
                    let mut outcome = ParallelModeDispatchOutcome::new(
                        request.trigger,
                        workspace_directory.clone(),
                        request.epoch_id,
                    );
                    outcome.blocked_reason =
                        Some(format!("dispatch command claim failed: {error}"));
                    outcome.status_copy_input = outcome.status_detail();
                    outcome
                }
            };
            let supervisor_snapshot = self.build_supervisor_snapshot(
                &workspace_directory,
                true,
                Some(&readiness_snapshot),
            );
            (supervisor_snapshot, outcome)
        } else {
            let supervisor_snapshot = self.build_supervisor_snapshot(
                &workspace_directory,
                false,
                Some(&readiness_snapshot),
            );
            let cause = readiness_snapshot
                .top_alert
                .as_deref()
                .unwrap_or("inspect the readiness panel before retrying");
            let mut outcome = ParallelModeDispatchOutcome::new(
                request.trigger,
                workspace_directory.clone(),
                request.epoch_id,
            );
            outcome.blocked_reason = Some(format!(
                "readiness: {} / {cause}",
                readiness_snapshot.readiness_label()
            ));
            outcome.status_copy_input = outcome.blocked_reason.clone().unwrap_or_default();
            (supervisor_snapshot, outcome)
        };

        ParallelModeDispatchOrchestratorTickResult {
            workspace_directory,
            readiness_snapshot,
            supervisor_snapshot,
            outcome,
        }
    }
}

struct ParallelModeDispatchExecutionContext<'a> {
    workspace_directory: &'a str,
    planning_projection: &'a PlanningRuntimeProjection,
    worker_port: Arc<dyn ParallelAgentWorkerPort>,
    turn_service: ParallelModeTurnService,
    planning: PlanningServices,
    event_sender: Sender<ParallelModeOrchestratorLoopEvent>,
    trigger: ParallelModeAutomationTrigger,
    epoch_id: u64,
}

fn dispatch_parallel_queue_pool(
    service: &ParallelModeService,
    context: ParallelModeDispatchExecutionContext<'_>,
) -> ParallelModeDispatchOutcome {
    /*
     * Dispatch is the handoff bridge from planning queue to parallel worker.
     * The service chooses candidates, leases slots, assembles handoffs, and starts
     * worker execution through the existing worker port. Inbound adapters only wake
     * this loop and project its result.
     */
    let workspace_directory = context.workspace_directory;
    let trigger = context.trigger;
    let epoch_id = context.epoch_id;
    let mut outcome =
        ParallelModeDispatchOutcome::new(trigger, workspace_directory.to_string(), epoch_id);

    let dispatch_plan = match service.build_dispatch_plan(
        workspace_directory,
        context.planning_projection,
        usize::MAX,
    ) {
        Ok(plan) => plan,
        Err(error) => {
            outcome.blocked_reason = Some(error);
            outcome.status_copy_input = outcome.status_detail();
            event_log::emit_lazy("parallel_dispatch_blocked", || {
                serde_json::json!({
                    "trigger": trigger.label(),
                    "workspace": workspace_directory,
                    "epoch_id": epoch_id,
                    "blocked_reason": outcome.blocked_reason,
                })
            });
            return outcome;
        }
    };
    outcome.idle_slot_count = dispatch_plan.idle_slot_count;
    outcome.candidate_task_ids = dispatch_plan
        .candidates
        .iter()
        .map(|task| task.task_id.clone())
        .collect();
    event_log::emit_lazy("parallel_dispatch_plan_built", || {
        serde_json::json!({
            "trigger": trigger.label(),
            "workspace": workspace_directory,
            "epoch_id": epoch_id,
            "idle_slot_count": dispatch_plan.idle_slot_count,
            "candidate_task_ids": &outcome.candidate_task_ids,
            "excluded_task_ids": &dispatch_plan.excluded_task_ids,
        })
    });
    if dispatch_plan.idle_slot_count == 0 {
        outcome.blocked_reason = Some("no idle slot is available for auto dispatch".to_string());
        outcome.status_copy_input = outcome.status_detail();
        event_log::emit_lazy("parallel_dispatch_blocked", || {
            serde_json::json!({
                "trigger": trigger.label(),
                "workspace": workspace_directory,
                "epoch_id": epoch_id,
                "idle_slot_count": outcome.idle_slot_count,
                "task_ids": outcome.candidate_task_ids,
                "blocked_reason": outcome.blocked_reason,
            })
        });
        return outcome;
    }
    if dispatch_plan.candidates.is_empty() {
        let reason = if dispatch_plan.excluded_task_ids.is_empty() {
            "no actionable queue task to auto dispatch".to_string()
        } else {
            format!(
                "no undispatched queue task available for auto dispatch / excluded: {}",
                dispatch_plan.excluded_task_ids.join(", ")
            )
        };
        outcome.blocked_reason = Some(reason);
        outcome.status_copy_input = outcome.status_detail();
        event_log::emit_lazy("parallel_dispatch_blocked", || {
            serde_json::json!({
                "trigger": trigger.label(),
                "workspace": workspace_directory,
                "epoch_id": epoch_id,
                "idle_slot_count": outcome.idle_slot_count,
                "task_ids": outcome.candidate_task_ids,
                "blocked_reason": outcome.blocked_reason,
            })
        });
        return outcome;
    }

    let mut launched_titles = Vec::new();
    let mut blocked_details = Vec::new();
    let agent_profiles =
        load_parallel_agent_profile_config(workspace_directory).unwrap_or_default();
    let mut used_agent_ids = active_parallel_agent_ids(service, workspace_directory);
    for task in dispatch_plan.candidates {
        let selected_profile = agent_profiles.select_available_profile(&used_agent_ids);
        if let Some(profile) = selected_profile.as_ref() {
            used_agent_ids.insert(profile.agent_id.clone());
        }
        let handoff = selected_profile
            .as_ref()
            .map(|profile| {
                context
                    .planning
                    .runtime
                    .build_sub_session_task_handoff_with_agent_profile(&task, profile)
            })
            .unwrap_or_else(|| {
                context
                    .planning
                    .runtime
                    .build_sub_session_task_handoff(&task)
            });
        let lease_request = if let Some(profile) = selected_profile.as_ref() {
            ParallelModeSlotLeaseRequest::from_task_identity_with_agent_id(
                &handoff.task.task_id,
                &handoff.task.task_title,
                profile.agent_id.clone(),
            )
        } else {
            ParallelModeSlotLeaseRequest::from_task_identity(
                &handoff.task.task_id,
                &handoff.task.task_title,
            )
        };
        match service.acquire_slot_lease(workspace_directory, lease_request) {
            Ok(lease) => {
                event_log::emit_lazy("parallel_dispatch_slot_lease_acquired", || {
                    serde_json::json!({
                        "trigger": trigger.label(),
                        "workspace": workspace_directory,
                        "epoch_id": epoch_id,
                        "slot_id": &lease.slot_id,
                        "agent_id": &lease.agent_id,
                        "task_id": &handoff.task.task_id,
                        "task_title": &handoff.task.task_title,
                        "agent_profile_id": selected_profile.as_ref().map(|profile| profile.agent_id.as_str()),
                        "worktree": &lease.worktree_path,
                        "service_name": &handoff.service_name,
                        "prompt_chars": handoff.prompt.chars().count(),
                        "developer_instructions_chars": handoff.developer_instructions.chars().count(),
                    })
                });
                let worker_request = ParallelDispatchWorkerRequest {
                    planning_workspace_directory: workspace_directory.to_string(),
                    worktree_directory: lease.worktree_path.clone(),
                    automation_epoch_id: epoch_id,
                    prompt: handoff.prompt,
                    developer_instructions: handoff.developer_instructions,
                    service_name: handoff.service_name,
                    handoff_task: handoff.task.clone(),
                };
                spawn_parallel_dispatch_worker(
                    worker_request,
                    context.worker_port.clone(),
                    context.turn_service.clone(),
                    context.planning.clone(),
                    context.event_sender.clone(),
                );
                outcome.launched_task_ids.push(handoff.task.task_id.clone());
                launched_titles.push(handoff.task.task_title);
            }
            Err(error) => blocked_details.push(format!("{}: {error}", handoff.task.task_id)),
        }
    }
    let launched_count = launched_titles.len();
    if launched_count == 0 {
        outcome.blocked_reason = Some(format!(
            "worker launch blocked / {}",
            blocked_details.join(" | ")
        ));
        outcome.status_copy_input = outcome.status_detail();
        event_log::emit_lazy("parallel_dispatch_blocked", || {
            serde_json::json!({
                "trigger": trigger.label(),
                "workspace": workspace_directory,
                "epoch_id": epoch_id,
                "idle_slot_count": outcome.idle_slot_count,
                "task_ids": outcome.candidate_task_ids,
                "blocked_reason": outcome.blocked_reason,
            })
        });
        return outcome;
    }

    let mut status = format!(
        "auto dispatched {launched_count} worker(s) / tasks: {}",
        launched_titles.join(" | ")
    );
    if !blocked_details.is_empty() {
        status.push_str(&format!(" / blocked: {}", blocked_details.join(" | ")));
    }
    outcome.status_copy_input = status;
    event_log::emit_lazy("parallel_dispatch_launched", || {
        serde_json::json!({
            "trigger": trigger.label(),
            "workspace": workspace_directory,
            "epoch_id": epoch_id,
            "idle_slot_count": outcome.idle_slot_count,
            "task_ids": outcome.candidate_task_ids,
            "launched_count": outcome.launched_task_ids.len(),
        })
    });
    outcome
}

fn active_parallel_agent_ids(
    service: &ParallelModeService,
    workspace_directory: &str,
) -> BTreeSet<String> {
    load_pool_runtime_context(service.planning_authority.as_ref(), workspace_directory)
        .map(|context| {
            context
                .slot_leases
                .values()
                .map(|lease| lease.agent_id.clone())
                .collect()
        })
        .unwrap_or_default()
}

fn parallel_runtime_event_for_dispatch_trigger(
    trigger: ParallelModeAutomationTrigger,
) -> ParallelModeRuntimeEvent {
    match trigger {
        ParallelModeAutomationTrigger::MainTurnPostEvaluation => {
            ParallelModeRuntimeEvent::AutoFollowQueued
        }
        ParallelModeAutomationTrigger::ParallelOfficialCompletion => {
            ParallelModeRuntimeEvent::ParallelCompletionFinalized
        }
        ParallelModeAutomationTrigger::TaskIntakeAfterEpoch => {
            ParallelModeRuntimeEvent::TaskIntakeCommitted
        }
    }
}

fn persist_dispatch_command_outcome(
    service: &ParallelModeService,
    workspace_directory: &str,
    command: &mut ParallelModeDispatchCommandSnapshot,
    outcome: &ParallelModeDispatchOutcome,
) {
    let timestamp = Utc::now().to_rfc3339();
    if outcome.blocked_reason.is_some() && outcome.launched_task_ids.is_empty() {
        command.mark_blocked(outcome.status_detail(), timestamp);
    } else {
        command.mark_completed(outcome.status_detail(), timestamp);
    }
    if let Err(error) = service.update_dispatch_command(workspace_directory, command) {
        event_log::emit_lazy("parallel_dispatch_command_update_failed", || {
            serde_json::json!({
                "workspace": workspace_directory,
                "command_id": &command.command_id,
                "state": command.state.label(),
                "error": error,
            })
        });
    }
}

/* 병렬 슬롯 워커는 TUI 스레드 밖에서 Codex 세션 스트림을 끝까지 소비하고,
 * 그 결과를 다시 슬롯 상태와 planning 권위 파일 갱신으로 접속한다. 이 파일의
 * 경계는 UI 이벤트 처리보다 넓어서, 실패를 런타임 notice로 남기면서도 마지막에는
 * supervisor snapshot 무효화를 반드시 보내는 것이 호출 계약이다.
 */
#[derive(Debug, Clone)]
struct ParallelDispatchWorkerRequest {
    // planning workspace는 official completion refresh가 반영될 authoritative root이다.
    planning_workspace_directory: String,
    // worktree는 실제 isolated Codex turn이 실행되는 slot checkout이다.
    worktree_directory: String,
    // automation epoch lets the UI drop delayed completion chaining after :parallel off.
    automation_epoch_id: u64,
    // prompt는 queue head handoff를 worker thread에 전달하는 최종 user-facing 입력이다.
    prompt: String,
    // developer_instructions/service_name은 application prompt assembly가 정한 app-server thread metadata다.
    developer_instructions: String,
    service_name: String,
    // handoff_task는 notice, completion contract, refresh prompt가 같은 task를 가리키게 하는 연결 키이다.
    handoff_task: PlanningTaskHandoff,
}

// 스트림 이벤트는 순서대로 오지만, 최종 판단에는 "시작 전 실패", "실패 이벤트",
// "TurnCompleted", "마지막 답변"을 한 번에 보존해야 한다.
#[derive(Debug, Clone, Default)]
struct ParallelDispatchWorkerStreamState {
    /*
     * started 여부와 failed-before-started 여부를 둘 다 보관한다. 같은 Failed 이벤트라도 thread가
     * 시작된 뒤의 실패는 running slot completion 실패이고, 시작 전 실패는 lease를 release할 수
     * 있는 unstarted-slot 실패로 처리해야 하기 때문이다.
     */
    saw_turn_started: bool,
    saw_failed_before_turn_started: bool,
    saw_failed_event: bool,
    /*
     * TurnCompleted는 official completion refresh의 유일한 성공 입구다. app-server stream이
     * 답변 text를 끝냈더라도 TurnCompleted가 없으면 changed planning files와 turn id가 없어
     * authority ledger에 안전하게 completion contract를 남길 수 없다.
     */
    turn_completed: Option<ParallelDispatchTurnCompleted>,
    // main reply는 official completion prompt의 증거 문맥으로 쓰되, slot 성공 판정 자체는 TurnCompleted가 맡는다.
    latest_main_reply: Option<String>,
}
#[derive(Debug, Clone)]
struct ParallelDispatchTurnCompleted {
    turn_id: String,
    changed_planning_file_paths: Vec<String>,
}

struct ParallelDispatchWorkerRunResult {
    notices: Vec<String>,
    worker_event_kind: ParallelModeControlPlaneWorkerEventKind,
}

struct ParallelDispatchOfficialCompletionOutcome {
    notices: Vec<String>,
    official_completion_refresh_succeeded: bool,
}

impl ParallelDispatchOfficialCompletionOutcome {
    fn failed(notices: Vec<String>) -> Self {
        Self {
            notices,
            official_completion_refresh_succeeded: false,
        }
    }

    fn succeeded(notices: Vec<String>) -> Self {
        Self {
            notices,
            official_completion_refresh_succeeded: true,
        }
    }
}

impl ParallelDispatchWorkerRunResult {
    fn launch_failed(notices: Vec<String>) -> Self {
        Self {
            notices,
            worker_event_kind: ParallelModeControlPlaneWorkerEventKind::LaunchFailed,
        }
    }

    fn stream_failed(notices: Vec<String>) -> Self {
        Self {
            notices,
            worker_event_kind: ParallelModeControlPlaneWorkerEventKind::StreamFailed,
        }
    }

    fn completed(notices: Vec<String>) -> Self {
        Self {
            notices,
            worker_event_kind: ParallelModeControlPlaneWorkerEventKind::Completed,
        }
    }
}

fn spawn_parallel_dispatch_worker(
    request: ParallelDispatchWorkerRequest,
    worker_port: Arc<dyn ParallelAgentWorkerPort>,
    turn_service: ParallelModeTurnService,
    planning: PlanningServices,
    outer_tx: Sender<ParallelModeOrchestratorLoopEvent>,
) {
    thread::spawn(move || {
        /*
         * Background worker는 TUI event loop를 직접 만지지 않는다. 모든 결과는 notice message와
         * supervisor snapshot invalidation으로 되돌아가며, sender 실패는 이미 UI가 내려가는 중이라는
         * 의미라 worker thread 안에서 추가 복구를 시도하지 않는다.
         */
        let workspace_directory = request.planning_workspace_directory.clone();
        let automation_epoch_id = request.automation_epoch_id;
        let task_id = request.handoff_task.task_id.clone();
        let task_title = request.handoff_task.task_title.clone();
        event_log::emit_lazy("parallel_worker_thread_started", || {
            parallel_worker_thread_started_trace_payload(&request)
        });
        let result = run_parallel_dispatch_worker(request, worker_port, turn_service, planning);
        let _ = outer_tx.send(ParallelModeOrchestratorLoopEvent::WorkerEvent(
            ParallelModeControlPlaneWorkerEvent::new(
                workspace_directory,
                automation_epoch_id,
                task_id,
                task_title,
                result.worker_event_kind,
                result.notices,
            ),
        ));
    });
}

fn run_parallel_dispatch_worker(
    request: ParallelDispatchWorkerRequest,
    worker_port: Arc<dyn ParallelAgentWorkerPort>,
    turn_service: ParallelModeTurnService,
    planning: PlanningServices,
) -> ParallelDispatchWorkerRunResult {
    let (event_tx, event_rx) = mpsc::channel();
    let service_request = request.clone();
    event_log::emit_lazy("parallel_worker_stream_starting", || {
        parallel_worker_stream_starting_trace_payload(&request)
    });
    let service_thread = thread::spawn(move || {
        /*
         * ParallelAgentWorkerPort owns app-server execution. This outer worker keeps
         * the receiver side so it can reduce stream events while the isolated worker
         * is still running, then joins to capture transport-level errors.
         */
        worker_port.run_isolated_new_thread_stream(
            ParallelAgentWorkerStreamRequest {
                cwd: &service_request.worktree_directory,
                prompt: &service_request.prompt,
                developer_instructions: &service_request.developer_instructions,
                service_name: &service_request.service_name,
            },
            event_tx,
        )
    });

    let mut notices = Vec::new();
    let mut stream_state = ParallelDispatchWorkerStreamState::default();

    // TurnCompleted 또는 Failed 이후의 이벤트는 official completion 판단에 쓰지 않는다.
    // 워커 스레드 join은 별도로 수행해 스트림 포트 자체의 오류까지 notice로 남긴다.
    while let Ok(event) = event_rx.recv() {
        emit_parallel_worker_stream_event(&request, &event);
        sync_parallel_dispatch_worker_event(&turn_service, &request, &event, &mut stream_state)
            .into_iter()
            .for_each(|notice| notices.push(notice));
        if matches!(
            event,
            ConversationStreamEvent::TurnCompleted { .. } | ConversationStreamEvent::Failed { .. }
        ) {
            break;
        }
    }

    match service_thread.join() {
        Ok(Ok(())) => {
            event_log::emit_lazy("parallel_worker_stream_joined", || {
                serde_json::json!({
                    "worktree": &request.worktree_directory,
                    "task_id": &request.handoff_task.task_id,
                    "result": "ok",
                    "saw_turn_started": stream_state.saw_turn_started,
                    "saw_failed_event": stream_state.saw_failed_event,
                    "turn_completed": stream_state.turn_completed.is_some(),
                })
            });
        }
        Ok(Err(error)) => {
            /*
             * A port error may happen after the event stream already emitted TurnCompleted
             * or Failed. Only synthesize a failure flag when the stream itself did not
             * provide a terminal event, otherwise finalize_stream_completion would double
             * count the failure class.
             */
            if stream_state.turn_completed.is_none() && !stream_state.saw_failed_event {
                stream_state.saw_failed_event = true;
                if !stream_state.saw_turn_started {
                    stream_state.saw_failed_before_turn_started = true;
                }
            }
            notices.push(format!(
                "parallel worker stream returned an error / task: {} / {error}",
                request.handoff_task.task_title
            ));
            event_log::emit_lazy("parallel_worker_stream_joined", || {
                serde_json::json!({
                    "worktree": &request.worktree_directory,
                    "task_id": &request.handoff_task.task_id,
                    "result": "error",
                    "error": error.to_string(),
                    "saw_turn_started": stream_state.saw_turn_started,
                    "saw_failed_event": stream_state.saw_failed_event,
                    "turn_completed": stream_state.turn_completed.is_some(),
                })
            });
        }
        Err(_) => {
            /*
             * Panic is treated like a terminal stream failure, but we still preserve
             * saw_turn_started so the turn service can distinguish a dirty running
             * slot from a launch failure that can be released.
             */
            if stream_state.turn_completed.is_none() && !stream_state.saw_failed_event {
                stream_state.saw_failed_event = true;
                if !stream_state.saw_turn_started {
                    stream_state.saw_failed_before_turn_started = true;
                }
            }
            notices.push(format!(
                "parallel worker stream panicked / task: {}",
                request.handoff_task.task_title
            ));
            event_log::emit_lazy("parallel_worker_stream_joined", || {
                serde_json::json!({
                    "worktree": &request.worktree_directory,
                    "task_id": &request.handoff_task.task_id,
                    "result": "panic",
                    "saw_turn_started": stream_state.saw_turn_started,
                    "saw_failed_event": stream_state.saw_failed_event,
                    "turn_completed": stream_state.turn_completed.is_some(),
                })
            });
        }
    }

    // 채널이 정상 종료돼도 완료 이벤트가 없으면 슬롯은 실패로 닫아야 한다. 그래야
    // 병렬 supervisor가 같은 worktree를 성공 슬롯으로 오인하지 않는다.
    if !stream_state.saw_failed_event && stream_state.turn_completed.is_none() {
        stream_state.saw_failed_event = true;
        if !stream_state.saw_turn_started {
            stream_state.saw_failed_before_turn_started = true;
        }
        notices.push(format!(
            "parallel worker stream ended without a completed turn / task: {}",
            request.handoff_task.task_title
        ));
    }

    let completion = turn_service.finalize_stream_completion(
        &request.worktree_directory,
        stream_state.saw_turn_started,
        stream_state.saw_failed_before_turn_started,
        stream_state.saw_failed_event,
        stream_state.saw_failed_event && stream_state.turn_completed.is_none(),
    );
    if let Some(notice) = completion.runtime_notice {
        notices.push(notice);
    }
    event_log::emit_lazy("parallel_worker_stream_finalized", || {
        serde_json::json!({
            "worktree": &request.worktree_directory,
            "task_id": &request.handoff_task.task_id,
            "saw_turn_started": stream_state.saw_turn_started,
            "saw_failed_before_turn_started": stream_state.saw_failed_before_turn_started,
            "saw_failed_event": stream_state.saw_failed_event,
            "turn_completed": stream_state.turn_completed.is_some(),
            "invalidate_supervisor_snapshot": completion.invalidate_supervisor_snapshot,
        })
    });

    if stream_state.saw_failed_event {
        /*
         * Once any stream failure is observed, do not attempt official completion refresh.
         * The planning ledger must not record an authoritative completion for a slot whose
         * app-server turn did not reach a clean terminal success.
         */
        turn_service.mark_official_completion_failed(
            &request.worktree_directory,
            "parallel worker stream failed before official completion refresh",
        );
        return if stream_state.saw_failed_before_turn_started {
            ParallelDispatchWorkerRunResult::launch_failed(notices)
        } else {
            ParallelDispatchWorkerRunResult::stream_failed(notices)
        };
    }

    let Some(turn_completed) = stream_state.turn_completed else {
        /*
         * This branch is defensive after the generic missing-completion failure above.
         * Keeping it explicit protects future changes that might add non-failed terminal
         * events without an official completion contract.
         */
        turn_service.mark_official_completion_failed(
            &request.worktree_directory,
            "parallel worker stream ended without a completed turn",
        );
        return ParallelDispatchWorkerRunResult::stream_failed(notices);
    };

    let official_completion = run_parallel_dispatch_official_completion(
        &request,
        &turn_service,
        &planning,
        &turn_completed,
        stream_state.latest_main_reply.as_deref(),
    );
    notices.extend(official_completion.notices);
    if official_completion.official_completion_refresh_succeeded {
        ParallelDispatchWorkerRunResult::completed(notices)
    } else {
        ParallelDispatchWorkerRunResult::stream_failed(notices)
    }
}

fn emit_parallel_worker_stream_event(
    request: &ParallelDispatchWorkerRequest,
    event: &ConversationStreamEvent,
) {
    match event {
        ConversationStreamEvent::ThreadPrepared {
            thread_id,
            title,
            cwd,
        } => event_log::emit_lazy("parallel_worker_stream_event", || {
            serde_json::json!({
                "event": "thread_prepared",
                "worktree": &request.worktree_directory,
                "task_id": &request.handoff_task.task_id,
                "thread_id": thread_id,
                "title": title,
                "cwd": cwd,
            })
        }),
        ConversationStreamEvent::TurnStarted { turn_id } => {
            event_log::emit_lazy("parallel_worker_stream_event", || {
                serde_json::json!({
                    "event": "turn_started",
                    "worktree": &request.worktree_directory,
                    "task_id": &request.handoff_task.task_id,
                    "turn_id": turn_id,
                })
            })
        }
        ConversationStreamEvent::AgentMessageCompleted {
            item_id,
            phase,
            text,
        } => event_log::emit_lazy("parallel_worker_stream_event", || {
            parallel_worker_agent_message_completed_trace_payload(request, item_id, phase, text)
        }),
        ConversationStreamEvent::TurnCompleted {
            turn_id,
            changed_planning_file_paths,
        } => event_log::emit_lazy("parallel_worker_stream_event", || {
            serde_json::json!({
                "event": "turn_completed",
                "worktree": &request.worktree_directory,
                "task_id": &request.handoff_task.task_id,
                "turn_id": turn_id,
                "changed_planning_file_paths": changed_planning_file_paths,
            })
        }),
        ConversationStreamEvent::Failed { message } => {
            event_log::emit_lazy("parallel_worker_stream_event", || {
                serde_json::json!({
                    "event": "failed",
                    "worktree": &request.worktree_directory,
                    "task_id": &request.handoff_task.task_id,
                    "message": message,
                })
            })
        }
        _ => {}
    }
}

fn sync_parallel_dispatch_worker_event(
    turn_service: &ParallelModeTurnService,
    request: &ParallelDispatchWorkerRequest,
    event: &ConversationStreamEvent,
    stream_state: &mut ParallelDispatchWorkerStreamState,
) -> Vec<String> {
    let mut notices = Vec::new();
    let outcome = turn_service.sync_stream_event(&request.worktree_directory, event);
    stream_state.saw_turn_started |= outcome.turn_started_observed;
    if let Some(notice) = outcome.runtime_notice {
        notices.push(notice);
    }

    match event {
        ConversationStreamEvent::AgentMessageCompleted { text, .. } => {
            let text = text.trim();
            if !text.is_empty() {
                /*
                 * Keep the latest non-empty completed assistant message. Hidden parallel
                 * workers may emit intermediate assistant messages, but the completion
                 * refresh prompt should use the final answer as the operator-facing proof.
                 */
                stream_state.latest_main_reply = Some(text.to_string());
            }
        }
        ConversationStreamEvent::TurnCompleted {
            turn_id,
            changed_planning_file_paths,
        } => {
            /*
             * changed_planning_file_paths is copied out before the loop stops because the
             * receiver exits on TurnCompleted. Later stream noise should not alter the
             * official completion validation summary for this slot.
             */
            stream_state.turn_completed = Some(ParallelDispatchTurnCompleted {
                turn_id: turn_id.clone(),
                changed_planning_file_paths: changed_planning_file_paths.clone(),
            });
        }
        ConversationStreamEvent::Failed { .. } => {
            stream_state.saw_failed_event = true;
            if !stream_state.saw_turn_started {
                stream_state.saw_failed_before_turn_started = true;
            }
        }
        _ => {}
    }

    notices
}

fn run_parallel_dispatch_official_completion(
    request: &ParallelDispatchWorkerRequest,
    turn_service: &ParallelModeTurnService,
    planning: &PlanningServices,
    turn_completed: &ParallelDispatchTurnCompleted,
    latest_main_reply: Option<&str>,
) -> ParallelDispatchOfficialCompletionOutcome {
    let mut notices = Vec::new();
    event_log::emit_lazy("parallel_official_completion_started", || {
        parallel_official_completion_started_trace_payload(
            request,
            turn_completed,
            latest_main_reply,
        )
    });

    // Official completion refreshes are serialized by slot lease order, not by thread wake-up
    // timing. That preserves planning authority when multiple parallel workers finish together.
    let refresh_order = match turn_service
        .reserve_official_completion_refresh_order(&request.worktree_directory)
    {
        Ok(Some(order)) => order,
        Ok(None) => {
            event_log::emit_lazy("parallel_official_completion_blocked", || {
                serde_json::json!({
                    "worktree": &request.worktree_directory,
                    "task_id": &request.handoff_task.task_id,
                    "blocked_reason": "no running slot lease was found",
                })
            });
            return ParallelDispatchOfficialCompletionOutcome::failed(vec![format!(
                "parallel worker completion skipped official refresh because no running slot lease was found / task: {}",
                request.handoff_task.task_title
            )]);
        }
        Err(error) => {
            turn_service.mark_official_completion_failed(&request.worktree_directory, &error);
            event_log::emit_lazy("parallel_official_completion_blocked", || {
                serde_json::json!({
                    "worktree": &request.worktree_directory,
                    "task_id": &request.handoff_task.task_id,
                    "blocked_reason": "refresh order reservation failed",
                    "error": &error,
                })
            });
            return ParallelDispatchOfficialCompletionOutcome::failed(vec![format!(
                "parallel worker completion could not reserve official refresh order / task: {} / {error}",
                request.handoff_task.task_title
            )]);
        }
    };

    let latest_main_reply = latest_main_reply
        .filter(|reply| !reply.trim().is_empty())
        .unwrap_or(
            "parallel worker TurnCompleted was captured, but no final text response was recorded",
        );
    let validation_summary =
        parallel_dispatch_validation_summary(&turn_completed.changed_planning_file_paths);

    let completion_report = match turn_service.begin_official_completion(
        &request.worktree_directory,
        &turn_completed.turn_id,
        Some(refresh_order),
        Some(latest_main_reply),
        Some(&validation_summary),
    ) {
        Ok(Some(report)) => report,
        Ok(None) => {
            event_log::emit_lazy("parallel_official_completion_blocked", || {
                serde_json::json!({
                    "worktree": &request.worktree_directory,
                    "task_id": &request.handoff_task.task_id,
                    "blocked_reason": "no running slot to report",
                    "refresh_order": refresh_order,
                })
            });
            return ParallelDispatchOfficialCompletionOutcome::failed(vec![format!(
                "parallel worker completion had no running slot to report / task: {}",
                request.handoff_task.task_title
            )]);
        }
        Err(error) => {
            turn_service.mark_official_completion_failed(&request.worktree_directory, &error);
            event_log::emit_lazy("parallel_official_completion_blocked", || {
                serde_json::json!({
                    "worktree": &request.worktree_directory,
                    "task_id": &request.handoff_task.task_id,
                    "blocked_reason": "completion capture failed",
                    "refresh_order": refresh_order,
                    "error": &error,
                })
            });
            return ParallelDispatchOfficialCompletionOutcome::failed(vec![format!(
                "parallel worker completion capture failed / task: {} / {error}",
                request.handoff_task.task_title
            )]);
        }
    };

    if let Some(notice) =
        turn_service.mark_official_completion_refreshing(&request.worktree_directory)
    {
        notices.push(notice);
    }

    let worker_request = PlanningOfficialCompletionRefreshRequest {
        /*
         * The refresh worker runs against the planning authority root, not the slot worktree.
         * Slot output is already captured in the completion contract; authority mutation must
         * happen in the canonical workspace so parallel workers converge on one ledger.
         */
        workspace_directory: &request.planning_workspace_directory,
        parent_thread_id: None,
        latest_user_message: None,
        latest_main_reply,
        previous_handoff_task: Some(&request.handoff_task),
        contract: &completion_report,
    };

    let worker_outcome = planning
        .worker
        .refresh_queue_from_official_completion(worker_request);

    let outcome = match worker_outcome {
        Ok(outcome) => outcome,
        Err(error) => {
            let detail = format!("parallel official completion refresh failed: {error}");
            turn_service.mark_official_completion_failed(&request.worktree_directory, &detail);
            event_log::emit_lazy("parallel_official_completion_failed", || {
                serde_json::json!({
                    "planning_workspace": &request.planning_workspace_directory,
                    "worktree": &request.worktree_directory,
                    "task_id": &request.handoff_task.task_id,
                    "refresh_order": refresh_order,
                    "detail": &detail,
                })
            });
            return ParallelDispatchOfficialCompletionOutcome::failed(vec![detail]);
        }
    };

    // A repair request or blocked runtime projection means the authority file is not safe for
    // auto-follow even if the worker itself produced a valid TurnCompleted event.
    if outcome.repair_request.is_some() || outcome.runtime_projection.blocks_auto_follow() {
        let detail = outcome
            .runtime_projection
            .preview_detail()
            .unwrap_or("parallel official completion refresh requires planning repair")
            .to_string();
        turn_service.mark_official_completion_failed(&request.worktree_directory, &detail);
        event_log::emit_lazy("parallel_official_completion_blocked", || {
            serde_json::json!({
                "planning_workspace": &request.planning_workspace_directory,
                "worktree": &request.worktree_directory,
                "task_id": &request.handoff_task.task_id,
                "refresh_order": refresh_order,
                "blocked_reason": detail,
                "worker_summary": outcome.worker_summary,
            })
        });
        notices.push(format!(
            "parallel official completion refresh blocked / task: {} / {detail}",
            request.handoff_task.task_title
        ));
        return ParallelDispatchOfficialCompletionOutcome::failed(notices);
    }

    if !matches!(
        outcome.runtime_projection.workspace_status(),
        PlanningRuntimeWorkspaceStatus::ReadyNoTask | PlanningRuntimeWorkspaceStatus::ReadyWithTask
    ) {
        /*
         * A non-ready projection after refresh means the worker may have changed files but
         * the runtime cannot safely choose a next queue head. Marking official completion
         * failed keeps auto-follow from chaining on top of unavailable planning state.
         */
        let detail = "parallel official completion refresh left planning unavailable";
        turn_service.mark_official_completion_failed(&request.worktree_directory, detail);
        event_log::emit_lazy("parallel_official_completion_blocked", || {
            serde_json::json!({
                "planning_workspace": &request.planning_workspace_directory,
                "worktree": &request.worktree_directory,
                "task_id": &request.handoff_task.task_id,
                "refresh_order": refresh_order,
                "blocked_reason": detail,
                "worker_summary": outcome.worker_summary,
            })
        });
        notices.push(format!(
            "parallel official completion refresh blocked / task: {} / {detail}",
            request.handoff_task.task_title
        ));
        return ParallelDispatchOfficialCompletionOutcome::failed(notices);
    }

    let authority_refresh_outcome = outcome
        .worker_summary
        .as_deref()
        .map(|summary| format!("official ledger refresh succeeded: {summary}"))
        .unwrap_or_else(|| "official ledger refresh succeeded".to_string());
    notices.extend(turn_service.finalize_official_completion_success(
        &request.worktree_directory,
        &authority_refresh_outcome,
    ));
    event_log::emit_lazy("parallel_official_completion_succeeded", || {
        serde_json::json!({
            "planning_workspace": &request.planning_workspace_directory,
            "worktree": &request.worktree_directory,
            "task_id": &request.handoff_task.task_id,
            "refresh_order": refresh_order,
            "authority_refresh_outcome": authority_refresh_outcome,
            "notice_count": notices.len(),
        })
    });
    ParallelDispatchOfficialCompletionOutcome::succeeded(notices)
}

fn parallel_official_completion_started_trace_payload(
    request: &ParallelDispatchWorkerRequest,
    turn_completed: &ParallelDispatchTurnCompleted,
    latest_main_reply: Option<&str>,
) -> serde_json::Value {
    serde_json::json!({
        "planning_workspace": &request.planning_workspace_directory,
        "worktree": &request.worktree_directory,
        "task_id": &request.handoff_task.task_id,
        "task_title": &request.handoff_task.task_title,
        "turn_id": &turn_completed.turn_id,
        "changed_planning_file_paths": &turn_completed.changed_planning_file_paths,
        "latest_main_reply_chars": latest_main_reply.map(|reply| reply.chars().count()),
    })
}

fn parallel_dispatch_validation_summary(changed_planning_file_paths: &[String]) -> String {
    if changed_planning_file_paths.is_empty() {
        /*
         * Empty change sets are still valid completion evidence. The summary must say that
         * explicitly so downstream official-completion prompts do not infer a missing
         * validation step from the absence of file paths.
         */
        return "parallel worker completed without planning file changes".to_string();
    }

    format!(
        "parallel worker completed with planning file changes: {}",
        changed_planning_file_paths.join(", ")
    )
}

fn parallel_worker_stream_starting_trace_payload(
    request: &ParallelDispatchWorkerRequest,
) -> serde_json::Value {
    serde_json::json!({
        "worktree": &request.worktree_directory,
        "task_id": &request.handoff_task.task_id,
        "task_title": &request.handoff_task.task_title,
        "service_name": &request.service_name,
        "prompt_chars": request.prompt.chars().count(),
        "developer_instructions_chars": request.developer_instructions.chars().count(),
    })
}

fn parallel_worker_thread_started_trace_payload(
    request: &ParallelDispatchWorkerRequest,
) -> serde_json::Value {
    serde_json::json!({
        "planning_workspace": &request.planning_workspace_directory,
        "worktree": &request.worktree_directory,
        "epoch_id": request.automation_epoch_id,
        "task_id": &request.handoff_task.task_id,
        "task_title": &request.handoff_task.task_title,
        "service_name": &request.service_name,
        "prompt_chars": request.prompt.chars().count(),
        "developer_instructions_chars": request.developer_instructions.chars().count(),
    })
}

fn parallel_worker_agent_message_completed_trace_payload(
    request: &ParallelDispatchWorkerRequest,
    item_id: &str,
    phase: &Option<String>,
    text: &str,
) -> serde_json::Value {
    serde_json::json!({
        "event": "agent_message_completed",
        "worktree": &request.worktree_directory,
        "task_id": &request.handoff_task.task_id,
        "item_id": item_id,
        "phase": phase,
        "text_chars": text.chars().count(),
    })
}

#[cfg(test)]
mod tests {
    use super::{
        ParallelDispatchOfficialCompletionOutcome, ParallelDispatchTurnCompleted,
        ParallelDispatchWorkerRequest, ParallelDispatchWorkerRunResult,
        ParallelDispatchWorkerStreamState, ParallelModeDispatchExecutionContext,
        dispatch_parallel_queue_pool, emit_parallel_worker_stream_event,
        parallel_dispatch_validation_summary, parallel_official_completion_started_trace_payload,
        parallel_runtime_event_for_dispatch_trigger,
        parallel_worker_agent_message_completed_trace_payload,
        parallel_worker_stream_starting_trace_payload,
        parallel_worker_thread_started_trace_payload, run_parallel_dispatch_worker,
        sync_parallel_dispatch_worker_event,
    };
    use crate::adapter::outbound::db::SqlitePlanningAuthorityAdapter;
    use crate::adapter::outbound::filesystem::FilesystemPlanningWorkspaceAdapter;
    use crate::adapter::outbound::git::parallel_mode_runtime::GitParallelModeRuntimeAdapter;
    use crate::application::port::outbound::github_automation_port::{
        GithubAutomationCapabilities, GithubAutomationPort, GithubAutomationPullRequest,
    };
    use crate::application::port::outbound::parallel_agent_worker_port::{
        ParallelAgentWorkerPort, ParallelAgentWorkerStreamRequest,
    };
    use crate::application::port::outbound::planning_worker_port::NoopPlanningWorkerPort;
    use crate::application::service::conversation_runtime_event::ConversationStreamEvent;
    use crate::application::service::parallel_mode::ParallelModeService;
    use crate::application::service::parallel_mode::turn::ParallelModeTurnService;
    use crate::application::service::planning::{PlanningServices, PlanningTaskHandoff};
    use crate::domain::parallel_mode::{
        ParallelModeAutomationTrigger, ParallelModeCapabilityKey, ParallelModeCapabilitySnapshot,
        ParallelModeCapabilityState, ParallelModeControlPlaneWorkerEventKind,
        ParallelModeRuntimeEvent,
    };
    use crate::test_utils::json_payload_contains;
    use anyhow::{Result, anyhow};
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::process::Command;
    use std::sync::{Arc, Mutex, mpsc};
    use std::time::{SystemTime, UNIX_EPOCH};

    struct NoopGithubAutomationPort;

    impl GithubAutomationPort for NoopGithubAutomationPort {
        fn inspect_capabilities(&self, _repo_root: &str) -> GithubAutomationCapabilities {
            GithubAutomationCapabilities::new(
                ready_capability(ParallelModeCapabilityKey::PushRemote),
                ready_capability(ParallelModeCapabilityKey::GhBinary),
                ready_capability(ParallelModeCapabilityKey::GhAuth),
            )
        }

        fn push_branch(
            &self,
            _repo_root: &str,
            _branch_name: &str,
            _force_with_lease: bool,
        ) -> Result<()> {
            Ok(())
        }

        fn ensure_pull_request(
            &self,
            _repo_root: &str,
            base_branch: &str,
            head_branch: &str,
            _title: &str,
            _body: &str,
        ) -> Result<GithubAutomationPullRequest> {
            Ok(GithubAutomationPullRequest::new(
                1,
                "https://github.example/pr/1",
                "open",
                base_branch,
                head_branch,
                false,
            ))
        }

        fn inspect_pull_request(
            &self,
            _repo_root: &str,
            pr_number: u64,
        ) -> Result<GithubAutomationPullRequest> {
            Ok(GithubAutomationPullRequest::new(
                pr_number,
                "https://github.example/pr/1",
                "open",
                "prerelease",
                "akra-agent/slot-1/task",
                false,
            ))
        }

        fn push_integration_branch(&self, _repo_root: &str, _branch_name: &str) -> Result<()> {
            Ok(())
        }

        fn close_pull_request(&self, _repo_root: &str, _pr_number: u64) -> Result<()> {
            Ok(())
        }
    }

    fn ready_capability(key: ParallelModeCapabilityKey) -> ParallelModeCapabilitySnapshot {
        ParallelModeCapabilitySnapshot::new(key, ParallelModeCapabilityState::Ready, "ready", None)
    }

    fn test_turn_service() -> ParallelModeTurnService {
        ParallelModeTurnService::new(test_parallel_service(Arc::new(
            SqlitePlanningAuthorityAdapter::new(),
        )))
    }

    fn test_parallel_service(
        authority: Arc<SqlitePlanningAuthorityAdapter>,
    ) -> ParallelModeService {
        ParallelModeService::new(
            authority,
            Arc::new(NoopGithubAutomationPort),
            Arc::new(GitParallelModeRuntimeAdapter::new()),
        )
    }

    fn test_planning_services(authority: Arc<SqlitePlanningAuthorityAdapter>) -> PlanningServices {
        PlanningServices::from_ports(
            Arc::new(FilesystemPlanningWorkspaceAdapter::new()),
            authority.clone(),
            authority,
            Arc::new(NoopPlanningWorkerPort),
        )
    }

    struct TempGitWorkspace {
        root: PathBuf,
        workspace: String,
    }

    impl TempGitWorkspace {
        fn new(prefix: &str) -> Self {
            let unique = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("system clock should be valid")
                .as_nanos();
            let root = std::env::temp_dir().join(format!("{prefix}-{unique}"));
            let repo_root = root.join("repo");
            fs::create_dir_all(&repo_root).expect("temp git repo should be created");
            run_git(&repo_root, &["init", "-q"]);
            run_git(&repo_root, &["config", "user.name", "RefinedStone"]);
            run_git(
                &repo_root,
                &["config", "user.email", "chem.en.9273@gmail.com"],
            );
            fs::write(repo_root.join("README.md"), "seed\n").expect("seed file should write");
            run_git(&repo_root, &["add", "README.md"]);
            run_git(&repo_root, &["commit", "-qm", "init"]);
            Self {
                root,
                workspace: repo_root.display().to_string(),
            }
        }

        fn path(&self) -> &str {
            self.workspace.as_str()
        }
    }

    impl Drop for TempGitWorkspace {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    fn run_git(repo_root: &Path, args: &[&str]) {
        let output = Command::new("git")
            .current_dir(repo_root)
            .args(args)
            .env("GIT_TERMINAL_PROMPT", "0")
            .output()
            .expect("git command should spawn");
        assert!(
            output.status.success(),
            "git command should succeed: git {:?}\nstdout: {}\nstderr: {}",
            args,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr),
        );
    }

    #[derive(Debug, Clone, Copy)]
    enum WorkerExit {
        Ok,
        Err,
        Panic,
    }

    #[derive(Debug)]
    struct ScriptedParallelAgentWorkerPort {
        events: Mutex<Vec<ConversationStreamEvent>>,
        exit: WorkerExit,
    }

    impl ScriptedParallelAgentWorkerPort {
        fn new(events: Vec<ConversationStreamEvent>, exit: WorkerExit) -> Self {
            Self {
                events: Mutex::new(events),
                exit,
            }
        }
    }

    impl ParallelAgentWorkerPort for ScriptedParallelAgentWorkerPort {
        fn run_isolated_new_thread_stream(
            &self,
            _request: ParallelAgentWorkerStreamRequest<'_>,
            event_sender: std::sync::mpsc::Sender<ConversationStreamEvent>,
        ) -> Result<()> {
            let events = self
                .events
                .lock()
                .expect("scripted worker events mutex should not be poisoned")
                .clone();
            for event in events {
                event_sender
                    .send(event)
                    .expect("worker reducer should still receive scripted events");
            }
            match self.exit {
                WorkerExit::Ok => Ok(()),
                WorkerExit::Err => Err(anyhow!("scripted worker port failed")),
                WorkerExit::Panic => panic!("scripted worker port panicked"),
            }
        }
    }

    fn run_scripted_worker(
        events: Vec<ConversationStreamEvent>,
        exit: WorkerExit,
    ) -> ParallelDispatchWorkerRunResult {
        run_scripted_worker_with_request(worker_request_with_secret_bodies(), events, exit)
    }

    fn run_scripted_worker_with_request(
        request: ParallelDispatchWorkerRequest,
        events: Vec<ConversationStreamEvent>,
        exit: WorkerExit,
    ) -> ParallelDispatchWorkerRunResult {
        let authority = Arc::new(SqlitePlanningAuthorityAdapter::new());
        let turn_service = ParallelModeTurnService::new(test_parallel_service(authority.clone()));
        let planning = test_planning_services(authority);
        run_parallel_dispatch_worker(
            request,
            Arc::new(ScriptedParallelAgentWorkerPort::new(events, exit)),
            turn_service,
            planning,
        )
    }

    fn with_test_event_logging<T>(action: impl FnOnce() -> T) -> T {
        use tracing_subscriber::prelude::*;

        let subscriber = tracing_subscriber::registry()
            .with(tracing_subscriber::EnvFilter::new(format!(
                "{}=debug",
                crate::diagnostics::trace_event_log::AKRA_EVENT_TARGET
            )))
            .with(tracing_subscriber::fmt::layer().with_writer(std::io::sink));
        tracing::subscriber::with_default(subscriber, action)
    }

    #[test]
    fn trigger_mapping_covers_public_automation_triggers() {
        assert_eq!(
            parallel_runtime_event_for_dispatch_trigger(
                ParallelModeAutomationTrigger::MainTurnPostEvaluation,
            ),
            ParallelModeRuntimeEvent::AutoFollowQueued
        );
        assert_eq!(
            parallel_runtime_event_for_dispatch_trigger(
                ParallelModeAutomationTrigger::ParallelOfficialCompletion,
            ),
            ParallelModeRuntimeEvent::ParallelCompletionFinalized
        );
        assert_eq!(
            parallel_runtime_event_for_dispatch_trigger(
                ParallelModeAutomationTrigger::TaskIntakeAfterEpoch,
            ),
            ParallelModeRuntimeEvent::TaskIntakeCommitted
        );
    }

    #[test]
    fn worker_result_constructors_preserve_control_plane_event_kinds() {
        let launch_failed = ParallelDispatchWorkerRunResult::launch_failed(vec!["launch".into()]);
        assert_eq!(
            launch_failed.worker_event_kind,
            ParallelModeControlPlaneWorkerEventKind::LaunchFailed
        );
        assert_eq!(launch_failed.notices, vec!["launch".to_string()]);

        let stream_failed = ParallelDispatchWorkerRunResult::stream_failed(vec!["stream".into()]);
        assert_eq!(
            stream_failed.worker_event_kind,
            ParallelModeControlPlaneWorkerEventKind::StreamFailed
        );

        let completed = ParallelDispatchWorkerRunResult::completed(vec!["done".into()]);
        assert_eq!(
            completed.worker_event_kind,
            ParallelModeControlPlaneWorkerEventKind::Completed
        );
        assert_eq!(completed.notices, vec!["done".to_string()]);
    }

    #[test]
    fn official_completion_outcome_constructors_mark_refresh_success_flag() {
        let failed = ParallelDispatchOfficialCompletionOutcome::failed(vec!["blocked".into()]);
        assert!(!failed.official_completion_refresh_succeeded);
        assert_eq!(failed.notices, vec!["blocked".to_string()]);

        let succeeded = ParallelDispatchOfficialCompletionOutcome::succeeded(vec!["ok".into()]);
        assert!(succeeded.official_completion_refresh_succeeded);
        assert_eq!(succeeded.notices, vec!["ok".to_string()]);
    }

    #[test]
    fn scripted_worker_run_classifies_missing_completion_error_and_panic_paths() {
        let missing_completion = run_scripted_worker(Vec::new(), WorkerExit::Ok);
        assert_eq!(
            missing_completion.worker_event_kind,
            ParallelModeControlPlaneWorkerEventKind::LaunchFailed
        );
        assert!(
            missing_completion
                .notices
                .iter()
                .any(|notice| notice.contains("ended without a completed turn"))
        );

        let port_error = run_scripted_worker(Vec::new(), WorkerExit::Err);
        assert_eq!(
            port_error.worker_event_kind,
            ParallelModeControlPlaneWorkerEventKind::LaunchFailed
        );
        assert!(
            port_error
                .notices
                .iter()
                .any(|notice| notice.contains("returned an error"))
        );

        let panic_result = run_scripted_worker(Vec::new(), WorkerExit::Panic);
        assert_eq!(
            panic_result.worker_event_kind,
            ParallelModeControlPlaneWorkerEventKind::LaunchFailed
        );
        assert!(
            panic_result
                .notices
                .iter()
                .any(|notice| notice.contains("panicked"))
        );
    }

    #[test]
    fn scripted_worker_run_keeps_started_failures_as_stream_failures() {
        let result = run_scripted_worker(
            vec![ConversationStreamEvent::TurnStarted {
                turn_id: "turn-started".to_string(),
            }],
            WorkerExit::Err,
        );

        assert_eq!(
            result.worker_event_kind,
            ParallelModeControlPlaneWorkerEventKind::StreamFailed
        );
        assert!(
            result
                .notices
                .iter()
                .any(|notice| notice.contains("returned an error"))
        );
    }

    #[test]
    fn scripted_worker_run_skips_official_completion_without_running_slot_lease() {
        let result = run_scripted_worker(
            vec![
                ConversationStreamEvent::TurnStarted {
                    turn_id: "turn-lease-missing".to_string(),
                },
                ConversationStreamEvent::AgentMessageCompleted {
                    item_id: "item-final".to_string(),
                    phase: Some("final_answer".to_string()),
                    text: "done".to_string(),
                },
                ConversationStreamEvent::TurnCompleted {
                    turn_id: "turn-lease-missing".to_string(),
                    changed_planning_file_paths: Vec::new(),
                },
            ],
            WorkerExit::Ok,
        );

        assert_eq!(
            result.worker_event_kind,
            ParallelModeControlPlaneWorkerEventKind::StreamFailed
        );
        assert!(
            result
                .notices
                .iter()
                .any(|notice| notice.contains("could not reserve official refresh order")),
            "notices: {:?}",
            result.notices
        );
    }

    #[test]
    fn scripted_worker_run_reports_no_running_slot_for_git_workspace_without_lease() {
        let workspace = TempGitWorkspace::new("parallel-no-running-slot");
        let mut request = worker_request_with_secret_bodies();
        request.planning_workspace_directory = workspace.path().to_string();
        request.worktree_directory = workspace.path().to_string();

        let result = run_scripted_worker_with_request(
            request,
            vec![
                ConversationStreamEvent::TurnStarted {
                    turn_id: "turn-no-lease".to_string(),
                },
                ConversationStreamEvent::TurnCompleted {
                    turn_id: "turn-no-lease".to_string(),
                    changed_planning_file_paths: Vec::new(),
                },
            ],
            WorkerExit::Ok,
        );

        assert_eq!(
            result.worker_event_kind,
            ParallelModeControlPlaneWorkerEventKind::StreamFailed
        );
        assert!(
            result
                .notices
                .iter()
                .any(|notice| notice.contains("no running slot lease was found")),
            "notices: {:?}",
            result.notices
        );
    }

    #[test]
    fn scripted_worker_run_traces_stream_event_shapes_when_logging_is_enabled() {
        let result = with_test_event_logging(|| {
            run_scripted_worker(
                vec![
                    ConversationStreamEvent::ThreadPrepared {
                        thread_id: "thread-prepared".to_string(),
                        title: "Prepared slot".to_string(),
                        cwd: "/tmp/workspace/.akra-pool/slot-1".to_string(),
                    },
                    ConversationStreamEvent::TurnStarted {
                        turn_id: "turn-started".to_string(),
                    },
                    ConversationStreamEvent::AgentMessageCompleted {
                        item_id: "item-final".to_string(),
                        phase: Some("final_answer".to_string()),
                        text: "final stream reply".to_string(),
                    },
                    ConversationStreamEvent::Failed {
                        message: "stream terminal failure".to_string(),
                    },
                ],
                WorkerExit::Ok,
            )
        });

        assert_eq!(
            result.worker_event_kind,
            ParallelModeControlPlaneWorkerEventKind::StreamFailed
        );

        emit_parallel_worker_stream_event(
            &worker_request_with_secret_bodies(),
            &ConversationStreamEvent::StatusUpdated {
                text: "status events are intentionally not traced here".to_string(),
            },
        );
    }

    #[test]
    fn dispatch_pool_reports_plan_build_error_before_worker_launch() {
        let authority = Arc::new(SqlitePlanningAuthorityAdapter::new());
        let service = test_parallel_service(authority.clone());
        let planning = test_planning_services(authority);
        let projection = planning
            .runtime
            .load_runtime_projection_or_invalid("/tmp/akra-not-a-git-repository");
        let (event_sender, _event_receiver) = mpsc::channel();

        let outcome = with_test_event_logging(|| {
            dispatch_parallel_queue_pool(
                &service,
                ParallelModeDispatchExecutionContext {
                    workspace_directory: "/tmp/akra-not-a-git-repository",
                    planning_projection: &projection,
                    worker_port: Arc::new(ScriptedParallelAgentWorkerPort::new(
                        Vec::new(),
                        WorkerExit::Ok,
                    )),
                    turn_service: ParallelModeTurnService::new(service.clone()),
                    planning,
                    event_sender,
                    trigger: ParallelModeAutomationTrigger::TaskIntakeAfterEpoch,
                    epoch_id: 99,
                },
            )
        });

        assert!(
            outcome
                .blocked_reason
                .as_deref()
                .is_some_and(|reason| reason.contains("repository inspection failed"))
        );
    }

    #[test]
    fn scripted_worker_keeps_port_error_notice_after_turn_completed() {
        let result = with_test_event_logging(|| {
            run_scripted_worker(
                vec![
                    ConversationStreamEvent::TurnStarted {
                        turn_id: "turn-completed-before-port-error".to_string(),
                    },
                    ConversationStreamEvent::AgentMessageCompleted {
                        item_id: "item-final".to_string(),
                        phase: Some("final_answer".to_string()),
                        text: "done before port error".to_string(),
                    },
                    ConversationStreamEvent::TurnCompleted {
                        turn_id: "turn-completed-before-port-error".to_string(),
                        changed_planning_file_paths: vec![
                            ".codex-exec-loop/planning/result.md".to_string(),
                        ],
                    },
                ],
                WorkerExit::Err,
            )
        });

        assert_eq!(
            result.worker_event_kind,
            ParallelModeControlPlaneWorkerEventKind::StreamFailed
        );
        assert!(
            result
                .notices
                .iter()
                .any(|notice| notice.contains("returned an error")),
            "notices: {:?}",
            result.notices
        );
        assert!(
            result
                .notices
                .iter()
                .any(|notice| notice.contains("could not reserve official refresh order")),
            "notices: {:?}",
            result.notices
        );
    }

    #[test]
    fn sync_worker_event_updates_reply_completion_and_failure_state() {
        let turn_service = test_turn_service();
        let request = worker_request_with_secret_bodies();
        let mut stream_state = ParallelDispatchWorkerStreamState::default();

        assert!(
            sync_parallel_dispatch_worker_event(
                &turn_service,
                &request,
                &ConversationStreamEvent::AgentMessageCompleted {
                    item_id: "blank".to_string(),
                    phase: None,
                    text: "   ".to_string(),
                },
                &mut stream_state,
            )
            .is_empty()
        );
        assert_eq!(stream_state.latest_main_reply, None);

        sync_parallel_dispatch_worker_event(
            &turn_service,
            &request,
            &ConversationStreamEvent::AgentMessageCompleted {
                item_id: "final".to_string(),
                phase: Some("final_answer".to_string()),
                text: "  final reply  ".to_string(),
            },
            &mut stream_state,
        );
        assert_eq!(
            stream_state.latest_main_reply.as_deref(),
            Some("final reply")
        );

        sync_parallel_dispatch_worker_event(
            &turn_service,
            &request,
            &ConversationStreamEvent::TurnCompleted {
                turn_id: "turn-1".to_string(),
                changed_planning_file_paths: vec!["docs/plan/result-output.md".to_string()],
            },
            &mut stream_state,
        );
        let turn_completed = stream_state
            .turn_completed
            .as_ref()
            .expect("turn completed should be captured");
        assert_eq!(turn_completed.turn_id, "turn-1");
        assert_eq!(
            turn_completed.changed_planning_file_paths,
            vec!["docs/plan/result-output.md".to_string()]
        );

        let mut failed_before_start = ParallelDispatchWorkerStreamState::default();
        sync_parallel_dispatch_worker_event(
            &turn_service,
            &request,
            &ConversationStreamEvent::Failed {
                message: "startup failed".to_string(),
            },
            &mut failed_before_start,
        );
        assert!(failed_before_start.saw_failed_event);
        assert!(failed_before_start.saw_failed_before_turn_started);

        let mut failed_after_start = ParallelDispatchWorkerStreamState::default();
        sync_parallel_dispatch_worker_event(
            &turn_service,
            &request,
            &ConversationStreamEvent::TurnStarted {
                turn_id: "turn-2".to_string(),
            },
            &mut failed_after_start,
        );
        sync_parallel_dispatch_worker_event(
            &turn_service,
            &request,
            &ConversationStreamEvent::Failed {
                message: "runtime failed".to_string(),
            },
            &mut failed_after_start,
        );
        assert!(failed_after_start.saw_turn_started);
        assert!(failed_after_start.saw_failed_event);
        assert!(!failed_after_start.saw_failed_before_turn_started);
    }

    #[test]
    fn parallel_worker_stream_starting_trace_payload_keeps_prompt_bodies_out_of_log() {
        let request = worker_request_with_secret_bodies();

        let payload = parallel_worker_stream_starting_trace_payload(&request);
        let fields = payload.as_object().expect("payload should be an object");

        assert_eq!(fields["prompt_chars"], 18);
        assert_eq!(fields["developer_instructions_chars"], 21);
        assert!(!fields.contains_key("prompt"));
        assert!(!fields.contains_key("developer_instructions"));
        assert!(!json_payload_contains(&payload, "SECRET-"));
    }

    #[test]
    fn parallel_worker_thread_started_trace_payload_keeps_prompt_bodies_out_of_log() {
        let request = worker_request_with_secret_bodies();

        let payload = parallel_worker_thread_started_trace_payload(&request);
        let fields = payload.as_object().expect("payload should be an object");

        assert_eq!(fields["planning_workspace"], "/tmp/workspace");
        assert_eq!(fields["epoch_id"], 7);
        assert_eq!(fields["prompt_chars"], 18);
        assert_eq!(fields["developer_instructions_chars"], 21);
        assert!(!fields.contains_key("prompt"));
        assert!(!fields.contains_key("developer_instructions"));
        assert!(!json_payload_contains(&payload, "SECRET-"));
    }

    #[test]
    fn parallel_worker_completed_message_trace_payload_keeps_text_body_out_of_log() {
        let request = worker_request_with_secret_bodies();

        let payload = parallel_worker_agent_message_completed_trace_payload(
            &request,
            "item-1",
            &Some("final_answer".to_string()),
            "assistant SECRET-REPLY body",
        );
        let fields = payload.as_object().expect("payload should be an object");

        assert_eq!(fields["text_chars"], 27);
        assert!(!fields.contains_key("text"));
        assert!(!json_payload_contains(&payload, "SECRET-REPLY"));
    }

    #[test]
    fn official_completion_started_trace_payload_keeps_latest_reply_body_out_of_log() {
        let request = worker_request_with_secret_bodies();
        let turn_completed = ParallelDispatchTurnCompleted {
            turn_id: "turn-secret".to_string(),
            changed_planning_file_paths: vec!["docs/plan.md".to_string()],
        };

        let payload = parallel_official_completion_started_trace_payload(
            &request,
            &turn_completed,
            Some("assistant SECRET-REPLY body"),
        );
        let fields = payload.as_object().expect("payload should be an object");

        assert_eq!(fields["latest_main_reply_chars"], 27);
        assert!(!fields.contains_key("latest_main_reply"));
        assert!(!json_payload_contains(&payload, "SECRET-REPLY"));
    }

    #[test]
    fn validation_summary_distinguishes_empty_and_changed_planning_paths() {
        assert_eq!(
            parallel_dispatch_validation_summary(&[]),
            "parallel worker completed without planning file changes"
        );
        assert_eq!(
            parallel_dispatch_validation_summary(&[
                "docs/plan/result-output.md".to_string(),
                "schema/task-authority.json".to_string(),
            ]),
            "parallel worker completed with planning file changes: docs/plan/result-output.md, schema/task-authority.json"
        );
    }

    #[test]
    fn noop_github_helper_methods_and_failed_git_diagnostics_are_covered() {
        let github = NoopGithubAutomationPort;
        let capabilities = github.inspect_capabilities("/tmp/repo");
        assert!(capabilities.push_ready());
        assert!(capabilities.pull_request_workflow_ready());
        github
            .push_branch("/tmp/repo", "feature/test", false)
            .expect("noop push should succeed");
        let pr = github
            .ensure_pull_request("/tmp/repo", "prerelease", "feature/test", "title", "body")
            .expect("noop PR ensure should succeed");
        assert_eq!(pr.base_branch, "prerelease");
        assert_eq!(
            github
                .inspect_pull_request("/tmp/repo", 12)
                .expect("noop PR inspect should succeed")
                .number,
            12
        );
        github
            .push_integration_branch("/tmp/repo", "prerelease")
            .expect("noop integration push should succeed");
        github
            .close_pull_request("/tmp/repo", 12)
            .expect("noop PR close should succeed");

        let temp = TempGitWorkspace::new("parallel-run-git-failure");
        let panic = std::panic::catch_unwind(|| {
            run_git(
                Path::new(temp.path()),
                &["definitely-not-a-real-git-subcommand"],
            );
        });
        assert!(panic.is_err());
    }

    fn worker_request_with_secret_bodies() -> ParallelDispatchWorkerRequest {
        ParallelDispatchWorkerRequest {
            planning_workspace_directory: "/tmp/workspace".to_string(),
            worktree_directory: "/tmp/workspace/.akra-pool/slot-1".to_string(),
            automation_epoch_id: 7,
            prompt: "prompt SECRET-BODY".to_string(),
            developer_instructions: "developer SECRET-BODY".to_string(),
            service_name: "akra-parallel-worker".to_string(),
            handoff_task: PlanningTaskHandoff {
                task_id: "task-a".to_string(),
                task_title: "Check trace retention".to_string(),
                direction_id: "direction-a".to_string(),
                combined_priority: 42,
                updated_at: "2026-05-09T00:00:00Z".to_string(),
                status_label: "ready".to_string(),
            },
        }
    }
}
