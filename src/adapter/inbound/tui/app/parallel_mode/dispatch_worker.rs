use crate::application::port::outbound::parallel_agent_worker_port::{
    ParallelAgentWorkerPort, ParallelAgentWorkerStreamRequest,
};
use crate::application::service::conversation_runtime_event::ConversationStreamEvent;
use crate::application::service::parallel_mode::turn::ParallelModeTurnService;
use crate::application::service::planning::{
    PlanningOfficialCompletionRefreshRequest, PlanningServices, PlanningTaskHandoff,
};
use crate::domain::parallel_mode::ParallelModeAutomationTrigger;
use std::sync::Arc;
use std::sync::mpsc;
use std::thread;

use super::super::BackgroundMessage;

/* 병렬 슬롯 워커는 TUI 스레드 밖에서 Codex 세션 스트림을 끝까지 소비하고,
 * 그 결과를 다시 슬롯 상태와 planning 권위 파일 갱신으로 접속한다. 이 파일의
 * 경계는 UI 이벤트 처리보다 넓어서, 실패를 런타임 notice로 남기면서도 마지막에는
 * supervisor snapshot 무효화를 반드시 보내는 것이 호출 계약이다.
 */
#[derive(Debug, Clone)]
pub(super) struct ParallelDispatchWorkerRequest {
    // planning workspace는 official completion refresh가 반영될 authoritative root이다.
    pub(super) planning_workspace_directory: String,
    // worktree는 실제 isolated Codex turn이 실행되는 slot checkout이다.
    pub(super) worktree_directory: String,
    // automation epoch lets the UI drop delayed completion chaining after :parallel off.
    pub(super) automation_epoch_id: u64,
    // prompt는 queue head handoff를 worker thread에 전달하는 최종 user-facing 입력이다.
    pub(super) prompt: String,
    // developer_instructions/service_name은 application prompt assembly가 정한 app-server thread metadata다.
    pub(super) developer_instructions: String,
    pub(super) service_name: String,
    // handoff_task는 notice, completion contract, refresh prompt가 같은 task를 가리키게 하는 연결 키이다.
    pub(super) handoff_task: PlanningTaskHandoff,
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
    official_completion_refresh_succeeded: bool,
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

pub(super) fn spawn_parallel_dispatch_worker(
    request: ParallelDispatchWorkerRequest,
    worker_port: Arc<dyn ParallelAgentWorkerPort>,
    turn_service: ParallelModeTurnService,
    planning: PlanningServices,
    outer_tx: std::sync::mpsc::Sender<BackgroundMessage>,
) {
    thread::spawn(move || {
        /*
         * Background worker는 TUI event loop를 직접 만지지 않는다. 모든 결과는 notice message와
         * supervisor snapshot invalidation으로 되돌아가며, sender 실패는 이미 UI가 내려가는 중이라는
         * 의미라 worker thread 안에서 추가 복구를 시도하지 않는다.
         */
        let workspace_directory = request.planning_workspace_directory.clone();
        let automation_epoch_id = request.automation_epoch_id;
        let result =
            run_parallel_dispatch_worker(request, worker_port, turn_service, planning.clone());
        for notice in result.notices {
            let _ = outer_tx.send(BackgroundMessage::ConversationRuntimeNotice(notice));
        }
        /*
         * 성공, 실패, panic 어느 경로든 supervisor snapshot을 다시 읽게 해야 slot lease와
         * official completion marker가 화면에 남지 않는다.
         */
        let _ = outer_tx.send(BackgroundMessage::InvalidateParallelModeSupervisorSnapshot);
        let planning_snapshot = planning
            .runtime
            .load_runtime_snapshot_or_invalid(&workspace_directory);
        if result.official_completion_refresh_succeeded
            && planning_snapshot.has_actionable_queue_head()
        {
            let _ = outer_tx.send(BackgroundMessage::RequestParallelModeDispatch {
                workspace_directory,
                trigger: ParallelModeAutomationTrigger::ParallelOfficialCompletion,
                epoch_id: automation_epoch_id,
            });
        }
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
        Ok(Ok(())) => {}
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
        return ParallelDispatchWorkerRunResult {
            notices,
            official_completion_refresh_succeeded: false,
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
        return ParallelDispatchWorkerRunResult {
            notices,
            official_completion_refresh_succeeded: false,
        };
    };

    let official_completion = run_parallel_dispatch_official_completion(
        &request,
        &turn_service,
        &planning,
        &turn_completed,
        stream_state.latest_main_reply.as_deref(),
    );
    notices.extend(official_completion.notices);
    ParallelDispatchWorkerRunResult {
        notices,
        official_completion_refresh_succeeded: official_completion
            .official_completion_refresh_succeeded,
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

    // Official completion refreshes are serialized by slot lease order, not by thread wake-up
    // timing. That preserves planning authority when multiple parallel workers finish together.
    let refresh_order = match turn_service
        .reserve_official_completion_refresh_order(&request.worktree_directory)
    {
        Ok(Some(order)) => order,
        Ok(None) => {
            return ParallelDispatchOfficialCompletionOutcome::failed(vec![format!(
                "parallel worker completion skipped official refresh because no running slot lease was found / task: {}",
                request.handoff_task.task_title
            )]);
        }
        Err(error) => {
            turn_service.mark_official_completion_failed(&request.worktree_directory, &error);
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
            return ParallelDispatchOfficialCompletionOutcome::failed(vec![format!(
                "parallel worker completion had no running slot to report / task: {}",
                request.handoff_task.task_title
            )]);
        }
        Err(error) => {
            turn_service.mark_official_completion_failed(&request.worktree_directory, &error);
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
            return ParallelDispatchOfficialCompletionOutcome::failed(vec![detail]);
        }
    };

    // A repair request or blocked runtime snapshot means the authority file is not safe for
    // auto-followup even if the worker itself produced a valid TurnCompleted event.
    if outcome.repair_request.is_some() || outcome.runtime_snapshot.blocks_auto_followup() {
        let detail = outcome
            .runtime_snapshot
            .preview_detail()
            .unwrap_or("parallel official completion refresh requires planning repair")
            .to_string();
        turn_service.mark_official_completion_failed(&request.worktree_directory, &detail);
        notices.push(format!(
            "parallel official completion refresh blocked / task: {} / {detail}",
            request.handoff_task.task_title
        ));
        return ParallelDispatchOfficialCompletionOutcome::failed(notices);
    }

    if !matches!(
        outcome.runtime_snapshot.workspace_status(),
        crate::application::service::planning::PlanningRuntimeWorkspaceStatus::ReadyNoTask
            | crate::application::service::planning::PlanningRuntimeWorkspaceStatus::ReadyWithTask
    ) {
        /*
         * A non-ready snapshot after refresh means the worker may have changed files but
         * the runtime cannot safely choose a next queue head. Marking official completion
         * failed keeps auto-followup from chaining on top of unavailable planning state.
         */
        let detail = "parallel official completion refresh left planning unavailable";
        turn_service.mark_official_completion_failed(&request.worktree_directory, detail);
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
    ParallelDispatchOfficialCompletionOutcome::succeeded(notices)
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
